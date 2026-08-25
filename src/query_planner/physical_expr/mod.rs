//! Stage 3.1 expression runtime: compiled [`PhysicalExpr`] trees evaluated
//! synchronously against row data.
//!
//! Ported from the upstream `exec/physical_expr` design with two deliberate
//! deviations:
//!
//! 1. **Sync evaluation.** Upstream returns `BoxFut<FlowResult<Value>>`
//!    because script/I-O functions need async. Every VardaDB function is
//!    pure and synchronous, so evaluation is
//!    `fn evaluate(&EvalContext) -> Result<QueryValue, ExprError>` matching
//!    the sync operator stack it plugs into.
//! 2. **Strict typing.** Filter *conditions* (the legacy `check_condition`
//!    bridge) pass vacuously on type mismatches because they guard against
//!    schema drift in stored rows. Expressions are explicit computations, so
//!    operand mismatches surface as [`ExprError`] instead. Number coercion
//!    still matches legacy semantics (Int/Float unified through f64, string
//!    comparisons try i64 parsing before lexical order).
//!
//! Null handling: `Eq`/`Ne` treat Null as an ordinary value (`Null == Null`
//! is true). Every other operator propagates Null (`1 + Null -> Null`,
//! `Null > 1 -> Null`). A missing field resolves to Null before evaluation.

use std::fmt;

use crate::query_planner::ir::{FieldPath, LogicalExpr, QueryRecord, QueryValue};

pub mod idiom;
pub mod literal;
pub mod ops;

pub use idiom::{FieldExpr, RecordSource, StoredSource};
pub use literal::LiteralExpr;
pub use ops::{BinaryExpr, UnaryExpr};

/// Why an expression failed to evaluate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprError {
    /// Operand kinds are incompatible for this operator.
    TypeMismatch {
        op: &'static str,
        left: &'static str,
        right: &'static str,
    },
    /// Unary operand kind is incompatible.
    UnaryTypeMismatch {
        op: &'static str,
        operand: &'static str,
    },
    DivisionByZero,
    ArithmeticOverflow,
    /// Named scalar function is not registered (3.1b registry lookup).
    UnknownFunction(String),
    /// Subquery expressions execute through the Stage 3.4 control-flow
    /// bridge; compiling one before then is rejected here.
    UnsupportedSubquery,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::TypeMismatch { op, left, right } => {
                write!(f, "type mismatch for {op}: {left} vs {right}")
            }
            ExprError::UnaryTypeMismatch { op, operand } => {
                write!(f, "type mismatch for unary {op}: {operand}")
            }
            ExprError::DivisionByZero => write!(f, "division by zero"),
            ExprError::ArithmeticOverflow => write!(f, "arithmetic overflow"),
            ExprError::UnknownFunction(name) => write!(f, "unknown function {name:?}"),
            ExprError::UnsupportedSubquery => {
                write!(f, "subquery expressions require the Stage 3.4 bridge")
            }
        }
    }
}

impl std::error::Error for ExprError {}

/// Row data an expression may reference while evaluating.
///
/// Two production shapes exist: materialized records ([`RecordSource`]) and
/// live stored-field lookups through the Phase-1 bridge ([`StoredSource`],
/// which reuses `PlannerFieldEval::stored_field` so relation fields keep the
/// edge-fallback semantics filters already rely on).
pub trait FieldSource {
    /// Resolve a path against this row. `None` means the field (or an
    /// intermediate segment) is absent; the evaluator maps that to Null.
    fn resolve(&self, path: &FieldPath) -> Option<QueryValue>;
}

/// Evaluation context handed to every expression node.
#[derive(Clone, Copy)]
pub struct EvalContext<'a> {
    pub row: &'a dyn FieldSource,
}

impl<'a> EvalContext<'a> {
    pub fn new(row: &'a dyn FieldSource) -> Self {
        EvalContext { row }
    }

    fn resolve(&self, path: &FieldPath) -> Option<QueryValue> {
        self.row.resolve(path)
    }
}

/// A compiled, executable expression node.
pub trait PhysicalExpr: fmt::Debug + Send + Sync {
    fn evaluate(&self, ctx: &EvalContext) -> Result<QueryValue, ExprError>;

    /// Structural description used by explain output and the 3.1c registry.
    fn describe(&self) -> String;
}

/// Compile a logical expression tree into an executable one.
///
/// `Function` nodes compile once the 3.1b registry lands; `Subquery` nodes
/// stay unsupported until the Stage 3.4 control-flow bridge exists.
pub fn compile(expr: &LogicalExpr) -> Result<Box<dyn PhysicalExpr>, ExprError> {
    match expr {
        LogicalExpr::Value(v) => Ok(Box::new(LiteralExpr::new(v.clone()))),
        LogicalExpr::Field(path) => Ok(Box::new(FieldExpr::new(path.clone()))),
        LogicalExpr::Binary { left, op, right } => Ok(Box::new(BinaryExpr::new(
            compile(left)?,
            *op,
            compile(right)?,
        ))),
        LogicalExpr::Unary { op, expr } => Ok(Box::new(UnaryExpr::new(*op, compile(expr)?))),
        LogicalExpr::Function { .. } => Err(ExprError::UnknownFunction(String::new())),
        LogicalExpr::Subquery(_) => Err(ExprError::UnsupportedSubquery),
    }
}

// ---------------------------------------------------------------------------
// Shared value helpers (used by ops.rs and later stages)
// ---------------------------------------------------------------------------

/// Numeric view of a scalar: Int/Float unify through f64 (legacy parity).
pub(crate) fn as_number(v: &QueryValue) -> Option<f64> {
    match v {
        QueryValue::Int(i) => Some(*i as f64),
        QueryValue::Float(f) => Some(*f),
        _ => None,
    }
}

/// Equality with numeric cross-type unification; `Enum` equals its string.
pub(crate) fn value_eq(left: &QueryValue, right: &QueryValue) -> bool {
    match (left, right) {
        (QueryValue::Null, QueryValue::Null) => true,
        (l, r) if as_number(l).is_some() && as_number(r).is_some() => {
            as_number(l) == as_number(r)
        }
        (QueryValue::Enum(a), QueryValue::String(b)) | (QueryValue::String(b), QueryValue::Enum(a)) => a == b,
        (QueryValue::List(a), QueryValue::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| value_eq(x, y))
        }
        _ => left == right,
    }
}

/// Three-way ordering comparison for scalars, mirroring legacy
/// `check_condition`: numbers compare through f64; strings try i64 parsing
/// first and fall back to lexical order; `Enum` compares as its string.
pub(crate) fn value_cmp(left: &QueryValue, right: &QueryValue) -> Result<std::cmp::Ordering, ExprError> {
    use std::cmp::Ordering;
    if let (Some(l), Some(r)) = (as_number(left), as_number(right)) {
        return Ok(l.partial_cmp(&r).unwrap_or(Ordering::Equal));
    }
    match (as_string_like(left), as_string_like(right)) {
        (Some(ls), Some(rs)) => {
            if let (Ok(li), Ok(ri)) = (ls.parse::<i64>(), rs.parse::<i64>()) {
                Ok(li.cmp(&ri))
            } else {
                Ok(ls.cmp(&rs))
            }
        }
        (None, _) => Err(mismatch("compare", left)),
        (_, None) => Err(mismatch("compare", right)),
    }
}

fn as_string_like(v: &QueryValue) -> Option<String> {
    match v {
        QueryValue::String(s) | QueryValue::Enum(s) => Some(s.clone()),
        _ => None,
    }
}

pub(crate) fn type_name(v: &QueryValue) -> &'static str {
    match v {
        QueryValue::Null => "null",
        QueryValue::Bool(_) => "bool",
        QueryValue::Int(_) => "int",
        QueryValue::Float(_) => "float",
        QueryValue::String(_) => "string",
        QueryValue::Enum(_) => "enum",
        QueryValue::List(_) => "list",
        QueryValue::Object(_) => "object",
        QueryValue::EntityId(_) => "entity_id",
    }
}

fn mismatch(op: &'static str, v: &QueryValue) -> ExprError {
    ExprError::TypeMismatch {
        op,
        left: type_name(v),
        right: "?",
    }
}

/// Convenience: evaluate against a borrowed record without building a source.
pub fn eval_record(expr: &dyn PhysicalExpr, record: &QueryRecord) -> Result<QueryValue, ExprError> {
    let src = RecordSource::new(record);
    expr.evaluate(&EvalContext::new(&src))
}
