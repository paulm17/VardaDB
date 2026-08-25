//! Stage 3.4 control-flow operators and the `plan_or_compute` fallback
//! bridge: contract tests for Unsupported-routing, expression evaluation,
//! candidate-pipeline parity, and every control-flow operator's buffering /
//! signal semantics.

use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::QueryTypeMetadata;
use vardadb::query_planner::ir::{
    BinaryOp, EntityId, FieldPath, LogicalExpr, QueryValue,
};
use vardadb::query_planner::operators::{
    ComputeOperator, EmptySource, ExecContext, ExecOperator, ExprValueOperator, FlowResult,
    ForeachOperator, FullTypeScan, IfElseOperator, PlannerError, ReturnOperator,
    SequenceOperator, UnionSources,
};
use vardadb::query_planner::physical_expr::{compile_arc, EvalContext, FieldSource};
use vardadb::query_planner::{
    candidates_or_legacy, field_value, plan_candidates, plan_or_compute, runtime_for,
};
use vardadb::realtime::bus::MutationSource;
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

fn ts(counter: u16) -> Timestamp {
    Timestamp::new(1_700_000_000_000, counter, 1)
}

// -- helpers ------------------------------------------------------------------

fn lit(v: QueryValue) -> LogicalExpr {
    LogicalExpr::Value(v)
}

fn fld(path: &str) -> LogicalExpr {
    LogicalExpr::Field(FieldPath::field(path))
}

fn bin(left: LogicalExpr, op: BinaryOp, right: LogicalExpr) -> LogicalExpr {
    LogicalExpr::Binary {
        left: Box::new(left),
        op,
        right: Box::new(right),
    }
}

fn s(v: &str) -> QueryValue {
    QueryValue::String(v.to_string())
}

fn i(v: i64) -> QueryValue {
    QueryValue::Int(v)
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

    // Authors: Paul(40), Ada(36), Bob(25). `name` is unique.
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

    let metadata = [(
        "Author".to_string(),
        QueryTypeMetadata {
            uniques: vec!["name".to_string()],
            inverses: vec![],
            relations: HashMap::new(),
        },
    )]
    .into_iter()
    .collect();

    Fixture {
        _dir: dir,
        resolver,
        metadata,
    }
}

fn exec_batches(op: &dyn ExecOperator, fx: &Fixture) -> Vec<Vec<u64>> {
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    match op.execute(&mut ctx) {
        FlowResult::Rows(batches) => batches
            .into_iter()
            .map(|b| b.0.into_iter().map(|e| e.uid).collect())
            .collect(),
        FlowResult::Break => panic!("unexpected break"),
        FlowResult::Continue => Vec::new(),
        other => panic!("unexpected flow: error={}", other.is_error()),
    }
}

// -- bridge contract ------------------------------------------------------------

#[test]
fn plan_or_compute_routes_only_unsupported_to_fallback() {
    let ok = plan_or_compute(
        || Ok::<_, PlannerError>(7),
        |_| panic!("fallback must not run on success"),
    );
    assert_eq!(ok.unwrap(), 7);

    let fell_back = plan_or_compute(
        || Err::<usize, _>(PlannerError::Unsupported("no expr support".into())),
        |err| match err {
            PlannerError::Unsupported(detail) => Ok(detail.len()),
            other => panic!("fallback got {other}"),
        },
    );
    assert_eq!(fell_back.unwrap(), "no expr support".len());

    let propagated = plan_or_compute(
        || Err::<i32, _>(PlannerError::Storage("disk full".into())),
        |_| panic!("storage errors must propagate, not fall back"),
    );
    assert!(matches!(propagated, Err(PlannerError::Storage(_))));
}

#[test]
fn field_value_evaluates_scalars_and_computations() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);

    // Bare field reference rides StoredSource's legacy stored-field root.
    assert_eq!(
        field_value(&rt, EntityId::new(101), &fld("age")).unwrap(),
        i(40)
    );

    // Computed expression over stored fields.
    let older = bin(fld("age"), BinaryOp::Add, lit(i(2)));
    assert_eq!(
        field_value(&rt, EntityId::new(101), &older).unwrap(),
        i(42)
    );

    // Missing fields evaluate to Null, not an error.
    assert_eq!(
        field_value(&rt, EntityId::new(102), &fld("nickname")).unwrap(),
        QueryValue::Null
    );

    // Unknown functions surface as Unsupported (the fallback trigger).
    let broken = LogicalExpr::Function {
        name: "nosuchfun".to_string(),
        args: vec![],
    };
    assert!(matches!(
        field_value(&rt, EntityId::new(101), &broken),
        Err(PlannerError::Unsupported(_))
    ));
}

#[test]
fn candidates_pipeline_matches_expected_narrowed_sets() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);

    let mut filter = HashMap::new();
    filter.insert(
        "age".to_string(),
        async_graphql::Value::Object(
            [(async_graphql::Name::new("ge"), async_graphql::Value::Number(30.into()))]
                .into_iter()
                .collect(),
        ),
    );
    let plan = plan_candidates("default", "Author", &filter, &[], &fx.metadata);
    let narrowed =
        candidates_or_legacy(&plan, &rt, "default").unwrap();
    let uids = narrowed.expect("operator runtime supports this shape");
    assert_eq!(
        uids.iter().map(|e| e.uid).collect::<Vec<_>>(),
        vec![101, 102],
        "Paul(40) and Ada(36) pass age>=30, ascending uid"
    );

    // Unfiltered plans narrow to the full type scan.
    let plan = plan_candidates("default", "Author", &HashMap::new(), &[], &fx.metadata);
    let all = candidates_or_legacy(&plan, &rt, "default")
        .unwrap()
        .unwrap();
    assert_eq!(all.iter().map(|e| e.uid).collect::<Vec<_>>(), vec![101, 102, 103]);
}

// -- control-flow operators -------------------------------------------------------

#[test]
fn expr_value_operator_buffers_scalar_results() {
    let expr = compile_arc(&bin(lit(i(1)), BinaryOp::Add, lit(i(1)))).unwrap();
    let op = ExprValueOperator::new(expr);
    assert_eq!(op.kind().as_str(), "expr");
    assert_eq!(op.cardinality().suggested_capacity(), Some(1));
    let fx = build_fixture();
    assert!(exec_batches(&op, &fx).is_empty());
    assert_eq!(op.take_value(), Some(i(2)));
    assert_eq!(op.take_value(), None, "buffer is consumed");
}

#[test]
fn compute_passes_rows_through_and_stashes_values() {
    let fx = build_fixture();
    let fields = vec![
        (
            "who".to_string(),
            compile_arc(&LogicalExpr::Function {
                name: "upper".to_string(),
                args: vec![fld("name")],
            })
            .unwrap(),
        ),
        (
            "twice".to_string(),
            compile_arc(&bin(fld("age"), BinaryOp::Mul, lit(i(2)))).unwrap(),
        ),
    ];
    let op = ComputeOperator::new(Box::new(FullTypeScan::new("Author")), fields);
    assert_eq!(op.kind().as_str(), "project");
    let batches = exec_batches(&op, &fx);
    let uids: Vec<u64> = batches.into_iter().flatten().collect();
    assert_eq!(uids, vec![101, 102, 103], "input passes through unchanged");

    let mut computed = op.take_computed();
    computed.sort_by_key(|row| row.uid);
    assert_eq!(computed.len(), 3);
    let paul = &computed[0];
    assert_eq!(paul.uid, 101);
    assert_eq!(paul.fields.get("who"), Some(&s("PAUL")));
    assert_eq!(paul.fields.get("twice"), Some(&i(80)));
    assert!(op.take_computed().is_empty(), "take consumes the stash");
}

#[test]
fn ifelse_picks_first_true_branch_or_else() {
    let fx = build_fixture();
    let cond_true = compile_arc(&lit(QueryValue::Bool(true))).unwrap();
    let cond_false = compile_arc(&lit(QueryValue::Bool(false))).unwrap();
    let yes = compile_arc(&lit(s("yes"))).unwrap();
    let no = compile_arc(&lit(s("no"))).unwrap();
    let fallback = compile_arc(&lit(s("fallback"))).unwrap();

    let hit = IfElseOperator::new(vec![(cond_false.clone(), no.clone()), (cond_true, yes)], None);
    exec_batches(&hit, &fx);
    assert_eq!(hit.take_value(), Some(s("yes")));

    let missed = IfElseOperator::new(vec![(cond_false, no)], Some(fallback));
    exec_batches(&missed, &fx);
    assert_eq!(missed.take_value(), Some(s("fallback")));

    let nothing = IfElseOperator::new(Vec::new(), None);
    exec_batches(&nothing, &fx);
    assert_eq!(nothing.take_value(), Some(QueryValue::Null));
}

fn loop_var(var: &str) -> LogicalExpr {
    fld(var)
}

#[test]
fn foreach_runs_steps_and_counts_completed_iterations() {
    let fx = build_fixture();
    // Range [1,2,3]; guard `$v ne 2` skips iteration 2 entirely.
    let guard = compile_arc(&bin(loop_var("v"), BinaryOp::Ne, lit(i(2)))).unwrap();
    let noop = compile_arc(&lit(QueryValue::Bool(true))).unwrap();
    let range = compile_arc(&lit(QueryValue::List(vec![i(1), i(2), i(3)]))).unwrap();
    let op = ForeachOperator::new(range, "v", vec![guard, noop]);
    assert_eq!(op.kind().as_str(), "control");
    assert!(exec_batches(&op, &fx).is_empty());
    assert_eq!(op.iterations(), 2, "element 2 tripped the continue-guard");

    // Without guards every element completes.
    let range = compile_arc(&lit(QueryValue::List(vec![i(1), i(2), i(3)]))).unwrap();
    let plain = ForeachOperator::new(range, "v", vec![compile_arc(&lit(QueryValue::Bool(true))).unwrap()]);
    exec_batches(&plain, &fx);
    assert_eq!(plain.iterations(), 3);
}

#[test]
fn foreach_break_when_stops_the_loop_early() {
    let fx = build_fixture();
    let stop = compile_arc(&bin(loop_var("v"), BinaryOp::Ge, lit(i(2)))).unwrap();
    let noop = compile_arc(&lit(QueryValue::Bool(true))).unwrap();
    let range = compile_arc(&lit(QueryValue::List(vec![i(1), i(2), i(3)]))).unwrap();
    let op = ForeachOperator::new(range, "v", vec![noop]).with_break_when(stop);
    exec_batches(&op, &fx);
    assert_eq!(
        op.iterations(),
        1,
        "element 1 completed; element 2 triggered break and does not count"
    );
}

#[test]
fn sequence_accumulates_until_a_step_signals() {
    let fx = build_fixture();
    let both = SequenceOperator::new(vec![
        Box::new(FullTypeScan::new("Author")),
        Box::new(FullTypeScan::new("Author")),
    ]);
    assert_eq!(both.kind().as_str(), "control");
    let flattened: Vec<u64> = exec_batches(&both, &fx).into_iter().flatten().collect();
    assert_eq!(flattened.len(), 6, "batches from both steps accumulate");

    // A returning step terminates the sequence with Break.
    let cut = SequenceOperator::new(vec![
        Box::new(ReturnOperator::new(Box::new(FullTypeScan::new("Author")))),
        Box::new(FullTypeScan::new("Author")),
    ]);
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    assert!(matches!(cut.execute(&mut ctx), FlowResult::Break));
}

#[test]
fn return_operator_buffers_payload_and_surfaces_break() {
    let fx = build_fixture();
    let op = ReturnOperator::new(Box::new(UnionSources {
        sources: vec![
            Box::new(FullTypeScan::new("Author")),
        ],
    }));
    assert_eq!(op.kind().as_str(), "control");
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    assert!(matches!(op.execute(&mut ctx), FlowResult::Break));
    let rows = op.take_rows().expect("payload buffered");
    let total: usize = rows.iter().map(|b| b.0.len()).sum();
    assert_eq!(total, 3);
    assert!(op.take_rows().is_none());
}

// EmptySource is the scalar context: every path resolves to None => Null.
#[test]
fn empty_source_resolves_nothing() {
    let src = EmptySource;
    assert_eq!(src.resolve(&FieldPath::field("anything")), None);
    let ctx = EvalContext::new(&src);
    let compiled = compile_arc(&fld("anything")).unwrap();
    assert_eq!(compiled.evaluate(&ctx).unwrap(), QueryValue::Null);
}
