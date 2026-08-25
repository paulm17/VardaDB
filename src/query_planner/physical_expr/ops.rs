//! Arithmetic, logical, comparison and containment operators.
//!
//! Number coercion mirrors legacy `check_condition`: Int/Float unify through
//! f64, string ordering tries i64 parsing before lexical order. Unlike the
//! filter bridge, mismatched operand kinds raise [`ExprError`] instead of
//! passing vacuously, and every operator except `Eq`/`Ne` propagates Null.

use crate::query_planner::ir::{BinaryOp, QueryValue, UnaryOp};
use crate::query_planner::physical_expr::{
    as_number, type_name, value_cmp, value_eq, EvalContext, ExprError, PhysicalExpr,
};

#[derive(Debug)]
pub struct BinaryExpr {
    left: Box<dyn PhysicalExpr>,
    op: BinaryOp,
    right: Box<dyn PhysicalExpr>,
}

impl BinaryExpr {
    pub fn new(left: Box<dyn PhysicalExpr>, op: BinaryOp, right: Box<dyn PhysicalExpr>) -> Self {
        BinaryExpr { left, op, right }
    }

    fn eval_op(&self, l: &QueryValue, r: &QueryValue) -> Result<QueryValue, ExprError> {
        match self.op {
            BinaryOp::Add => arith(l, r, "add", |a, b| a.checked_add(b), |a, b| a + b),
            BinaryOp::Sub => arith(l, r, "sub", |a, b| a.checked_sub(b), |a, b| a - b),
            BinaryOp::Mul => arith(l, r, "mul", |a, b| a.checked_mul(b), |a, b| a * b),
            BinaryOp::Div => match (l, r) {
                (x, y) if is_null(x) || is_null(y) => Ok(QueryValue::Null),
                (_, QueryValue::Int(0)) => Err(ExprError::DivisionByZero),
                (_, QueryValue::Float(f)) if *f == 0.0 => Err(ExprError::DivisionByZero),
                (QueryValue::Int(a), QueryValue::Int(b)) => Ok(QueryValue::Int(a / b)),
                _ => float_arith(l, r, "div", |a, b| a / b),
            },
            BinaryOp::Mod => match (l, r) {
                (x, y) if is_null(x) || is_null(y) => Ok(QueryValue::Null),
                (_, QueryValue::Int(0)) => Err(ExprError::DivisionByZero),
                (_, QueryValue::Float(f)) if *f == 0.0 => Err(ExprError::DivisionByZero),
                (QueryValue::Int(a), QueryValue::Int(b)) => Ok(QueryValue::Int(a % b)),
                _ => float_arith(l, r, "mod", |a, b| a % b),
            },
            BinaryOp::And => bool_pair(l, r, "and").map(|(a, b)| QueryValue::Bool(a && b)),
            BinaryOp::Or => bool_pair(l, r, "or").map(|(a, b)| QueryValue::Bool(a || b)),
            BinaryOp::Eq => Ok(QueryValue::Bool(value_eq(l, r))),
            BinaryOp::Ne => Ok(QueryValue::Bool(!value_eq(l, r))),
            BinaryOp::Gt => compare(l, r, std::cmp::Ordering::is_gt),
            BinaryOp::Ge => compare(l, r, std::cmp::Ordering::is_ge),
            BinaryOp::Lt => compare(l, r, std::cmp::Ordering::is_lt),
            BinaryOp::Le => compare(l, r, std::cmp::Ordering::is_le),
            BinaryOp::Contains => contains(l, r),
            BinaryOp::In => membership(l, r),
        }
    }
}

impl PhysicalExpr for BinaryExpr {
    fn evaluate(&self, ctx: &EvalContext) -> Result<QueryValue, ExprError> {
        let l = self.left.evaluate(ctx)?;
        let r = self.right.evaluate(ctx)?;
        self.eval_op(&l, &r)
    }

    fn describe(&self) -> String {
        format!("({} {:?} {})", self.left.describe(), self.op, self.right.describe())
    }
}

#[derive(Debug)]
pub struct UnaryExpr {
    op: UnaryOp,
    expr: Box<dyn PhysicalExpr>,
}

impl UnaryExpr {
    pub fn new(op: UnaryOp, expr: Box<dyn PhysicalExpr>) -> Self {
        UnaryExpr { op, expr }
    }
}

impl PhysicalExpr for UnaryExpr {
    fn evaluate(&self, ctx: &EvalContext) -> Result<QueryValue, ExprError> {
        let v = self.expr.evaluate(ctx)?;
        match self.op {
            UnaryOp::Neg => match v {
                QueryValue::Null => Ok(QueryValue::Null),
                QueryValue::Int(i) => i.checked_neg().map(QueryValue::Int).ok_or(ExprError::ArithmeticOverflow),
                QueryValue::Float(f) => Ok(QueryValue::Float(-f)),
                other => Err(ExprError::UnaryTypeMismatch {
                    op: "neg",
                    operand: type_name(&other),
                }),
            },
            UnaryOp::Not => match v {
                QueryValue::Bool(b) => Ok(QueryValue::Bool(!b)),
                other => Err(ExprError::UnaryTypeMismatch {
                    op: "not",
                    operand: type_name(&other),
                }),
            },
        }
    }

    fn describe(&self) -> String {
        format!("({:?} {})", self.op, self.expr.describe())
    }
}

// -- helpers ----------------------------------------------------------------

fn is_null(v: &QueryValue) -> bool {
    matches!(v, QueryValue::Null)
}

fn arith(
    l: &QueryValue,
    r: &QueryValue,
    op: &'static str,
    int_op: fn(i64, i64) -> Option<i64>,
    float_op: fn(f64, f64) -> f64,
) -> Result<QueryValue, ExprError> {
    if is_null(l) || is_null(r) {
        return Ok(QueryValue::Null);
    }
    if let (QueryValue::Int(a), QueryValue::Int(b)) = (l, r) {
        return int_op(*a, *b)
            .map(QueryValue::Int)
            .ok_or(ExprError::ArithmeticOverflow);
    }
    float_binary(l, r, op, float_op)
}

fn float_arith(
    l: &QueryValue,
    r: &QueryValue,
    op: &'static str,
    f: fn(f64, f64) -> f64,
) -> Result<QueryValue, ExprError> {
    float_binary(l, r, op, f)
}

fn float_binary(
    l: &QueryValue,
    r: &QueryValue,
    op: &'static str,
    f: fn(f64, f64) -> f64,
) -> Result<QueryValue, ExprError> {
    match (as_number(l), as_number(r)) {
        (Some(a), Some(b)) => Ok(QueryValue::Float(f(a, b))),
        (None, _) => Err(type_mismatch(op, l)),
        (_, None) => Err(type_mismatch(op, r)),
    }
}

fn bool_pair(
    l: &QueryValue,
    r: &QueryValue,
    op: &'static str,
) -> Result<(bool, bool), ExprError> {
    match (l, r) {
        (QueryValue::Bool(a), QueryValue::Bool(b)) => Ok((*a, *b)),
        (x, _) if !matches!(x, QueryValue::Bool(_)) => Err(type_mismatch(op, x)),
        (_, y) => Err(type_mismatch(op, y)),
    }
}

fn compare(
    l: &QueryValue,
    r: &QueryValue,
    keep: fn(std::cmp::Ordering) -> bool,
) -> Result<QueryValue, ExprError> {
    if is_null(l) || is_null(r) {
        return Ok(QueryValue::Null);
    }
    value_cmp(l, r).map(|ord| QueryValue::Bool(keep(ord)))
}

/// `left contains right`: substring for strings (case-sensitive, legacy
/// parity), element membership for lists.
fn contains(l: &QueryValue, r: &QueryValue) -> Result<QueryValue, ExprError> {
    match (l, r) {
        (QueryValue::String(haystack), QueryValue::String(needle)) => {
            Ok(QueryValue::Bool(haystack.contains(needle.as_str())))
        }
        (QueryValue::List(items), needle) => {
            Ok(QueryValue::Bool(items.iter().any(|item| value_eq(item, needle))))
        }
        (x, _) if is_null(x) || is_null(r) => Ok(QueryValue::Null),
        (x, _) => Err(ExprError::TypeMismatch {
            op: "contains",
            left: type_name(x),
            right: type_name(r),
        }),
    }
}

/// `left in right` where right must be a list.
fn membership(l: &QueryValue, r: &QueryValue) -> Result<QueryValue, ExprError> {
    match r {
        QueryValue::List(items) => {
            if is_null(l) {
                return Ok(QueryValue::Null);
            }
            Ok(QueryValue::Bool(items.iter().any(|item| value_eq(item, l))))
        }
        other => Err(ExprError::TypeMismatch {
            op: "in",
            left: type_name(l),
            right: type_name(other),
        }),
    }
}

fn type_mismatch(op: &'static str, v: &QueryValue) -> ExprError {
    ExprError::TypeMismatch {
        op,
        left: type_name(v),
        right: "?",
    }
}
