//! Stage 3.1a expression-runtime tests: pure evaluation semantics plus
//! stored-field integration through the Phase-1 `PlannerFieldEval` bridge.

use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata};
use vardadb::query_planner::ir::{
    BinaryOp, EntityId, FieldPath, LogicalExpr, QueryRecord, QueryValue, UnaryOp,
};
use vardadb::query_planner::physical_expr::{
    compile, eval_record, EvalContext, ExprError, FieldExpr, RecordSource, StoredSource,
};
use vardadb::query_planner::physical_expr::PhysicalExpr;
use vardadb::query_planner::runtime_for;
use vardadb::realtime::bus::MutationSource;
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

// -- helpers ------------------------------------------------------------------

fn lit(v: QueryValue) -> LogicalExpr {
    LogicalExpr::Value(v)
}

fn i(v: i64) -> QueryValue {
    QueryValue::Int(v)
}

fn f(v: f64) -> QueryValue {
    QueryValue::Float(v)
}

fn s(v: &str) -> QueryValue {
    QueryValue::String(v.to_string())
}

fn bin(left: LogicalExpr, op: BinaryOp, right: LogicalExpr) -> LogicalExpr {
    LogicalExpr::Binary {
        left: Box::new(left),
        op,
        right: Box::new(right),
    }
}

fn record(fields: &[(&str, QueryValue)]) -> QueryRecord {
    QueryRecord {
        id: EntityId::new(1),
        fields: fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    }
}

fn eval_over(expr: &LogicalExpr, rec: &QueryRecord) -> Result<QueryValue, ExprError> {
    let compiled = compile(expr).expect("compile");
    eval_record(compiled.as_ref(), rec)
}

// -- literals & arithmetic ------------------------------------------------------

#[test]
fn literals_evaluate_to_themselves() {
    let rec = record(&[]);
    assert_eq!(eval_over(&lit(i(7)), &rec).unwrap(), i(7));
    assert_eq!(eval_over(&lit(s("hi")), &rec).unwrap(), s("hi"));
    assert_eq!(eval_over(&lit(QueryValue::Null), &rec).unwrap(), QueryValue::Null);
}

#[test]
fn int_arithmetic_stays_int_and_checks_overflow() {
    let rec = record(&[]);
    let add = |l, r| eval_over(&bin(lit(l), BinaryOp::Add, lit(r)), &rec).unwrap();
    assert_eq!(add(i(2), i(3)), i(5));
    assert_eq!(
        eval_over(&bin(lit(i(6)), BinaryOp::Div, lit(i(4))), &rec).unwrap(),
        i(1),
        "int division truncates"
    );
    assert_eq!(
        eval_over(&bin(lit(i(6)), BinaryOp::Mod, lit(i(4))), &rec).unwrap(),
        i(2)
    );
    assert_eq!(
        eval_over(
            &bin(lit(i(i64::MAX)), BinaryOp::Add, lit(i(1))),
            &rec
        ),
        Err(ExprError::ArithmeticOverflow)
    );
}

#[test]
fn mixed_numeric_arithmetic_promotes_to_float() {
    let rec = record(&[]);
    let out = eval_over(&bin(lit(i(2)), BinaryOp::Add, lit(f(0.5))), &rec).unwrap();
    assert_eq!(out, f(2.5));
    // Div by zero is an error in both int and float space.
    assert_eq!(
        eval_over(&bin(lit(i(1)), BinaryOp::Div, lit(i(0))), &rec),
        Err(ExprError::DivisionByZero)
    );
    assert_eq!(
        eval_over(&bin(lit(f(1.0)), BinaryOp::Mod, lit(f(0.0))), &rec),
        Err(ExprError::DivisionByZero)
    );
}

#[test]
fn arithmetic_propagates_null() {
    let rec = record(&[]);
    assert_eq!(
        eval_over(&bin(lit(QueryValue::Null), BinaryOp::Mul, lit(i(3))), &rec).unwrap(),
        QueryValue::Null
    );
}

// -- comparisons ----------------------------------------------------------------

#[test]
fn equality_unifies_numeric_types_and_treats_null_as_value() {
    let rec = record(&[]);
    let eq = |l, r| eval_over(&bin(lit(l), BinaryOp::Eq, lit(r)), &rec).unwrap();
    let ne = |l, r| eval_over(&bin(lit(l), BinaryOp::Ne, lit(r)), &rec).unwrap();
    assert_eq!(eq(i(5), f(5.0)), QueryValue::Bool(true));
    assert_eq!(eq(s("a"), QueryValue::Enum("a".into())), QueryValue::Bool(true));
    assert_eq!(eq(QueryValue::Null, QueryValue::Null), QueryValue::Bool(true));
    assert_eq!(ne(QueryValue::Null, i(1)), QueryValue::Bool(true));
}

#[test]
fn ordering_matches_legacy_number_and_string_semantics() {
    let rec = record(&[]);
    let gt = |l, r| eval_over(&bin(lit(l), BinaryOp::Gt, lit(r)), &rec).unwrap();
    // Numeric cross-type via f64.
    assert_eq!(gt(f(2.5), i(2)), QueryValue::Bool(true));
    // Strings: i64 parse precedence beats lexical order ("10" > "9").
    assert_eq!(gt(s("10"), s("9")), QueryValue::Bool(true));
    // Non-parseable strings fall back to lexical.
    assert_eq!(gt(s("b"), s("aa")), QueryValue::Bool(true));
    // Ordering propagates Null.
    assert_eq!(
        eval_over(&bin(lit(QueryValue::Null), BinaryOp::Le, lit(i(1))), &rec).unwrap(),
        QueryValue::Null
    );
    // Bool vs Int ordering is a type error, not vacuous truth.
    assert!(matches!(
        eval_over(&bin(lit(QueryValue::Bool(true)), BinaryOp::Lt, lit(i(1))), &rec),
        Err(ExprError::TypeMismatch { .. })
    ));
}

// -- logical / containment ------------------------------------------------------

#[test]
fn logical_ops_require_bools() {
    let rec = record(&[]);
    let and = bin(lit(QueryValue::Bool(true)), BinaryOp::And, lit(QueryValue::Bool(false)));
    assert_eq!(eval_over(&and, &rec).unwrap(), QueryValue::Bool(false));
    let bad = bin(lit(i(1)), BinaryOp::And, lit(QueryValue::Bool(true)));
    assert!(matches!(
        eval_over(&bad, &rec),
        Err(ExprError::TypeMismatch { .. })
    ));
}

#[test]
fn contains_is_case_sensitive_substring_or_list_membership() {
    let rec = record(&[]);
    let c = |l, r| eval_over(&bin(lit(l), BinaryOp::Contains, lit(r)), &rec).unwrap();
    assert_eq!(c(s("Intro to Rust"), s("Rust")), QueryValue::Bool(true));
    assert_eq!(c(s("rust"), s("Rust")), QueryValue::Bool(false));
    assert_eq!(
        c(
            QueryValue::List(vec![i(1), i(2)]),
            i(2)
        ),
        QueryValue::Bool(true)
    );
    assert!(matches!(
        eval_over(&bin(lit(i(1)), BinaryOp::Contains, lit(s("x"))), &rec),
        Err(ExprError::TypeMismatch { .. })
    ));
}

#[test]
fn membership_requires_list_on_right() {
    let rec = record(&[]);
    let inside = bin(
        lit(s("go")),
        BinaryOp::In,
        lit(QueryValue::List(vec![s("go"), s("rust")])),
    );
    assert_eq!(eval_over(&inside, &rec).unwrap(), QueryValue::Bool(true));
    let not_a_list = bin(lit(s("go")), BinaryOp::In, lit(s("golang")));
    assert!(matches!(
        eval_over(&not_a_list, &rec),
        Err(ExprError::TypeMismatch { .. })
    ));
}

// -- unary ----------------------------------------------------------------------

#[test]
fn unary_neg_and_not() {
    let rec = record(&[]);
    let neg = LogicalExpr::Unary {
        op: UnaryOp::Neg,
        expr: Box::new(lit(i(4))),
    };
    assert_eq!(eval_over(&neg, &rec).unwrap(), i(-4));
    let not = LogicalExpr::Unary {
        op: UnaryOp::Not,
        expr: Box::new(lit(QueryValue::Bool(false))),
    };
    assert_eq!(eval_over(&not, &rec).unwrap(), QueryValue::Bool(true));
    let bad_neg = LogicalExpr::Unary {
        op: UnaryOp::Neg,
        expr: Box::new(lit(s("x"))),
    };
    assert!(matches!(
        eval_over(&bad_neg, &rec),
        Err(ExprError::UnaryTypeMismatch { .. })
    ));
}

// -- field resolution -------------------------------------------------------------

#[test]
fn field_paths_walk_objects_lists_and_default_to_null() {
    use vardadb::query_planner::ir::FieldSegment;
    let mut inner = std::collections::BTreeMap::new();
    inner.insert("deep".to_string(), i(42));
    let rec = record(&[
        ("age", i(40)),
        (
            "meta",
            QueryValue::Object(std::collections::BTreeMap::from([(
                "nested".to_string(),
                QueryValue::Object(inner),
            )])),
        ),
        ("tags", QueryValue::List(vec![s("a"), s("b")])),
    ]);
    let src = RecordSource::new(&rec);
    let ctx = EvalContext::new(&src);

    let age = FieldExpr::new(FieldPath::field("age"));
    assert_eq!(age.evaluate(&ctx).unwrap(), i(40));

    let deep = FieldExpr::new(FieldPath {
        segments: vec![
            FieldSegment::Field("meta".into()),
            FieldSegment::Field("nested".into()),
            FieldSegment::Field("deep".into()),
        ],
    });
    assert_eq!(deep.evaluate(&ctx).unwrap(), i(42));

    let idx_expr = FieldExpr::new(FieldPath {
        segments: vec![FieldSegment::Field("tags".into()), FieldSegment::Index(1)],
    });
    assert_eq!(idx_expr.evaluate(&ctx).unwrap(), s("b"));

    // Out-of-range index and missing fields both resolve to Null.
    let oob = FieldExpr::new(FieldPath {
        segments: vec![FieldSegment::Field("tags".into()), FieldSegment::Index(9)],
    });
    assert_eq!(oob.evaluate(&ctx).unwrap(), QueryValue::Null);
    let missing = FieldExpr::new(FieldPath::field("nope"));
    assert_eq!(missing.evaluate(&ctx).unwrap(), QueryValue::Null);
}

// -- stored-source integration -----------------------------------------------------

struct Fixture {
    _dir: tempfile::TempDir,
    resolver: SqliteResolver,
    metadata: HashMap<String, QueryTypeMetadata>,
    paul: u64,
}

fn build_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    for (uid, name, age) in [(101u64, "Paul", 40i64), (102, "Ada", 36)] {
        resolver
            .create_node_internal(
                "Author",
                uid,
                vec_str_map(&[
                    ("name", serde_json::json!(name)),
                    ("age", serde_json::json!(age)),
                ]),
                &["name".to_string()],
                &[],
                &HashMap::new(),
                MutationSource::Local,
                Some(Timestamp::new(1_700_000_000_000, uid as u16, 1)),
            )
            .unwrap();
    }

    let book_inverses = [InverseInfo {
        field: "author".to_string(),
        inverse_type: "Author".to_string(),
        inverse_field: "books".to_string(),
        inverse_is_list: true,
    }];
    resolver
        .create_node_internal(
            "Book",
            201,
            vec_str_map(&[
                ("title", serde_json::json!("Planner Internals")),
                ("author", serde_json::json!("101")),
            ]),
            &[],
            &book_inverses,
            &HashMap::new(),
            MutationSource::Local,
            Some(Timestamp::new(1_700_000_000_000, 201, 1)),
        )
        .unwrap();

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
    }
}

fn vec_str_map(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn stored_source_evaluates_fields_through_planner_bridge() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let src = StoredSource::new(&rt, EntityId::new(fx.paul));
    let ctx = EvalContext::new(&src);

    let age = FieldExpr::new(FieldPath::field("age"));
    assert_eq!(age.evaluate(&ctx).unwrap(), i(40));

    // Computed predicate: age + 2 > 41 => true for Paul (40+2=42).
    let expr = compile(&bin(
        bin(
            LogicalExpr::Field(FieldPath::field("age")),
            BinaryOp::Add,
            lit(i(2)),
        ),
        BinaryOp::Gt,
        lit(i(41)),
    ))
    .unwrap();
    assert_eq!(expr.evaluate(&ctx).unwrap(), QueryValue::Bool(true));

    // Relation edge-fallback parity: `books` resolves through the inverse
    // edge exactly like FilterOperator's relation arm does today.
    let books = FieldExpr::new(FieldPath::field("books"));
    match books.evaluate(&ctx).unwrap() {
        QueryValue::List(items) => {
            assert_eq!(items.len(), 1, "Paul authored one book: {items:?}");
        }
        other => panic!("expected list of related uids, got {other:?}"),
    }

    // Missing field evaluates to Null rather than failing.
    let nope = FieldExpr::new(FieldPath::field("nickname"));
    assert_eq!(nope.evaluate(&ctx).unwrap(), QueryValue::Null);
}

// -- compile() surface -----------------------------------------------------------

#[test]
fn compile_rejects_function_and_subquery_until_later_stages() {
    let function = LogicalExpr::Function {
        name: "lower".to_string(),
        args: vec![lit(s("X"))],
    };
    assert!(matches!(
        compile(&function),
        Err(ExprError::UnknownFunction(_))
    ));
    let subquery = LogicalExpr::Subquery(Box::new(vardadb::query_planner::ir::LogicalQuery::scan(
        "Author",
    )));
    assert!(matches!(compile(&subquery), Err(ExprError::UnsupportedSubquery)));
}
