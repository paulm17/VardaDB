//! Stage 2.1 root-cutover tests.
//!
//! `scan_nodes_internal` / `count_nodes_internal` are thin dispatchers over
//! the planner operator pipeline. These tests pin the GraphQL-observable
//! behavior of the dispatcher across every access shape: full scan, unique,
//! SQL pushdown, text, ordered-index fast path, cursors, pagination and
//! counts.

use std::collections::HashMap;

use async_graphql::Value;
use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata};
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
    metadata: HashMap<String, QueryTypeMetadata>,
}

fn build_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // Authors: Paul(40), Ada(36), Bob(25); `name` unique; ages distinct.
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

    // Books with searchable titles linked to authors via `author`.
    let book_inverses = vec![InverseInfo {
        field: "author".to_string(),
        inverse_type: "Author".to_string(),
        inverse_field: "books".to_string(),
        inverse_is_list: true,
    }];
    let mut author_search = HashMap::new();
    author_search.insert("title".to_string(), vec!["term".to_string(), "fulltext".to_string()]);
    for (uid, title, author) in [
        (201u64, "Planner Internals", 101u64),
        (202, "Query Engines", 101),
        (203, "Cooking Basics", 102),
    ] {
        resolver
            .create_node_internal(
                "Book",
                uid,
                str_map(&[
                    ("title", serde_json::json!(title)),
                    ("author", serde_json::json!(author.to_string())),
                ]),
                &[],
                &book_inverses,
                &author_search,
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

    Fixture { _dir: dir, resolver, metadata }
}

fn scan(fx: &Fixture, filter: HashMap<String, Value>) -> Vec<u64> {
    fx.resolver.scan_nodes_internal(
        "Author",
        filter,
        HashMap::new(),
        None,
        None,
        None,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    )
}

fn sorted(mut v: Vec<u64>) -> Vec<u64> {
    v.sort();
    v
}

fn gql(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn gq(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else {
                Value::Number(async_graphql::Number::from_f64(n.as_f64().unwrap()).unwrap())
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(items) => {
            Value::List(items.into_iter().map(gq).collect())
        }
        other => panic!("unsupported fixture literal: {}", other),
    }
}

fn op_obj(op: &str, v: serde_json::Value) -> Value {
    let mut m = async_graphql::indexmap::IndexMap::new();
    m.insert(async_graphql::Name::new(op), gq(v));
    Value::Object(m)
}

#[test]
fn unfiltered_full_scan_streams_every_row() {
    let fx = build_fixture();
    assert_eq!(scan(&fx, HashMap::new()), vec![101, 102, 103]);
}

#[test]
fn unique_lookup_hit_and_miss() {
    let fx = build_fixture();
    let hit = scan(&fx, gql(&[("name", Value::from("Ada"))]));
    assert_eq!(hit, vec![102]);
    let miss = scan(&fx, gql(&[("name", Value::from("Nobody"))]));
    assert_eq!(miss, Vec::<u64>::new());
}

#[test]
fn pushdown_comparisons_match_expected_sets() {
    let fx = build_fixture();
    let gt30 = scan(&fx, gql(&[("age", op_obj("gt", serde_json::json!(30)))]));
    assert_eq!(sorted(gt30), vec![101, 102]);
    let in_set = scan(
        &fx,
        gql(&[("age", op_obj("in", serde_json::json!([25, 40])))]),
    );
    assert_eq!(sorted(in_set), vec![101, 103]);
    // NOTE legacy `contains` pushdown has no type argument (parity quirk).
    let contains = scan(&fx, gql(&[("name", op_obj("contains", serde_json::json!("aul")))]));
    assert_eq!(contains, vec![101]);
}

#[test]
fn text_search_routes_through_bm25_source() {
    let fx = build_fixture();
    // Text predicates divert to the BM25 path before candidate planning.
    let all_terms = fx.resolver.scan_nodes_internal(
        "Book",
        gql(&[("title", op_obj("allofterms", serde_json::json!("planner internals")))]),
        HashMap::new(),
        None,
        None,
        None,
        &[],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(all_terms, vec![201]);
    let any_text = fx.resolver.scan_nodes_internal(
        "Book",
        gql(&[("title", op_obj("anyoftext", serde_json::json!("engines")))]),
        HashMap::new(),
        None,
        None,
        None,
        &[],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(any_text, vec![202]);
}

#[test]
fn sorted_scans_use_order_index_and_fall_back() {
    let fx = build_fixture();
    let mut sort_asc = HashMap::new();
    sort_asc.insert("age".to_string(), Value::Enum(async_graphql::Name::new("ASC")));
    let asc = fx.resolver.scan_nodes_internal(
        "Author",
        HashMap::new(),
        sort_asc.clone(),
        None,
        None,
        None,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(asc, vec![103, 102, 101]); // ages 25,36,40

    let mut sort_desc = HashMap::new();
    sort_desc.insert("age".to_string(), Value::Enum(async_graphql::Name::new("DESC")));
    let desc = fx.resolver.scan_nodes_internal(
        "Author",
        HashMap::new(),
        sort_desc,
        None,
        None,
        None,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(desc, vec![101, 102, 103]);

    // Type without an order-indexable history still sorts physically.
    let mut sort_name = HashMap::new();
    sort_name.insert("title".to_string(), Value::Enum(async_graphql::Name::new("ASC")));
    let books = fx.resolver.scan_nodes_internal(
        "Book",
        HashMap::new(),
        sort_name,
        None,
        None,
        None,
        &[],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(books.len(), 3);
    assert_eq!(sorted(books), vec![201, 202, 203]);
}

#[test]
fn cursor_semantics_all_three_legacy_behaviors() {
    let fx = build_fixture();

    // Unfiltered stream: absent cursor yields nothing (seek).
    let none_unfiltered = scan(&fx, HashMap::new())
        .is_empty();
    let seek_absent = fx.resolver.scan_nodes_internal(
        "Author",
        HashMap::new(),
        HashMap::new(),
        None,
        Some("9999".to_string()),
        None,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert!(seek_absent.is_empty());
    assert!(!none_unfiltered);

    // Present cursor is strictly-after in both filtered and unfiltered paths.
    let present = scan(&fx, gql(&[("age", op_obj("gt", serde_json::json!(0)))]));
    assert_eq!(sorted(present), vec![101, 102, 103]);

    // Filtered set-based path keeps all rows when the cursor is absent.
    let keep_all = fx.resolver.scan_nodes_internal(
        "Author",
        gql(&[("age", op_obj("gt", serde_json::json!(0)))]),
        HashMap::new(),
        None,
        Some("9999".to_string()),
        None,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(keep_all, vec![101, 102, 103]);

    // Sorted fast-path with absent cursor yields empty (legacy seen_after).
    let mut sort_age = HashMap::new();
    sort_age.insert("age".to_string(), Value::Enum(async_graphql::Name::new("ASC")));
    let sorted_absent = fx.resolver.scan_nodes_internal(
        "Author",
        HashMap::new(),
        sort_age,
        None,
        Some("9999".to_string()),
        None,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert!(sorted_absent.is_empty());
}

#[test]
fn pagination_offset_first_combinations() {
    let fx = build_fixture();
    let page = fx.resolver.scan_nodes_internal(
        "Author",
        HashMap::new(),
        HashMap::new(),
        Some(1),
        Some("101".to_string()),
        Some(1),
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    // Legacy order: positional cursor skip, THEN offset, THEN truncate.
    assert_eq!(page, vec![103]);
}

#[test]
fn counts_across_shapes() {
    let fx = build_fixture();
    let plain = fx.resolver.count_nodes_internal(
        "Author",
        HashMap::new(),
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(plain, 3);

    let filtered = fx.resolver.count_nodes_internal(
        "Author",
        gql(&[("age", op_obj("ge", serde_json::json!(36)))]),
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(filtered, 2);

    let text_count = fx.resolver.count_nodes_internal(
        "Book",
        gql(&[("title", op_obj("allofterms", serde_json::json!("query")))]),
        &[],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(text_count, 1);

    // Unique-miss narrows to an authoritative empty set.
    let miss = fx.resolver.count_nodes_internal(
        "Author",
        gql(&[("name", Value::from("Ghost"))]),
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(miss, 0);
}
