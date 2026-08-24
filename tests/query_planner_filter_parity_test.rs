//! M3 filter-operator tests: the residual [`FilterOperator`] must be
//! observationally identical to the legacy `check_filter_recursive_cached`
//! evaluation for every scalar/logical/relation shape, and ordering metadata
//! must survive filtering so Stage 2.1 sort elimination stays sound.

use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata};
use vardadb::query_planner::ir::{
    FieldPath, FilterOp, LogicalFilter, QueryValue, SortDirection,
};
use vardadb::query_planner::lowering::lower_filter_map;
use vardadb::query_planner::operators::{
    build_source_tree, ExecContext, ExecOperator, FilterOperator, FlowResult, FullTypeScan,
    OrderedIndexScan,
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

    let book_inverses = [InverseInfo {
        field: "author".to_string(),
        inverse_type: "Author".to_string(),
        inverse_field: "books".to_string(),
        inverse_is_list: true,
    }];
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

    Fixture {
        _dir: dir,
        resolver,
        metadata,
        paul: 101,
        ada: 102,
        bob: 103,
    }
}

fn exec_op(op: &dyn ExecOperator, fx: &Fixture) -> Vec<u64> {
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

fn filtered(fx: &Fixture, input: Box<dyn ExecOperator>, filter: LogicalFilter) -> Vec<u64> {
    exec_op(FilterOperator::boxed(input, filter).as_ref(), fx)
}

fn pred(field: &str, op: FilterOp, value: QueryValue) -> LogicalFilter {
    LogicalFilter::Predicate(vardadb::query_planner::ir::FilterPredicate {
        path: FieldPath::field(field),
        op,
        value,
    })
}

/// Parity harness mirroring the M5 cutover shape: source tree from the access
/// planner plus a residual filter operator, compared against the legacy
/// `scan_nodes_internal` output for the same raw GraphQL filter map.
fn assert_parity(fx: &Fixture, type_name: &str, filter_map: &HashMap<String, async_graphql::Value>) {
    let uniques: Vec<String> = fx
        .metadata
        .get(type_name)
        .map(|m| m.uniques.clone())
        .unwrap_or_default();

    let legacy = fx.resolver.scan_nodes_internal(
        type_name,
        filter_map.clone(),
        HashMap::new(),
        None,
        None,
        None,
        &uniques,
        None,
        &fx.metadata,
        None,
    );

    let plan = plan_candidates("default", type_name, filter_map, &uniques, &fx.metadata);
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let got: Vec<u64> = match build_source_tree(type_name, &plan.source) {
        Ok(source) => {
            // M5 cutover shape: source tree plus residual filter operator.
            let pipeline: Box<dyn ExecOperator> = match &plan.residual {
                Some(f) if !f.is_empty_conjunction() => FilterOperator::boxed(source, f.clone()),
                _ => source,
            };
            exec_op(pipeline.as_ref(), fx)
        }
        Err(_) if plan.source.kind() == "relation_expansion" => {
            // Until M7 lands operator subplans, relation-expansion sources
            // execute through the CandidatePlan bridge (nested candidates +
            // residual verification inside the planner).
            plan.execute_uids(&rt).unwrap_or_default()
        }
        Err(e) => panic!("source tree failed for {filter_map:?}: {e}"),
    };
    let mut got = got;
    got.sort_unstable();

    let mut expected = legacy;
    expected.sort_unstable();
    assert_eq!(
        got,
        expected,
        "pipeline/legacy mismatch for {type_name} {filter_map:?}"
    );
}

#[test]
fn scalar_eq_residual_filters_rows() {
    let fx = build_fixture();
    let got = filtered(
        &fx,
        Box::new(FullTypeScan::new("Author")),
        pred("name", FilterOp::Eq, QueryValue::String("Ada".into())),
    );
    assert_eq!(got, vec![fx.ada]);
}

#[test]
fn comparison_ops_match_legacy_semantics() {
    let fx = build_fixture();
    let scan = || Box::new(FullTypeScan::new("Author")) as Box<dyn ExecOperator>;

    let gt = filtered(&fx, scan(), pred("age", FilterOp::Gt, QueryValue::Int(30)));
    assert_eq!(gt, vec![fx.paul, fx.ada]);

    let le = filtered(&fx, scan(), pred("age", FilterOp::Le, QueryValue::Int(36)));
    assert_eq!(le, vec![fx.ada, fx.bob]);

    let ne = filtered(&fx, scan(), pred("age", FilterOp::Ne, QueryValue::Int(40)));
    assert_eq!(ne, vec![fx.ada, fx.bob]);
}

#[test]
fn and_or_not_structure_evaluation() {
    let fx = build_fixture();
    // (age >= 36 OR name = "Bob") AND NOT age = 25 -> Paul, Ada
    let filter = LogicalFilter::And(vec![
        LogicalFilter::Or(vec![
            pred("age", FilterOp::Ge, QueryValue::Int(36)),
            pred("name", FilterOp::Eq, QueryValue::String("Bob".into())),
        ]),
        LogicalFilter::Not(Box::new(pred("age", FilterOp::Eq, QueryValue::Int(25)))),
    ]);
    let got = filtered(
        &fx,
        Box::new(FullTypeScan::new("Author")),
        filter,
    );
    assert_eq!(got, vec![fx.paul, fx.ada]);
}

#[test]
fn empty_or_matches_legacy_vacuous_false() {
    let fx = build_fixture();
    let got = filtered(
        &fx,
        Box::new(FullTypeScan::new("Author")),
        LogicalFilter::Or(vec![]),
    );
    assert!(got.is_empty());

    let all = filtered(
        &fx,
        Box::new(FullTypeScan::new("Author")),
        LogicalFilter::And(vec![]),
    );
    assert_eq!(all.len(), 3);
}

#[test]
fn contains_and_in_operators() {
    let fx = build_fixture();
    let scan = || Box::new(FullTypeScan::new("Author")) as Box<dyn ExecOperator>;

    let contains = filtered(
        &fx,
        scan(),
        pred("name", FilterOp::Contains, QueryValue::String("a".into())),
    );
    assert_eq!(contains, vec![fx.paul, fx.ada]);

    let in_list = filtered(
        &fx,
        scan(),
        pred(
            "age",
            FilterOp::In,
            QueryValue::List(vec![QueryValue::Int(25), QueryValue::Int(40)]),
        ),
    );
    assert_eq!(in_list, vec![fx.paul, fx.bob]);
}

#[test]
fn between_lowers_to_ge_le_pair_and_stays_parity() {
    let fx = build_fixture();
    let mut filter_map: HashMap<String, async_graphql::Value> = HashMap::new();
    filter_map.insert(
        "age".to_string(),
        async_graphql::Value::Object(async_graphql::indexmap::IndexMap::from([
            (
                async_graphql::Name::new("between"),
                async_graphql::Value::List(vec![
                    async_graphql::Value::from(30i64),
                    async_graphql::Value::from(40i64),
                ]),
            ),
        ])),
    );

    // Lowering contract: between becomes Ge + Le.
    let lowered = lower_filter_map(&filter_map);
    let parts = match &lowered {
        LogicalFilter::And(parts) => parts.clone(),
        other => panic!("expected conjunction, got {:?}", other),
    };
    let inner = match parts.as_slice() {
        [LogicalFilter::And(inner)] => inner.clone(),
        other => panic!("expected nested pair, got {:?}", other),
    };
    let ops: Vec<FilterOp> = inner
        .iter()
        .map(|p| match p {
            LogicalFilter::Predicate(pr) => pr.op,
            other => panic!("expected predicate, got {:?}", other),
        })
        .collect();
    assert_eq!(ops, vec![FilterOp::Ge, FilterOp::Le]);

    assert_parity(&fx, "Author", &filter_map);
}

#[test]
fn relation_traversal_single_reference() {
    let fx = build_fixture();
    // Books whose author is older than 38 -> 201, 202 (Paul, 40).
    let filter = LogicalFilter::Relation {
        field: "author".to_string(),
        target_type: "Author".to_string(),
        filter: Box::new(pred("age", FilterOp::Gt, QueryValue::Int(38))),
    };
    let got = filtered(
        &fx,
        Box::new(FullTypeScan::new("Book")),
        filter,
    );
    assert_eq!(got, vec![201, 202]);
}

#[test]
fn relation_traversal_list_any_match() {
    let fx = build_fixture();
    // Authors with a book containing "Query" -> Paul only.
    let filter = LogicalFilter::Relation {
        field: "books".to_string(),
        target_type: "Book".to_string(),
        filter: Box::new(pred(
            "title",
            FilterOp::Contains,
            QueryValue::String("Query".into()),
        )),
    };
    let got = filtered(
        &fx,
        Box::new(FullTypeScan::new("Author")),
        filter,
    );
    assert_eq!(got, vec![fx.paul]);
}

#[test]
fn ordering_preserved_through_filter() {
    let fx = build_fixture();
    let ordered = OrderedIndexScan {
        type_name: "Author".to_string(),
        field: "age".to_string(),
        direction: SortDirection::Asc,
        cursor: None,
        limit: None,
    };
    let key = vardadb::query_planner::ir::OrderKey {
        path: FieldPath::field("age"),
        direction: SortDirection::Asc,
    };

    let op = FilterOperator::boxed(
        Box::new(ordered),
        pred("age", FilterOp::Le, QueryValue::Int(38)),
    );
    assert!(op.output_ordering().satisfies(&key));

    let got = exec_op(op.as_ref(), &fx);
    // Ages ascend: Bob(25), Ada(36).
    assert_eq!(got, vec![fx.bob, fx.ada]);
}

#[test]
fn text_predicates_pass_residual_like_legacy() {
    let fx = build_fixture();
    let got = filtered(
        &fx,
        Box::new(FullTypeScan::new("Author")),
        pred(
            "name",
            FilterOp::AllOfTerms,
            QueryValue::String("never-indexed-here".into()),
        ),
    );
    // Legacy check_condition ignores text ops in residual evaluation;
    // authoritative filtering happens at the BM25 source operator.
    assert_eq!(got.len(), 3);
}

#[test]
fn pipeline_matches_legacy_on_varied_filters() {
    let fx = build_fixture();

    let mk = |k: &str, v: async_graphql::Value| -> HashMap<String, async_graphql::Value> {
        let mut m = HashMap::new();
        m.insert(k.to_string(), v);
        m
    };
    let obj = |entries: &[(&str, async_graphql::Value)]| {
        async_graphql::Value::Object(async_graphql::indexmap::IndexMap::from_iter(
            entries.iter().map(|(k, v)| (async_graphql::Name::new(*k), v.clone())),
        ))
    };

    // Scalar pushdown + residual combination.
    assert_parity(
        &fx,
        "Author",
        &mk("age", obj(&[("gt", async_graphql::Value::from(26i64))])),
    );
    // Multiple conditions on one field.
    assert_parity(
        &fx,
        "Author",
        &mk(
            "age",
            obj(&[
                ("gt", async_graphql::Value::from(20i64)),
                ("lt", async_graphql::Value::from(41i64)),
            ]),
        ),
    );
    // Relation conjunct (legacy recursive expansion path).
    let mut rel: HashMap<String, async_graphql::Value> = HashMap::new();
    rel.insert(
        "books".to_string(),
        obj(&[(
            "title",
            obj(&[("anyofterms", async_graphql::Value::from("planner"))]),
        )]),
    );
    assert_parity(&fx, "Author", &rel);
}
