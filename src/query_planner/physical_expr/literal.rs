//! Literal expression nodes.

use crate::query_planner::ir::QueryValue;
use crate::query_planner::physical_expr::{EvalContext, ExprError, PhysicalExpr};

#[derive(Debug, Clone)]
pub struct LiteralExpr {
    value: QueryValue,
}

impl LiteralExpr {
    pub fn new(value: QueryValue) -> Self {
        LiteralExpr { value }
    }
}

impl PhysicalExpr for LiteralExpr {
    fn evaluate(&self, _ctx: &EvalContext) -> Result<QueryValue, ExprError> {
        Ok(self.value.clone())
    }

    fn describe(&self) -> String {
        match &self.value {
            QueryValue::String(s) => format!("{s:?}"),
            other => format!("{other:?}"),
        }
    }
}
