//! M-D expression-parser tests: textual syntax -> planner `LogicalExpr` IR.

use vardadb::query_planner::ir::{BinaryOp, FieldPath, FieldSegment, LogicalExpr, QueryValue, UnaryOp};
use vardadb::query_planner::parse_expression;
use vardadb::query_planner::parser::root_fields;

fn i(v: i64) -> LogicalExpr {
    LogicalExpr::Value(QueryValue::Int(v))
}
fn f(v: f64) -> LogicalExpr {
    LogicalExpr::Value(QueryValue::Float(v))
}
fn s(v: &str) -> LogicalExpr {
    LogicalExpr::Value(QueryValue::String(v.to_string()))
}
fn fld(path: &str) -> LogicalExpr {
    let segments = path
        .split('.')
        .map(|seg| FieldSegment::Field(seg.to_string()))
        .collect();
    LogicalExpr::Field(FieldPath { segments })
}
fn bin(left: LogicalExpr, op: BinaryOp, right: LogicalExpr) -> LogicalExpr {
    LogicalExpr::Binary { left: Box::new(left), op, right: Box::new(right) }
}
fn un(op: UnaryOp, expr: LogicalExpr) -> LogicalExpr {
    LogicalExpr::Unary { op, expr: Box::new(expr) }
}

/// LogicalExpr intentionally lacks PartialEq (Subquery wraps LogicalQuery);
/// structural equality via Debug render is exact for parser-produced trees.
fn assert_expr(src: &str, got: LogicalExpr, want: LogicalExpr) {
    assert_eq!(
        format!("{got:?}"),
        format!("{want:?}"),
        "parsed tree mismatch for {src:?}"
    );
}

fn parsed(src: &str) -> LogicalExpr {
    parse_expression(src).unwrap_or_else(|e| panic!("{src}: {e}"))
}

#[test]
fn literals_int_float_and_strings() {
    assert_expr("42", parsed("42"), i(42));
    assert_expr("2.5", parsed("2.5"), f(2.5));
    assert_expr("\"rust\"", parsed("\"rust\""), s("rust"));
    assert_expr("'go'", parsed("'go'"), s("go"));
    assert_expr("\"line\\nbreak\"", parsed("\"line\\nbreak\""), s("line\nbreak"));
}

#[test]
fn arithmetic_precedence_mul_over_add() {
    // 1 + 2 * 3 => Add(1, Mul(2, 3))
    assert_expr(
        "1 + 2 * 3",
        parsed("1 + 2 * 3"),
        bin(i(1), BinaryOp::Add, bin(i(2), BinaryOp::Mul, i(3))),
    );
    // Parentheses override
    assert_expr(
        "(1 + 2) * 3",
        parsed("(1 + 2) * 3"),
        bin(bin(i(1), BinaryOp::Add, i(2)), BinaryOp::Mul, i(3)),
    );
}

#[test]
fn comparison_binds_looser_than_addition() {
    // age + 10 > 40 => Gt(Add(age,10), 40)
    assert_expr(
        "age + 10 > 40",
        parsed("age + 10 > 40"),
        bin(bin(fld("age"), BinaryOp::Add, i(10)), BinaryOp::Gt, i(40)),
    );
}

#[test]
fn logical_precedence_or_lowest() {
    // a or b and c => Or(a, And(b, c))
    assert_expr(
        "a or b and c",
        parsed("a or b and c"),
        bin(fld("a"), BinaryOp::Or, bin(fld("b"), BinaryOp::And, fld("c"))),
    );
}

#[test]
fn all_comparison_operators_map() {
    let cases = [
        ("a == b", BinaryOp::Eq),
        ("a != b", BinaryOp::Ne),
        ("a >= b", BinaryOp::Ge),
        ("a <= b", BinaryOp::Le),
        ("a > b", BinaryOp::Gt),
        ("a < b", BinaryOp::Lt),
        ("a in b", BinaryOp::In),
        ("a contains b", BinaryOp::Contains),
    ];
    for (src, op) in cases {
        assert_expr(src, parsed(src), bin(fld("a"), op, fld("b")));
    }
}

#[test]
fn unary_not_and_negate() {
    assert_expr("not ok", parsed("not ok"), un(UnaryOp::Not, fld("ok")));
    assert_expr("-price", parsed("-price"), un(UnaryOp::Neg, fld("price")));
    // Precedence: -age > 10 => Gt(Neg(age), 10)
    assert_expr(
        "-age > 10",
        parsed("-age > 10"),
        bin(un(UnaryOp::Neg, fld("age")), BinaryOp::Gt, i(10)),
    );
}

#[test]
fn function_calls_including_namespaced() {
    let upper = LogicalExpr::Function {
        name: "upper".to_string(),
        args: vec![fld("name")],
    };
    assert_expr("upper(name)", parsed("upper(name)"), upper);

    let namespaced = LogicalExpr::Function {
        name: "math::sum".to_string(),
        args: vec![fld("price")],
    };
    assert_expr("math::sum(price)", parsed("math::sum(price)"), namespaced);

    let concat = LogicalExpr::Function {
        name: "concat".to_string(),
        args: vec![fld("first"), s(" "), fld("last")],
    };
    assert_expr(
        "concat(first, \" \", last)",
        parsed("concat(first, \" \", last)"),
        concat,
    );
}

#[test]
fn dotted_and_indexed_paths() {
    // profile.age
    assert_expr("profile.age", parsed("profile.age"), fld("profile.age"));

    // tags[0]
    let tags0 = LogicalExpr::Field(FieldPath {
        segments: vec![
            FieldSegment::Field("tags".to_string()),
            FieldSegment::Index(0),
        ],
    });
    assert_expr("tags[0]", parsed("tags[0]"), tags0);

    // a[1].b[2]
    let mixed = LogicalExpr::Field(FieldPath {
        segments: vec![
            FieldSegment::Field("a".to_string()),
            FieldSegment::Index(1),
            FieldSegment::Field("b".to_string()),
            FieldSegment::Index(2),
        ],
    });
    assert_expr("a[1].b[2]", parsed("a[1].b[2]"), mixed);
}

#[test]
fn real_world_predicate_shapes() {
    // The motivating example from the follow-up proposal.
    assert_expr(
        "age + 10 > 40",
        parsed("age + 10 > 40"),
        bin(bin(fld("age"), BinaryOp::Add, i(10)), BinaryOp::Gt, i(40)),
    );

    // upper(name) == "PAUL"
    let expected = bin(
        LogicalExpr::Function {
            name: "upper".to_string(),
            args: vec![fld("name")],
        },
        BinaryOp::Eq,
        s("PAUL"),
    );
    assert_expr("upper(name) == \"PAUL\"", parsed("upper(name) == \"PAUL\""), expected);
}

#[test]
fn parse_errors_report_positions() {
    for src in ["age +", "(1", "\"unterminated", "a @ b", "1 2"] {
        let err = parse_expression(src).expect_err(src);
        assert!(!err.message.is_empty(), "{src}");
    }
}

#[test]
fn root_fields_collects_distinct_roots() {
    // Function names are not field roots; only Field segments count.
    let expr = parse_expression("(age + len(nick)) > 40 or age < 20").unwrap();
    let mut roots = Vec::new();
    root_fields(&expr, &mut roots);
    assert_eq!(roots, vec!["age".to_string(), "nick".to_string()]);

    let expr = parse_expression("profile.city == \"Oslo\"").unwrap();
    let mut roots = Vec::new();
    root_fields(&expr, &mut roots);
    assert_eq!(roots, vec!["profile".to_string()]);
}
