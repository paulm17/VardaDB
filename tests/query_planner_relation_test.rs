//! Stage 2.2a relation-pipeline tests.
//!
//! `resolve_list_internal` is a thin dispatcher over
//! [`build_relation_pipeline`]: edge scan -> optional cosine re-rank ->
//! residual filter -> sort -> cursor -> offset -> limit. These tests pin the
//! GraphQL-observable behavior against the legacy semantics, including the
//! keep-all-if-absent cursor and the embedding-drop re-rank rules.

use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata, Resolver};
use vardadb::realtime::bus::MutationSource;
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

fn ts(counter: u16) -> Timestamp {
    Timestamp::new(1_700_000_000_000, counter, 1)
}

fn str_map(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

struct Fixture {
    _dir: tempfile::TempDir,
    resolver: SqliteResolver,
    _metadata: HashMap<String, QueryTypeMetadata>,
}

#[allow(clippy::too_many_arguments)]
fn rel(
    fx: &Fixture,
    parent: u64,
    filter: HashMap<String, serde_json::Value>,
    sort: &[(&str, &str)],
    first: Option<usize>,
    after: Option<&str>,
    offset: Option<usize>,
    near_vector: Option<Vec<f64>>,
) -> Vec<u64> {
    let sort_map: HashMap<String, async_graphql::Value> = sort
        .iter()
        .map(|(f, d)| (f.to_string(), async_graphql::Value::String(d.to_string())))
        .collect();
    fx.resolver
        .resolve_list(
            parent,
            "books",
            filter.into_iter()
                .map(|(k, v)| (k, serde_json_to_gql(&v)))
                .collect(),
            sort_map,
            first,
            after.map(|s| s.to_string()),
            offset,
            near_vector,
        )
        .unwrap()
}

/// Minimal serde_json -> async_graphql conversion for filters.
fn serde_json_to_gql(v: &serde_json::Value) -> async_graphql::Value {
    match v {
        serde_json::Value::Null => async_graphql::Value::Null,
        serde_json::Value::Bool(b) => async_graphql::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                async_graphql::Value::Number(i.into())
            } else {
                async_graphql::Value::Number(async_graphql::Number::from_f64(n.as_f64().unwrap()).unwrap())
            }
        }
        serde_json::Value::String(s) => async_graphql::Value::String(s.clone()),
        serde_json::Value::Array(items) => async_graphql::Value::List(
            items.iter().map(serde_json_to_gql).collect(),
        ),
        serde_json::Value::Object(map) => {
            let mut out = async_graphql::indexmap::IndexMap::new();
            for (k, v) in map {
                out.insert(
                    async_graphql::Name::new(k),
                    serde_json_to_gql(v),
                );
            }
            async_graphql::Value::Object(out)
        }
    }
}

fn build_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    for (uid, name, age) in [(101u64, "Paul", 40i64), (102, "Ada", 36), (103, "Bob", 25)] {
        resolver
            .create_node_internal(
                "Author",
                uid,
                str_map(&[
                    ("name", serde_json::json!(name)),
                    ("age", serde_json::json!(age)),
                ]),
                &["name".to_string()],
                &[],
                &HashMap::new(),
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let book_inverses = vec![InverseInfo {
        field: "author".to_string(),
        inverse_type: "Author".to_string(),
        inverse_field: "books".to_string(),
        inverse_is_list: true,
    }];
    for (uid, title, author, embedding) in [
        (201u64, "Planner Internals", 101u64, Some(vec![1.0, 0.0])),
        (202, "Query Engines", 101, Some(vec![0.6, 0.8])),
        (203, "Cooking Basics", 102, None),
    ] {
        let mut fields = vec![
            ("title", serde_json::json!(title)),
            ("author", serde_json::json!(author.to_string())),
        ];
        if let Some(vec) = embedding {
            fields.push(("embedding", serde_json::json!(vec)));
        }
        resolver
            .create_node_internal(
                "Book",
                uid,
                str_map(&fields),
                &[],
                &book_inverses,
                &HashMap::new(),
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let mut metadata = HashMap::new();
    metadata.insert(
        "Author".to_string(),
        QueryTypeMetadata {
            uniques: vec!["name".to_string()],
            inverses: vec![InverseInfo {
                field: "books".to_string(),
                inverse_type: "Book".to_string(),
                inverse_field: "author".to_string(),
                inverse_is_list: true,
            }],
            relations: HashMap::from([("books".to_string(), "Book".to_string())]),
        },
    );
    metadata.insert(
        "Book".to_string(),
        QueryTypeMetadata {
            uniques: vec![],
            inverses: vec![],
            relations: HashMap::from([("author".to_string(), "Author".to_string())]),
        },
    );

    Fixture { _dir: dir, resolver, _metadata: metadata }
}

fn filter_map(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[test]
fn plain_list_preserves_edge_order() {
    let fx = build_fixture();
    assert_eq!(rel(&fx, 101, filter_map(&[]), &[], None, None, None, None), vec![201, 202]);
    assert_eq!(rel(&fx, 102, filter_map(&[]), &[], None, None, None, None), vec![203]);
    assert_eq!(rel(&fx, 103, filter_map(&[]), &[], None, None, None, None), Vec::<u64>::new());
}

#[test]
fn filtered_list_residual() {
    let fx = build_fixture();
    assert_eq!(
        rel(&fx, 101, filter_map(&[("title", serde_json::json!({"contains": "Query"}))]), &[], None, None, None, None),
        vec![202]
    );
}

#[test]
fn sorted_list_asc_and_desc() {
    let fx = build_fixture();
    assert_eq!(
        rel(&fx, 101, filter_map(&[]), &[("title", "ASC")], None, None, None, None),
        vec![201, 202]
    );
    assert_eq!(
        rel(&fx, 101, filter_map(&[]), &[("title", "DESC")], None, None, None, None),
        vec![202, 201]
    );
}

#[test]
fn pagination_first_cursor_offset_order() {
    let fx = build_fixture();
    assert_eq!(rel(&fx, 101, filter_map(&[]), &[], Some(1), None, None, None), vec![201]);
    // Strictly-after cursor semantics.
    assert_eq!(rel(&fx, 101, filter_map(&[]), &[], Some(1), Some("201"), None, None), vec![202]);
    // Offset applies after cursor skip.
    assert_eq!(rel(&fx, 101, filter_map(&[]), &[], Some(1), None, Some(1), None), vec![202]);
}

#[test]
fn absent_cursor_keeps_all_like_legacy() {
    let fx = build_fixture();
    assert_eq!(
        rel(&fx, 101, filter_map(&[]), &[], None, Some("999"), None, None),
        vec![201, 202]
    );
}

#[test]
fn cosine_rerank_orders_by_distance() {
    let fx = build_fixture();
    // Query closest to Book 201's [1,0].
    assert_eq!(
        rel(&fx, 101, filter_map(&[]), &[], None, None, None, Some(vec![1.0, 0.0])),
        vec![201, 202]
    );
    // Reversed query flips the order.
    assert_eq!(
        rel(&fx, 101, filter_map(&[]), &[], None, None, None, Some(vec![0.0, 1.0])),
        vec![202, 201]
    );
}

#[test]
fn rerank_drops_rows_without_usable_embedding() {
    let fx = build_fixture();
    // Book 203 has no stored embedding, so Ada's list becomes empty.
    assert_eq!(
        rel(&fx, 102, filter_map(&[]), &[], None, None, None, Some(vec![1.0, 0.0])),
        Vec::<u64>::new()
    );
}

#[test]
fn rerank_then_residual_filter_combination() {
    let fx = build_fixture();
    assert_eq!(
        rel(
            &fx,
            101,
            filter_map(&[("title", serde_json::json!({"contains": "Query"}))]),
            &[],
            None,
            None,
            None,
            Some(vec![0.0, 1.0])
        ),
        vec![202]
    );
}
