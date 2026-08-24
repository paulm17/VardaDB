//! M4 sort/limit/cursor/offset operator tests.
//!
//! Every ordering/pagination shape is compared against the legacy
//! `scan_nodes_internal` tail semantics (sort -> positional cursor skip ->
//! offset -> limit) so the pipeline can never drift from GraphQL-observed
//! behavior.

use std::collections::HashMap;

use async_graphql::Value;
use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::QueryTypeMetadata;
use vardadb::query_planner::ir::{FieldPath, OrderKey, SortDirection};
use vardadb::query_planner::operators::{
    build_source_tree, CursorSkipOperator, ExecContext, ExecOperator, FlowResult,
    FullTypeScan, LimitOperator, OffsetOperator, OrderedIndexScan, SortOperator,
};
use vardadb::query_planner::runtime_for;
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
    paul: u64,
    ada: u64,
    bob: u64,
}

fn build_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // Authors: Paul(40), Ada(36), Bob(25). `name` is unique; ages distinct so
    // sort comparisons never depend on tie-breaking.
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

    let mut metadata = HashMap::new();
    metadata.insert(
        "Author".to_string(),
        QueryTypeMetadata {
            uniques: vec!["name".to_string()],
            inverses: vec![],
            relations: HashMap::new(),
        },
    );

    Fixture {
        _dir: dir,
        resolver,
        metadata,
        paul: 101,
        ada: 102,
        bob: 103,
    }
}

/// Runs a pipeline and returns the flattened uid sequence in output order.
fn exec_uids(op: &dyn ExecOperator, fx: &Fixture) -> Vec<u64> {
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    match op.execute(&mut ctx) {
        FlowResult::Rows(batches) => batches
            .into_iter()
            .flat_map(|b| b.0.into_iter().map(|e| e.uid))
            .collect(),
        other => panic!("expected rows, got error={}", other.is_error()),
    }
}

/// Legacy reference execution through scan_nodes_internal.
#[allow(clippy::too_many_arguments)]
fn legacy_scan(
    fx: &Fixture,
    sort_field: Option<(&str, bool)>,
    first: Option<usize>,
    after: Option<&str>,
    offset: Option<usize>,
    filter: Option<HashMap<String, Value>>,
) -> Vec<u64> {
    let mut sort = HashMap::new();
    if let Some((field, asc)) = sort_field {
        sort.insert(
            field.to_string(),
            Value::Enum(async_graphql::Name::new(if asc { "ASC" } else { "DESC" })),
        );
    }
    fx.resolver.scan_nodes_internal(
        "Author",
        filter.unwrap_or_default(),
        sort,
        first,
        after.map(str::to_string),
        offset,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    )
}

fn gql_obj(
    pairs: &[(&str, Value)],
) -> HashMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn op_obj(op: &str, v: Value) -> Value {
    let mut map = async_graphql::indexmap::IndexMap::new();
    map.insert(async_graphql::Name::new(op), v);
    Value::Object(map)
}

fn key(field: &str, direction: SortDirection) -> OrderKey {
    OrderKey {
        path: FieldPath::field(field),
        direction,
    }
}

fn full_scan() -> Box<dyn ExecOperator> {
    Box::new(FullTypeScan::new("Author"))
}

#[test]
fn sort_ascending_matches_legacy() {
    let fx = build_fixture();
    let op = SortOperator::boxed(
        full_scan(),
        vec![key("age", SortDirection::Asc)],
    );
    assert_eq!(exec_uids(op.as_ref(), &fx), vec![fx.bob, fx.ada, fx.paul]);
    assert_eq!(
        exec_uids(op.as_ref(), &fx),
        legacy_scan(&fx, Some(("age", true)), None, None, None, None)
    );
}

#[test]
fn sort_descending_matches_legacy() {
    let fx = build_fixture();
    let op = SortOperator::boxed(
        full_scan(),
        vec![key("age", SortDirection::Desc)],
    );
    assert_eq!(exec_uids(op.as_ref(), &fx), vec![fx.paul, fx.ada, fx.bob]);
    assert_eq!(
        exec_uids(op.as_ref(), &fx),
        legacy_scan(&fx, Some(("age", false)), None, None, None, None)
    );
}

#[test]
fn string_sort_matches_legacy() {
    let fx = build_fixture();
    let op = SortOperator::boxed(
        full_scan(),
        vec![key("name", SortDirection::Asc)],
    );
    // Ada < Bob < Paul lexicographically.
    assert_eq!(exec_uids(op.as_ref(), &fx), vec![fx.ada, fx.bob, fx.paul]);
    assert_eq!(
        exec_uids(op.as_ref(), &fx),
        legacy_scan(&fx, Some(("name", true)), None, None, None, None)
    );
}

#[test]
fn sort_eliminated_when_input_declares_ordering() {
    let fx = build_fixture();
    let source = OrderedIndexScan {
        type_name: "Author".to_string(),
        field: "age".to_string(),
        direction: SortDirection::Asc,
        cursor: None,
        limit: None,
    };
    let op = SortOperator::new(
        Box::new(source),
        vec![key("age", SortDirection::Asc)],
    );

    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    // Explain capture is off unless VARDADB_DEBUG is set; force it on here.
    ctx.explain = vardadb::query_planner::operators::ExplainCapture::new(true);
    match op.execute(&mut ctx) {
        FlowResult::Rows(batches) => {
            let uids: Vec<u64> = batches
                .into_iter()
                .flat_map(|b| b.0.into_iter().map(|e| e.uid))
                .collect();
            assert_eq!(uids, vec![fx.bob, fx.ada, fx.paul]);
        }
        other => panic!("expected rows, got error={}", other.is_error()),
    }
    let stats = ctx.explain.stats();
    assert!(
        stats
            .iter()
            .any(|s| s.notes.iter().any(|n| n.contains("eliminated"))),
        "expected a sort-elimination note, got {:?}",
        stats.iter().flat_map(|s| s.notes.iter()).collect::<Vec<_>>()
    );

    // Conflicting direction must NOT eliminate: re-sort descending.
    let source = OrderedIndexScan {
        type_name: "Author".to_string(),
        field: "age".to_string(),
        direction: SortDirection::Asc,
        cursor: None,
        limit: None,
    };
    let op = SortOperator::new(
        Box::new(source),
        vec![key("age", SortDirection::Desc)],
    );
    assert_eq!(exec_uids(&op, &fx), vec![fx.paul, fx.ada, fx.bob]);
    let mut ctx2 = ExecContext::new(&rt, "default");
    ctx2.explain = vardadb::query_planner::operators::ExplainCapture::new(true);
    let _ = op.execute(&mut ctx2).into_rows().unwrap();
    assert!(
        ctx2.explain
            .stats()
            .iter()
            .any(|s| s.notes.iter().any(|n| n.starts_with("sorted"))),
        "conflicting direction must physically sort, got {:?}",
        ctx2.explain
            .stats()
            .iter()
            .flat_map(|s| s.notes.iter())
            .collect::<Vec<_>>()
    );
}

#[test]
fn limit_truncates_and_zero_limit_empties() {
    let fx = build_fixture();
    let op = LimitOperator::boxed(full_scan(), 2);
    let mut got = exec_uids(op.as_ref(), &fx);
    got.sort_unstable();
    assert_eq!(got, vec![fx.paul, fx.ada]);

    let op = LimitOperator::boxed(full_scan(), 0);
    assert!(exec_uids(op.as_ref(), &fx).is_empty());
}

#[test]
fn cursor_skips_positionally_like_legacy() {
    use vardadb::query_planner::operators::CursorSkipOperator as Skip;
    let fx = build_fixture();

    // Present cursor, sorted stream: both paths emit rows strictly after it.
    // Age-ascending order is [bob, ada, paul]; after bob leaves [ada, paul].
    let pipeline = || {
        LimitOperator::boxed(
            Skip::boxed(
                SortOperator::boxed(full_scan(), vec![key("age", SortDirection::Asc)]),
                Some(fx.bob),
            ),
            10,
        )
    };
    assert_eq!(exec_uids(pipeline().as_ref(), &fx), vec![fx.ada, fx.paul]);
    assert_eq!(
        exec_uids(pipeline().as_ref(), &fx),
        legacy_scan(&fx, Some(("age", true)), Some(10), Some("103"), None, None)
    );

    // ABSENT cursor diverges between the two legacy paths:
    //
    // 1. Sorted fast path (sorted_index_scan): yields nothing.
    let seek = Skip::seek(
        SortOperator::boxed(full_scan(), vec![key("age", SortDirection::Asc)]),
        Some(999),
    );
    assert!(exec_uids(&seek, &fx).is_empty());
    assert_eq!(
        exec_uids(&seek, &fx),
        legacy_scan(&fx, Some(("age", true)), None, Some("999"), None, None),
        "sorted legacy path returns empty for an absent cursor"
    );

    // 2. Unsorted streaming path: the legacy range scan starts at key(uid+1),
    //    so an absent cursor also yields nothing (seek semantics).
    let stream_seek = Skip::seek_boxed(full_scan(), Some(999));
    assert!(exec_uids(stream_seek.as_ref(), &fx).is_empty());
    assert_eq!(
        exec_uids(stream_seek.as_ref(), &fx),
        legacy_scan(&fx, None, None, Some("999"), None, None),
        "unsorted range-scan path seeks past the cursor uid"
    );

    // 3. Candidate-set sources keep everything on an absent cursor (the
    //    legacy candidates branch has no key to seek to).
    let pushdown_plan = vardadb::query_planner::plan_candidates(
        "default",
        "Author",
        &gql_obj(&[("age", op_obj("gt", async_graphql::Value::Number(0.into())))]),
        &[],
        &fx.metadata,
    );
    let source = build_source_tree("Author", &pushdown_plan.source).unwrap();
    let keep_all = Skip::boxed(source, Some(999));
    assert_eq!(exec_uids(keep_all.as_ref(), &fx), vec![fx.paul, fx.ada, fx.bob]);
    assert_eq!(
        exec_uids(keep_all.as_ref(), &fx),
        legacy_scan(
            &fx,
            None,
            None,
            Some("999"),
            None,
            Some(gql_obj(&[("age", op_obj("gt", async_graphql::Value::Number(0.into())))]))
        ),
        "candidate-set tail keeps all rows when the cursor is absent"
    );
}

#[test]
fn offset_applies_after_cursor_like_legacy() {
    let fx = build_fixture();
    let pipeline = || {
        LimitOperator::boxed(
            OffsetOperator::boxed(
                CursorSkipOperator::boxed(
                    SortOperator::boxed(full_scan(), vec![key("age", SortDirection::Asc)]),
                    Some(fx.bob),
                ),
                1,
            ),
            10,
        )
    };
    assert_eq!(exec_uids(pipeline().as_ref(), &fx), vec![fx.paul]);
    assert_eq!(
        exec_uids(pipeline().as_ref(), &fx),
        legacy_scan(&fx, Some(("age", true)), Some(10), Some("103"), Some(1), None)
    );
}

#[test]
fn unsorted_pagination_matches_legacy_streaming_path() {
    let fx = build_fixture();
    // No sort: legacy streams the type index ascending and applies offset/limit
    // inline; the pipeline applies them as operators over the same sequence.
    let pipeline = || {
        LimitOperator::boxed(
            OffsetOperator::boxed(CursorSkipOperator::boxed(full_scan(), None), 1),
            1,
        )
    };
    assert_eq!(exec_uids(pipeline().as_ref(), &fx), vec![fx.ada]);
    assert_eq!(
        exec_uids(pipeline().as_ref(), &fx),
        legacy_scan(&fx, None, Some(1), None, Some(1), None)
    );
}

#[test]
fn ordering_metadata_flows_through_pagination_ops() {
    let fx = build_fixture();
    let op = LimitOperator::boxed(
        CursorSkipOperator::boxed(
            Box::new(OrderedIndexScan {
                type_name: "Author".to_string(),
                field: "age".to_string(),
                direction: SortDirection::Asc,
                cursor: None,
                limit: None,
            }),
            None,
        ),
        2,
    );
    assert!(op.output_ordering().satisfies(&key("age", SortDirection::Asc)));
    assert_eq!(
        op.cardinality(),
        vardadb::query_planner::operators::CardinalityHint::Bounded(2)
    );
    assert_eq!(exec_uids(op.as_ref(), &fx), vec![fx.bob, fx.ada]);
}

#[test]
fn planner_pipeline_with_sort_and_pagination_matches_legacy_end_to_end() {
    use vardadb::query_planner::lower_root_query;

    let fx = build_fixture();

    // Lower a real query shape: sorted, cursor-paginated, limited.
    let mut sort_map = HashMap::new();
    sort_map.insert("age".to_string(), Value::Enum(async_graphql::Name::new("DESC")));
    let lq = lower_root_query("Author", &HashMap::new(), &sort_map, Some(2), Some("102".to_string()), None);

    let plan = vardadb::query_planner::plan_candidates(
        "default",
        "Author",
        &HashMap::new(),
        &["name".to_string()],
        &fx.metadata,
    );
    let source = build_source_tree("Author", &plan.source).unwrap();

    let mut keys = Vec::new();
    for k in &lq.order_by {
        keys.push(k.clone());
    }
    let after_uid = match &lq.pagination.after {
        Some(vardadb::query_planner::ir::CursorValue::Entity(e)) => Some(e.uid),
        _ => None,
    };
    let pipeline = LimitOperator::boxed(
        OffsetOperator::boxed(
            CursorSkipOperator::boxed(SortOperator::boxed(source, keys), after_uid),
            lq.pagination.offset.unwrap_or(0),
        ),
        lq.pagination.first.unwrap_or(usize::MAX),
    );

    assert_eq!(
        exec_uids(pipeline.as_ref(), &fx),
        vec![fx.bob],
        "desc order [paul,ada,bob]; after ada(102) emits strictly-later rows -> [bob]"
    );
}
