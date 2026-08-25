//! M7 nested-subplan parity: `RelationExpansion` composes its child operator
//! subplan directly inside `build_source_tree`, replacing the Phase-1
//! nested-candidates runtime bridge. Filter shapes mirror the legacy
//! end-to-end coverage in `nested_filtering_test.rs` (User -> posts -> Post).

use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata};
use vardadb::query_planner::operators::{
    build_source_tree, ExecContext, ExecOperator, FilterOperator, FlowResult,
};
use vardadb::query_planner::{plan_candidates, runtime_for};
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

fn s(v: &str) -> async_graphql::Value {
    async_graphql::Value::String(v.to_string())
}

fn obj(pairs: &[(&str, async_graphql::Value)]) -> async_graphql::Value {
    let mut map = async_graphql::indexmap::IndexMap::new();
    for (k, v) in pairs {
        map.insert(async_graphql::Name::new(k), v.clone());
    }
    async_graphql::Value::Object(map)
}

fn gql_map(pairs: &[(&str, async_graphql::Value)]) -> HashMap<String, async_graphql::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

struct Fixture {
    _dir: tempfile::TempDir,
    resolver: SqliteResolver,
    metadata: HashMap<String, QueryTypeMetadata>,
    alice: u64,
    bob: u64,
}

fn build_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // Posts first: "Intro to Rust" (301) and "Advanced Go" (302), each linked
    // back to its owner through the inverse edge field `user`.
    let post_inverses = [InverseInfo {
        field: "user".to_string(),
        inverse_type: "User".to_string(),
        inverse_field: "posts".to_string(),
        inverse_is_list: true,
    }];
    for (uid, title, owner, likes) in [
        (301u64, "Intro to Rust", 101u64, 10i64),
        (302, "Advanced Go", 102, 2),
    ] {
        resolver
            .create_node_internal(
                "Post",
                uid,
                str_map(&[
                    ("title", serde_json::json!(title)),
                    ("user", serde_json::json!(owner.to_string())),
                    ("likes", serde_json::json!(likes)),
                ]),
                &[],
                &post_inverses,
                &HashMap::new(),
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    // Users: Alice owns the Rust post, Bob the Go post.
    for (uid, name) in [(101u64, "Alice"), (102, "Bob")] {
        resolver
            .create_node_internal(
                "User",
                uid,
                str_map(&[("name", serde_json::json!(name))]),
                &["name".to_string()],
                &[],
                &HashMap::new(),
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let mut metadata = HashMap::new();
    metadata.insert(
        "User".to_string(),
        QueryTypeMetadata {
            uniques: vec!["name".to_string()],
            inverses: vec![InverseInfo {
                field: "posts".to_string(),
                inverse_type: "Post".to_string(),
                inverse_field: "user".to_string(),
                inverse_is_list: true,
            }],
            relations: HashMap::from([("posts".to_string(), "Post".to_string())]),
        },
    );
    metadata.insert(
        "Post".to_string(),
        QueryTypeMetadata {
            uniques: vec![],
            inverses: vec![],
            relations: HashMap::new(),
        },
    );

    Fixture {
        _dir: dir,
        resolver,
        metadata,
        alice: 101,
        bob: 102,
    }
}

/// Plan -> source-tree pipeline (the M6/M7 production shape), returning uids.
fn pipeline_uids(fx: &Fixture, type_name: &str, filter: &HashMap<String, async_graphql::Value>) -> Vec<u64> {
    let plan = plan_candidates("default", type_name, filter, &[], &fx.metadata);
    assert!(
        !matches!(
            plan.source,
            vardadb::query_planner::CandidateSource::FullTypeScan
        ) || filter.is_empty(),
        "expected narrowing for {filter:?}, got {:?}",
        plan.source
    );
    let source = build_source_tree(type_name, &plan.source)
        .unwrap_or_else(|e| panic!("source tree failed for {filter:?}: {e}"));
    let pipeline: Box<dyn ExecOperator> = match &plan.residual {
        Some(f) if !f.is_empty_conjunction() => FilterOperator::boxed(source, f.clone()),
        _ => source,
    };
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    match pipeline.execute(&mut ctx) {
        FlowResult::Rows(batches) => {
            let mut uids: Vec<u64> = batches
                .into_iter()
                .flat_map(|b| b.0.into_iter().map(|e| e.uid))
                .collect();
            uids.sort_unstable();
            uids
        }
        other => panic!("expected rows, got error={}", other.is_error()),
    }
}

/// Legacy reference implementation for the same nested shape.
fn legacy_uids(fx: &Fixture, type_name: &str, filter: &HashMap<String, async_graphql::Value>) -> Vec<u64> {
    let mut uids = fx.resolver.scan_nodes_internal(
        type_name,
        filter.clone(),
        HashMap::new(),
        None,
        None,
        None,
        &[],
        None,
        &fx.metadata,
        None,
    );
    uids.sort_unstable();
    uids
}

#[test]
fn nested_contains_rust_matches_owner_only() {
    let fx = build_fixture();
    let filter = gql_map(&[("posts", obj(&[("title", obj(&[("contains", s("Rust"))]))]))]);
    let expected = legacy_uids(&fx, "User", &filter);
    assert_eq!(expected, vec![fx.alice], "legacy baseline");
    assert_eq!(pipeline_uids(&fx, "User", &filter), expected);
}

#[test]
fn nested_contains_go_matches_other_owner_only() {
    let fx = build_fixture();
    let filter = gql_map(&[("posts", obj(&[("title", obj(&[("contains", s("Go"))]))]))]);
    let expected = legacy_uids(&fx, "User", &filter);
    assert_eq!(expected, vec![fx.bob], "legacy baseline");
    assert_eq!(pipeline_uids(&fx, "User", &filter), expected);
}

#[test]
fn nested_scalar_conjunction_intersects_with_relation() {
    let fx = build_fixture();
    // Alice AND posts containing "Go" => empty (her post mentions Rust).
    let filter = gql_map(&[
        ("name", obj(&[("eq", s("Alice"))])),
        ("posts", obj(&[("title", obj(&[("contains", s("Go"))]))])),
    ]);
    let expected = legacy_uids(&fx, "User", &filter);
    assert_eq!(expected, Vec::<u64>::new(), "legacy baseline");
    assert_eq!(pipeline_uids(&fx, "User", &filter), expected);

    // Same shape against Bob's own name narrows to Bob only.
    let filter = gql_map(&[
        ("name", obj(&[("eq", s("Bob"))])),
        ("posts", obj(&[("title", obj(&[("contains", s("Go"))]))])),
    ]);
    let expected = legacy_uids(&fx, "User", &filter);
    assert_eq!(expected, vec![fx.bob], "legacy baseline");
    assert_eq!(pipeline_uids(&fx, "User", &filter), expected);
}

#[test]
fn nested_conjunct_subfilter_prefetches_both_fields() {
    // Two-field conjunct inside the relation sub-filter: the hash-join
    // prefetch must batch-load BOTH referenced child fields in one pass and
    // keep parity with the legacy residual walk.
    let fx = build_fixture();
    let filter = gql_map(&[(
        "posts",
        obj(&[
            ("likes", obj(&[("ge", async_graphql::Value::Number(5.into()))])),
            ("title", obj(&[("contains", s("Rust"))])),
        ]),
    )]);
    let expected = legacy_uids(&fx, "User", &filter);
    assert_eq!(expected, vec![fx.alice], "legacy baseline");
    assert_eq!(pipeline_uids(&fx, "User", &filter), expected);

    // Flip the conjunct: low likes + Go title isolates Bob instead.
    let filter = gql_map(&[(
        "posts",
        obj(&[
            ("likes", obj(&[("le", async_graphql::Value::Number(5.into()))])),
            ("title", obj(&[("contains", s("Go"))])),
        ]),
    )]);
    let expected = legacy_uids(&fx, "User", &filter);
    assert_eq!(expected, vec![fx.bob], "legacy baseline");
    assert_eq!(pipeline_uids(&fx, "User", &filter), expected);
}

#[test]
fn inline_list_edges_any_match_semantics() {
    // Diamond shape: two parents share child posts through INLINE list edges
    // (stored directly on the rows, no inverse-edge resolution). Both must
    // pass alongside the inverse-edge user — list edges match when ANY
    // referenced child satisfies the sub-filter.
    use vardadb::query_planner::lower_filter_map;
    use vardadb::query_planner::operators::{ExecContext, FullTypeScan};

    let fx = build_fixture();
    for (uid, name, edges) in [
        (201u64, "Carol", vec![serde_json::json!("301")]),
        (202, "Dan", vec![serde_json::json!("301"), serde_json::json!("302")]),
    ] {
        fx.resolver
            .create_node_internal(
                "User",
                uid,
                str_map(&[
                    ("name", serde_json::json!(name)),
                    ("posts", serde_json::Value::Array(edges)),
                ]),
                &[],
                &[],
                &HashMap::new(),
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let filter = gql_map(&[("posts", obj(&[("title", obj(&[("contains", s("Rust"))]))]))]);
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    let pipeline =
        FilterOperator::boxed(Box::new(FullTypeScan::new("User")), lower_filter_map(&filter));
    match pipeline.execute(&mut ctx) {
        FlowResult::Rows(batches) => {
            let mut uids: Vec<u64> = batches
                .into_iter()
                .flat_map(|b| b.0.into_iter().map(|e| e.uid))
                .collect();
            uids.sort_unstable();
            assert_eq!(
                uids,
                vec![101, 201, 202],
                "Alice (inverse edge), Carol + Dan (inline lists sharing post 301)"
            );
        }
        other => panic!("expected rows, got error={}", other.is_error()),
    }
}
