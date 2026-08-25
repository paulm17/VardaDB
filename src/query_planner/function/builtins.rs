//! The nine Stage 3.1b builtin scalar functions.
//!
//! Contract (uniform across all builtins):
//! - **Null propagation** happens one level up: [`FunctionExpr`] short-circuits
//!   any Null argument to Null before dispatch, so `evaluate` normally never
//!   sees Null. The defensive arms below only matter for direct calls.
//! - **Strict typing**: wrong argument kinds raise
//!   `ExprError::TypeMismatch` (never silent coercion).

use std::sync::Arc;

use crate::query_planner::ir::QueryValue;
use crate::query_planner::physical_expr::{type_name, ExprError};

use super::{ScalarFunction, Signature};

fn expect_str<'a>(
    fname: &'static str,
    args: &'a [QueryValue],
) -> Result<&'a str, ExprError> {
    match &args[0] {
        QueryValue::String(s) => Ok(s),
        QueryValue::Null => Ok(""),
        other => Err(ExprError::TypeMismatch {
            op: fname,
            left: type_name(other),
            right: "string",
        }),
    }
}

macro_rules! string_fn {
    ($name:ident, $op:expr, $body:expr) => {
        #[derive(Debug)]
        pub struct $name;

        impl ScalarFunction for $name {
            fn name(&self) -> &'static str {
                $op
            }

            fn signature(&self) -> Signature {
                Signature::new().arg("string", "string").returns("string")
            }

            fn evaluate(&self, args: &[QueryValue]) -> Result<QueryValue, ExprError> {
                let s = expect_str($op, args)?;
                let body: fn(&str) -> String = $body;
                Ok(QueryValue::String(body(s)))
            }
        }
    };
}

string_fn!(Lower, "lower", |s| s.to_lowercase());
string_fn!(Upper, "upper", |s| s.to_uppercase());
string_fn!(Trim, "trim", |s| s.trim().to_string());

#[derive(Debug)]
pub struct Len;

impl ScalarFunction for Len {
    fn name(&self) -> &'static str {
        "len"
    }

    fn signature(&self) -> Signature {
        Signature::new().arg("value", "any").returns("int")
    }

    fn evaluate(&self, args: &[QueryValue]) -> Result<QueryValue, ExprError> {
        let n = match &args[0] {
            QueryValue::String(s) => s.chars().count() as i64,
            QueryValue::List(items) => items.len() as i64,
            QueryValue::Object(fields) => fields.len() as i64,
            QueryValue::Null => return Ok(QueryValue::Null),
            other => {
                return Err(ExprError::TypeMismatch {
                    op: "len",
                    left: type_name(other),
                    right: "string|list|object",
                })
            }
        };
        Ok(QueryValue::Int(n))
    }
}

#[derive(Debug)]
pub struct Concat;

impl ScalarFunction for Concat {
    fn name(&self) -> &'static str {
        "concat"
    }

    fn signature(&self) -> Signature {
        Signature::new()
            .arg("first", "string")
            .variadic("rest")
            .returns("string")
    }

    fn evaluate(&self, args: &[QueryValue]) -> Result<QueryValue, ExprError> {
        let mut joined = String::new();
        for arg in args {
            match arg {
                QueryValue::String(s) => joined.push_str(s),
                QueryValue::Null => {}
                other => {
                    return Err(ExprError::TypeMismatch {
                        op: "concat",
                        left: type_name(other),
                        right: "string",
                    })
                }
            }
        }
        Ok(QueryValue::String(joined))
    }
}

#[derive(Debug)]
pub struct Abs;

impl ScalarFunction for Abs {
    fn name(&self) -> &'static str {
        "abs"
    }

    fn signature(&self) -> Signature {
        Signature::new().arg("number", "number").returns("number")
    }

    fn evaluate(&self, args: &[QueryValue]) -> Result<QueryValue, ExprError> {
        match &args[0] {
            QueryValue::Int(i) => i
                .checked_abs()
                .map(QueryValue::Int)
                .ok_or(ExprError::ArithmeticOverflow),
            QueryValue::Float(f) => Ok(QueryValue::Float(f.abs())),
            QueryValue::Null => Ok(QueryValue::Null),
            other => Err(ExprError::TypeMismatch {
                op: "abs",
                left: type_name(other),
                right: "number",
            }),
        }
    }
}

/// Shared shape for the three rounding functions (`ceil`/`floor`/`round`):
/// Int passes through unchanged; Float applies the f64 operation.
macro_rules! round_fn {
    ($name:ident, $op:expr, $method:ident) => {
        #[derive(Debug)]
        pub struct $name;

        impl ScalarFunction for $name {
            fn name(&self) -> &'static str {
                $op
            }

            fn signature(&self) -> Signature {
                Signature::new().arg("number", "number").returns("number")
            }

            fn evaluate(&self, args: &[QueryValue]) -> Result<QueryValue, ExprError> {
                match &args[0] {
                    QueryValue::Int(_) => Ok(args[0].clone()),
                    QueryValue::Float(f) => Ok(QueryValue::Float(f.$method())),
                    QueryValue::Null => Ok(QueryValue::Null),
                    other => Err(ExprError::TypeMismatch {
                        op: $op,
                        left: type_name(other),
                        right: "number",
                    }),
                }
            }
        }
    };
}

round_fn!(Ceil, "ceil", ceil);
round_fn!(Floor, "floor", floor);
round_fn!(Round, "round", round);

/// Every builtin registered in the default registry.
pub fn all() -> Vec<Arc<dyn ScalarFunction>> {
    vec![
        Arc::new(Lower),
        Arc::new(Upper),
        Arc::new(Len),
        Arc::new(Concat),
        Arc::new(Trim),
        Arc::new(Abs),
        Arc::new(Ceil),
        Arc::new(Floor),
        Arc::new(Round),
    ]
}
