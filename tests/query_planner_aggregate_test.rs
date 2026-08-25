//! Stage 3.2a tests: builtin aggregate functions (`count`, `math::sum`,
//! `math::mean`, `math::min`, `math::max`) and the hash-grouped aggregation
//! operator.

use std::sync::Arc;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::QueryTypeMetadata;
use vardadb::query_planner::function::default_aggregate_registry;
use vardadb::query_planner::ir::{
    BinaryOp, EntityId, FieldPath, LogicalExpr, QueryValue,
};
use vardadb::query_planner::operators::{
    AggregateSpec, ExecContext, ExecOperator, FlowResult, FullTypeScan, HashAggregateOperator,
};
use vardadb::query_planner::physical_expr::compile;
use vardadb::query_planner::runtime_for;
use vardadb::realtime::bus::MutationSource;
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

fn ts(counter: u16) -> Timestamp {
    Timestamp::new(1_700_000_000_000, counter, 1)
}

fn s(v: &str) -> QueryValue {
    QueryValue::String(v.to_string())
}

fn i(v: i64) -> QueryValue {
    QueryValue::Int(v)
}

fn f(v: f64) -> QueryValue {
    QueryValue::Float(v)
}

fn lit(v: QueryValue) -> LogicalExpr {
    LogicalExpr::Value(v)
}

fn fld(path: &str) -> LogicalExpr {
    LogicalExpr::Field(FieldPath::field(path))
}

struct Fixture {
    _dir: tempfile::TempDir,
    resolver: SqliteResolver,
    metadata: std::collections::HashMap<String, QueryTypeMetadata>,
}

/// Authors Paul(40) / Ada(36) / Bob(25); `name` unique.
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
                &std::collections::HashMap::new(),
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
            relations: std::collections::HashMap::new(),
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

fn spec(func_name: &str, arg: Option<LogicalExpr>, alias: &str) -> AggregateSpec {
    let func = default_aggregate_registry()
        .get(func_name)
        .unwrap_or_else(|| panic!("{func_name} registered"));
    AggregateSpec {
        func,
        arg: arg.map(|expr| Arc::from(compile(&expr).unwrap())),
        alias: alias.to_string(),
    }
}

/// Run an aggregate over every Author row and return the materialized groups.
fn aggregate_authors(
    fx: &Fixture,
    specs: Vec<AggregateSpec>,
    group_by: Vec<LogicalExpr>,
) -> Result<
    (
        Vec<(Vec<QueryValue>, Vec<(String, QueryValue)>)>,
        usize,
    ),
    (),
> {
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new_with_explain(&rt, "default", true);
    let input = Box::new(FullTypeScan::new("Author")) as Box<dyn ExecOperator>;
    let group_exprs: Vec<_> = group_by
        .iter()
        .map(|e| -> Arc<dyn vardadb::query_planner::physical_expr::PhysicalExpr> {
            Arc::from(compile(e).unwrap())
        })
        .collect();
    let agg = HashAggregateOperator::new(input, specs, group_exprs);
    match agg.execute(&mut ctx) {
        FlowResult::Rows(_) => {
            let rows = agg
                .take_groups()
                .into_iter()
                .map(|g| (g.key, g.outputs))
                .collect();
            Ok((rows, ctx.explain.take_stats().len()))
        }
        FlowResult::Error(_) => Err(()),
        other => panic!("unexpected flow: error={}", other.is_error()),
    }
}

fn output<'a>(
    outputs: &'a [(String, QueryValue)],
    alias: &str,
) -> &'a QueryValue {
    &outputs.iter().find(|(a, _)| a == alias).unwrap().1
}

// -- accumulators ------------------------------------------------------------

#[test]
fn count_skips_nulls_and_merges() {
    let func = default_aggregate_registry().get("count").unwrap();
    let mut acc = func.create_accumulator();
    acc.update(&i(1)).unwrap();
    acc.update(&QueryValue::Null).unwrap();
    acc.update(&s("x")).unwrap();
    assert_eq!(acc.finalize().unwrap(), i(2));

    let mut left = func.create_accumulator();
    left.update(&i(1)).unwrap();
    let mut right = func.create_accumulator();
    right.update(&i(1)).unwrap();
    right.update(&i(1)).unwrap();
    left.merge(right).unwrap();
    assert_eq!(left.finalize().unwrap(), i(3));
}

#[test]
fn sum_stays_int_promotes_float_and_errors_on_overflow_or_type() {
    let func = default_aggregate_registry().get("math::sum").unwrap();

    let mut acc = func.create_accumulator();
    for v in [i(10), i(32)] {
        acc.update(&v).unwrap();
    }
    assert_eq!(acc.finalize().unwrap(), i(42));

    let mut mixed = func.create_accumulator();
    mixed.update(&i(1)).unwrap();
    mixed.update(&f(0.5)).unwrap();
    assert_eq!(mixed.finalize().unwrap(), f(1.5));

    // No values seen => Null.
    assert_eq!(func.create_accumulator().finalize().unwrap(), QueryValue::Null);

    let mut overflow = func.create_accumulator();
    assert!(matches!(
        overflow.update(&i(i64::MAX)).and_then(|_| overflow.update(&i(1))),
        Err(vardadb::query_planner::physical_expr::ExprError::ArithmeticOverflow)
    ));

    let mut bad = func.create_accumulator();
    assert!(bad.update(&s("x")).is_err());
}

#[test]
fn mean_divides_over_non_null_values_only() {
    let func = default_aggregate_registry().get("math::mean").unwrap();
    let mut acc = func.create_accumulator();
    acc.update(&QueryValue::Null).unwrap();
    acc.update(&i(10)).unwrap();
    acc.update(&f(5.0)).unwrap();
    assert_eq!(acc.finalize().unwrap(), f(7.5));
    assert_eq!(func.create_accumulator().finalize().unwrap(), QueryValue::Null);
    let mut bad = func.create_accumulator();
    assert!(bad.update(&QueryValue::Bool(true)).is_err());
}

#[test]
fn min_max_use_legacy_ordering_across_kinds() {
    let min = default_aggregate_registry().get("math::min").unwrap();
    let max = default_aggregate_registry().get("math::max").unwrap();

    let mut lo = min.create_accumulator();
    for v in [i(9), i(2)] {
        lo.update(&v).unwrap();
    }
    assert_eq!(lo.finalize().unwrap(), i(2));

    // Numeric strings order through i64-parse precedence ("9" < "10").
    let mut strlo = min.create_accumulator();
    for v in [s("10"), s("9")] {
        strlo.update(&v).unwrap();
    }
    assert_eq!(strlo.finalize().unwrap(), s("9"));

    let mut hi = max.create_accumulator();
    for v in [s("apple"), s("banana")] {
        hi.update(&v).unwrap();
    }
    assert_eq!(hi.finalize().unwrap(), s("banana"));

    // Empty extremum is Null; merge keeps the extreme of both sides.
    let mut left = max.create_accumulator();
    left.update(&i(1)).unwrap();
    let mut right = max.create_accumulator();
    right.update(&i(99)).unwrap();
    left.merge(right).unwrap();
    assert_eq!(left.finalize().unwrap(), i(99));
}

#[test]
fn merge_equals_sequential_for_every_builtin() {
    let registry = default_aggregate_registry();
    for name in ["count", "math::sum", "math::mean", "math::min", "math::max"] {
        let func = registry.get(name).unwrap();
        let values = [i(4), QueryValue::Null, i(-2), f(6.5), s("skip-me")];
        // Only feed values valid for the function; sum/mean reject strings.
        // Extremums compare within one value family only (legacy semantics),
        // so they get a numeric-only feed; count tolerates everything.
        let feed: Vec<QueryValue> = match name {
            "count" => values.to_vec(),
            "math::min" | "math::max" => vec![i(4), QueryValue::Null, i(-2)],
            _ => vec![i(4), QueryValue::Null, i(-2), f(6.5)],
        };

        let mut whole = func.create_accumulator();
        for v in &feed {
            whole.update(v).unwrap();
        }

        let (a_vals, b_vals) = feed.split_at(feed.len() / 2);
        let mut left = func.create_accumulator();
        for v in a_vals {
            left.update(v).unwrap();
        }
        let mut right = func.create_accumulator();
        for v in b_vals {
            right.update(v).unwrap();
        }
        left.merge(right).unwrap();

        assert_eq!(
            left.finalize().unwrap(),
            whole.finalize().unwrap(),
            "{name}: merged != sequential"
        );
    }
}

#[test]
fn reset_and_clone_box_behave() {
    let func = default_aggregate_registry().get("math::sum").unwrap();
    let mut acc = func.create_accumulator();
    acc.update(&i(7)).unwrap();
    let snapshot = acc.clone_box();
    assert_eq!(snapshot.finalize().unwrap(), i(7));
    acc.reset();
    assert_eq!(acc.finalize().unwrap(), QueryValue::Null);
    acc.update(&i(3)).unwrap();
    // Snapshot unaffected by later mutation.
    assert_eq!(snapshot.finalize().unwrap(), i(7));
}

#[test]
fn aggregate_registry_shape() {
    let registry = default_aggregate_registry();
    assert_eq!(registry.len(), 5);
    for name in ["count", "math::sum", "math::mean", "math::min", "math::max"] {
        assert!(registry.contains(name), "{name}");
    }
    assert!(!registry.contains("COUNT"));
    assert!(registry.get("nope").is_none());
}

// -- operator ----------------------------------------------------------------

#[test]
fn global_aggregation_computes_all_specs() {
    let fx = build_fixture();
    let (rows, stats) = aggregate_authors(
        &fx,
        vec![
            spec("count", Some(lit(i(1))), "n"),
            spec("math::sum", Some(fld("age")), "total"),
            spec("math::mean", Some(fld("age")), "avg"),
            spec("math::min", Some(fld("age")), "lo"),
            spec("math::max", Some(fld("age")), "hi"),
        ],
        vec![],
    )
    .unwrap();
    assert_eq!(stats, 2, "scan + aggregate stats recorded");
    assert_eq!(rows.len(), 1, "single global group");
    let (_, outputs) = &rows[0];
    assert_eq!(outputs.len(), 5);
    assert_eq!(output(&outputs, "n"), &i(3));
    assert_eq!(output(&outputs, "total"), &i(101));
    assert_eq!(output(&outputs, "avg"), &f(101.0 / 3.0));
    assert_eq!(output(&outputs, "lo"), &i(25));
    assert_eq!(output(&outputs, "hi"), &i(40));
}

#[test]
fn grouped_aggregation_orders_rows_by_key() {
    let fx = build_fixture();
    let (rows, _) = aggregate_authors(
        &fx,
        vec![
            spec("count", Some(lit(i(1))), "n"),
            spec("math::sum", Some(fld("age")), "total"),
        ],
        vec![fld("name")],
    )
    .unwrap();
    let keys: Vec<QueryValue> = rows.iter().map(|(k, _)| k[0].clone()).collect();
    assert_eq!(
        keys,
        vec![s("Ada"), s("Bob"), s("Paul")],
        "group rows sorted by canonical key"
    );
    assert_eq!(output(&rows[0].1, "n"), &i(1));
    assert_eq!(output(&rows[2].1, "total"), &i(40));
}

#[test]
fn null_group_keys_share_one_group() {
    let fx = build_fixture();
    let (rows, _) = aggregate_authors(
        &fx,
        vec![spec("count", Some(lit(i(1))), "n")],
        vec![fld("missing_field")],
    )
    .unwrap();
    assert_eq!(rows.len(), 1, "all rows land in the Null-key group");
    assert_eq!(rows[0].0[0], QueryValue::Null);
    assert_eq!(output(&rows[0].1, "n"), &i(3));
}

#[test]
fn type_mismatch_inside_aggregate_is_pipeline_error() {
    let fx = build_fixture();
    assert!(
        aggregate_authors(
            &fx,
            vec![spec("math::sum", Some(fld("name")), "bad")],
            vec![]
        )
        .is_err(),
        "sum over strings must fail the pipeline"
    );
}

#[test]
fn first_count_and_detail_helpers() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new_with_explain(&rt, "default", true);
    let agg = HashAggregateOperator::new(
        Box::new(FullTypeScan::new("Author")),
        vec![spec("count", Some(lit(i(1))), "rows")],
        vec![],
    );
    assert!(matches!(agg.execute(&mut ctx), FlowResult::Rows(_)));
    assert_eq!(agg.first_count(), Some(3));
    let detail = agg.detail();
    assert!(detail.starts_with("hash_aggregate"), "{detail}");
    assert!(detail.contains("count(Int(1)) AS rows"), "{detail}");
    assert_eq!(agg.children().len(), 1);
}

#[test]
fn count_constant_arg_counts_rows_not_values() {
    let fx = build_fixture();
    // Constant-literal shape (Int(1)) is what the planner emits for row counts.
    let (rows, _) = aggregate_authors(
        &fx,
        vec![spec("count", Some(lit(i(1))), "n")],
        vec![],
    )
    .unwrap();
    assert_eq!(output(&rows[0].1, "n"), &i(3));

    let (null_rows, _) = aggregate_authors(
        &fx,
        vec![spec("count", Some(bin_missing()), "n")],
        vec![],
    )
    .unwrap();
    assert_eq!(output(&null_rows[0].1, "n"), &i(0));
}

fn bin_missing() -> LogicalExpr {
    LogicalExpr::Binary {
        left: Box::new(fld("missing")),
        op: BinaryOp::Add,
        right: Box::new(lit(i(1))),
    }
}

#[test]
fn entity_ids_flow_through_batch_consumption() {
    // Sanity: operator consumes uid batches (rows_in accounting) without
    // leaking them downstream -- execute returns empty batch list.
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    let agg = HashAggregateOperator::boxed(
        Box::new(FullTypeScan::new("Author")),
        vec![spec("count", Some(lit(i(1))), "n")],
        vec![],
    );
    match agg.execute(&mut ctx) {
        FlowResult::Rows(batches) => assert!(batches.is_empty()),
        other => panic!("expected rows, got error={}", other.is_error()),
    }
    assert_eq!(EntityId::new(1).uid, 1);
}
