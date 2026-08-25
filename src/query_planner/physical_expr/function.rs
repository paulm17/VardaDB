//! Scalar-function application node for compiled expression trees.

use std::sync::Arc;

use crate::query_planner::function::ScalarFunction;
use crate::query_planner::ir::QueryValue;

use super::{EvalContext, ExprError, PhysicalExpr};

/// Applies a registered [`ScalarFunction`] to evaluated arguments.
///
/// Uniform null semantics: if any argument evaluates to Null the call
/// short-circuits to Null without invoking the function — matching how every
/// other operator in this runtime propagates Null. Type enforcement stays
/// inside the function implementations (strict typing).
pub struct FunctionExpr {
    name: String,
    func: Arc<dyn ScalarFunction>,
    args: Vec<Box<dyn PhysicalExpr>>,
}

impl FunctionExpr {
    pub fn new(
        name: String,
        func: Arc<dyn ScalarFunction>,
        args: Vec<Box<dyn PhysicalExpr>>,
    ) -> Self {
        FunctionExpr { name, func, args }
    }
}

impl std::fmt::Debug for FunctionExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionExpr")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl PhysicalExpr for FunctionExpr {
    fn evaluate(&self, ctx: &EvalContext) -> Result<QueryValue, ExprError> {
        let mut values = Vec::with_capacity(self.args.len());
        for arg in &self.args {
            let value = arg.evaluate(ctx)?;
            if matches!(value, QueryValue::Null) {
                return Ok(QueryValue::Null);
            }
            values.push(value);
        }
        self.func.evaluate(&values)
    }

    fn describe(&self) -> String {
        let rendered = self
            .args
            .iter()
            .map(|a| a.describe())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}({})", self.name, rendered)
    }
}
