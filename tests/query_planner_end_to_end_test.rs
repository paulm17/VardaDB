//! Stage 3.5 end-to-end coverage: every production read entry point
//! (`scan_nodes_internal`, `resolve_list_internal`, `count_nodes_internal`)
//! plus the 3.4 bridge helpers run on the planner-first pipeline over one
//! shared fixture. Vector/hybrid paths are exercised by hybrid_search_test
//! (background flush), so they are intentionally not duplicated here.

use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata};
use vardadb::query_planner::ir::{
    AggregateFunction, AggregateSpec, EntityId, FieldPath, LogicalExpr, QueryValue,
};
use vardadb::query_planner::operators::{ExecOperator, FullTypeScan, HashAggregateOperator};
use vardadb::query_planner::{
    candidates_or_legacy, compile_aggregates, field_value, plan_candidates, runtime_for,
};
use vardadb::realtime::bus::MutationSource;
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

fn ts(counter: u16) -> Timestamp {
    Timestamp::new(1_700_000_000_000, counter, 1)
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

    for (uid, name, age) in [(101u64, "Paul", 40i64), (102, "Ada", 36), (103, "Bob", 25)] {
        resolver
            .create_node_internal(
                "Author",
                uid,
                [
                    ("name".to_string(), serde_json::json!(name)),
                    ("age".to_string(), serde_json::json!(age)),
                ]
                .into_iter()
                .collect(),
                &["name".to_string()],
                &[],
                &HashMap::new(),
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let book_inverses = [InverseInfo {
        field: "author".to_string(),
        inverse_type: "Author".to_string(),
        inverse_field: "books".to_string(),
        inverse_is_list: true,
    }];
    let mut search_fields = HashMap::new();
    search_fields.insert(
        "title".to_string(),
        vec!["term".to_string(), "fulltext".to_string()],
    );
    for (uid, title, author) in [
        (201u64, "Planner Internals", 101u64),
        (202, "Query Engines", 101),
        (203, "Cooking Basics", 102),
    ] {
        resolver
            .create_node_internal(
                "Book",
                uid,
                [
                    ("title".to_string(), serde_json::json!(title)),
                    ("author".to_string(), serde_json::json!(author.to_string())),
                ]
                .into_iter()
                .collect(),
                &[],
                &book_inverses,
                &search_fields,
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let metadata = [
        (
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
        ),
        (
            "Book".to_string(),
            QueryTypeMetadata {
                uniques: vec![],
                inverses: vec![],
                relations: HashMap::from([("author".to_string(), "Author".to_string())]),
            },
        ),
    ]
    .into_iter()
    .collect();

    Fixture {
        _dir: dir,
        resolver,
        metadata,
    }
}

fn gql_obj(pairs: &[(&str, async_graphql::Value)]) -> async_graphql::Value {
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

fn s(v: &str) -> async_graphql::Value {
    async_graphql::Value::String(v.to_string())
}

#[test]
fn scan_pipeline_handles_sort_pagination_and_text() {
    let fx = build_fixture();

    // age DESC + first 2 => Paul(40), Ada(36).
    let mut sort = HashMap::new();
    sort.insert("age".to_string(), s("DESC"));
    let got = fx
        .resolver
        .scan_nodes_internal(
            "Author",
            HashMap::new(),
            sort,
            Some(2),
            None,
            None,
            &["name".to_string()],
            None,
            &fx.metadata,
            None,
        )
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(got, vec![101, 102]);

    // name ASC + offset 1 skips Ada => Bob(103), Paul(101).
    let mut sort = HashMap::new();
    sort.insert("name".to_string(), s("ASC"));
    let got = fx
        .resolver
        .scan_nodes_internal(
            "Author",
            HashMap::new(),
            sort,
            None,
            None,
            Some(1),
            &["name".to_string()],
            None,
            &fx.metadata,
            None,
        )
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(got, vec![103, 101]);

    // Text search routes through TextBM25Scan inside the same pipeline.
    let filter = gql_map(&[("title", gql_obj(&[("allofterms", s("planner"))]))]);
    let got = fx
        .resolver
        .scan_nodes_internal(
            "Book",
            filter,
            HashMap::new(),
            None,
            None,
            None,
            &[],
            None,
            &fx.metadata,
            None,
        )
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(got, vec![201]);
}

#[test]
fn relation_pipeline_resolves_nested_filters_end_to_end() {
    let fx = build_fixture();

    // Authors owning a book whose title contains "Cooking" => Ada only.
    let nested = gql_obj(&[("title", gql_obj(&[("contains", s("Cooking"))]))]);
    let filter = gql_map(&[("books", nested)]);
    let got = fx
        .resolver
        .scan_nodes_internal(
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
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(got, vec![102]);

    // The public Resolver edge (production relation pipeline): Paul's books.
    use vardadb::engine::resolver::Resolver;
    let got = fx
        .resolver
        .resolve_list(101, "books", HashMap::new(), HashMap::new(), None, None, None, None)
        .unwrap();
    let mut uids: Vec<u64> = got.into_iter().collect();
    uids.sort_unstable();
    assert_eq!(uids, vec![201, 202]);
}

#[test]
fn count_and_aggregates_flow_through_the_planner() {
    let fx = build_fixture();

    // Unfiltered count takes the O(prefix) fast path.
    assert_eq!(
        fx.resolver
            .count_nodes_internal("Author", HashMap::new(), &[], None, &fx.metadata, None),
        3
    );

    // Filtered count runs the pipeline aggregate; ge 30 keeps Paul and Ada.
    let filter = gql_map(&[("age", gql_obj(&[("ge", async_graphql::Value::Number(30.into()))]))]);
    assert_eq!(
        fx.resolver
            .count_nodes_internal("Author", filter, &[], None, &fx.metadata, None),
        2
    );

    // sum(age) over every author == 40 + 36 + 25 = 101.
    let specs = compile_aggregates(&[AggregateSpec {
        function: AggregateFunction::Sum,
        expr: Some(LogicalExpr::Field(FieldPath::field("age"))),
        alias: "total".to_string(),
    }])
    .unwrap();
    let op = HashAggregateOperator::new(Box::new(FullTypeScan::new("Author")), specs, Vec::new());
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = vardadb::query_planner::operators::ExecContext::new_with_explain(
        &rt,
        "default",
        true,
    );
    match op.execute(&mut ctx) {
        vardadb::query_planner::operators::FlowResult::Rows(_) => {}
        other => panic!("expected rows, got error={}", other.is_error()),
    }
    let groups = op.groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].outputs.iter().find(|(a, _)| a == "total").unwrap().1,
        QueryValue::Int(101)
    );
}

#[test]
fn bridge_helpers_serve_callers_over_live_data() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);

    // candidates_or_legacy narrows through the planned pipeline shape.
    let plan = plan_candidates(
        "default",
        "Author",
        &gql_map(&[("age", gql_obj(&[("ge", async_graphql::Value::Number(30.into()))]))]),
        &["name".to_string()],
        &fx.metadata,
    );
    let narrowed = candidates_or_legacy(&plan, &rt, "default").unwrap().unwrap();
    let mut uids: Vec<u64> = narrowed.into_iter().map(|e| e.uid).collect();
    uids.sort_unstable();
    assert_eq!(uids, vec![101, 102]);

    // field_value evaluates computed expressions against stored rows.
    let expr = LogicalExpr::Binary {
        left: Box::new(LogicalExpr::Field(FieldPath::field("age"))),
        op: vardadb::query_planner::ir::BinaryOp::Add,
        right: Box::new(LogicalExpr::Value(QueryValue::Int(2))),
    };
    let value = field_value(&rt, EntityId::new(101), &expr).unwrap();
    assert_eq!(value, QueryValue::Int(42));
}
