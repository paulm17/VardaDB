//! Subquery expression evaluation (M-C).
//!
//! A [`LogicalExpr::Subquery`] wraps a [`LogicalQuery`]. V1 semantics:
//!
//! - only `QueryRoot::TypeScan` roots are supported;
//! - the child filter compiles through the same [`FilterOperator`]
//!   machinery residual filters use, so nested relation arms and
//!   computed-expression nodes behave identically;
//! - results are the child uids as strings (matching the uid-string edge
//!   encoding every relation field stores), sorted ascending and truncated
//!   to `pagination.first` when present — sufficient for `Contains` / `In`
//!   membership tests;
//! - the result is row-independent (child filters cannot reference the
//!   outer row), so it is evaluated once and cached;
//! - evaluation requires an [`EvalContext`] carrying pipeline access
//!   (`with_runtime`); bare contexts raise
//!   [`ExprError::UnsupportedSubquery`].
//!
//! The node stores owned IR (`LogicalQuery` parts are `Send + Sync`) and
//! rebuilds its operator pipeline per evaluation; the cache makes that a
//! one-time cost.

use std::fmt;
use std::sync::Mutex;

use crate::query_planner::ir::{LogicalFilter, LogicalQuery, QueryRoot};
use crate::query_planner::operators::{
    build_source_tree, ExecContext, ExecOperator, FilterOperator, FlowResult,
};
use crate::query_planner::plan::CandidateSource;
use crate::query_planner::traits::PlannerRuntime;

use super::{EvalContext, ExprError, PhysicalExpr, QueryValue};

pub struct SubqueryExpr {
    type_name: String,
    filter: Option<LogicalFilter>,
    limit: Option<usize>,
    cache: Mutex<Option<Vec<QueryValue>>>,
}

impl fmt::Debug for SubqueryExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SubqueryExpr(scan {}, first={:?})",
            self.type_name, self.limit
        )
    }
}

impl SubqueryExpr {
    pub fn try_new(query: LogicalQuery) -> Result<Self, ExprError> {
        let type_name = match &query.root {
            QueryRoot::TypeScan { type_name } => type_name.clone(),
            other => {
                return Err(ExprError::Execution(format!(
                    "unsupported subquery root: {other:?}"
                )))
            }
        };
        if let Some(filter) = &query.filter {
            // Fail compilation on malformed child filters up front.
            crate::query_planner::operators::compile_filter(filter)?;
        }
        Ok(SubqueryExpr {
            type_name,
            filter: query.filter,
            limit: query.pagination.first,
            cache: Mutex::new(None),
        })
    }

    fn pipeline(&self) -> Result<Box<dyn ExecOperator>, ExprError> {
        let source = build_source_tree(&self.type_name, &CandidateSource::FullTypeScan)
            .map_err(|e| ExprError::Execution(e.to_string()))?;
        match &self.filter {
            Some(filter) if !filter.is_empty_conjunction() => {
                FilterOperator::try_boxed(source, filter.clone())
            }
            _ => Ok(source),
        }
    }

    fn collect(&self, rt: &dyn PlannerRuntime, db: &str) -> Result<Vec<QueryValue>, ExprError> {
        let mut ctx = ExecContext::new_with_explain(rt, db, false);
        match self.pipeline()?.execute(&mut ctx) {
            FlowResult::Rows(batches) => {
                let mut uids: Vec<u64> =
                    batches.into_iter().flat_map(|b| b.0).map(|e| e.uid).collect();
                uids.sort_unstable();
                uids.dedup();
                if let Some(first) = self.limit {
                    uids.truncate(first);
                }
                Ok(uids
                    .into_iter()
                    .map(|uid| QueryValue::String(uid.to_string()))
                    .collect())
            }
            FlowResult::Break | FlowResult::Continue => Ok(Vec::new()),
            FlowResult::Error(e) => Err(ExprError::Execution(e.to_string())),
        }
    }
}

impl PhysicalExpr for SubqueryExpr {
    fn evaluate(&self, ctx: &EvalContext) -> Result<QueryValue, ExprError> {
        {
            let cached = self
                .cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(values) = cached.as_ref() {
                return Ok(QueryValue::List(values.clone()));
            }
        }
        let rt = ctx.runtime.ok_or(ExprError::UnsupportedSubquery)?;
        let db = ctx.db_name.unwrap_or("default");
        let values = self.collect(rt, db)?;
        *self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(values.clone());
        Ok(QueryValue::List(values))
    }

    fn describe(&self) -> String {
        format!("subquery(scan {})", self.type_name)
    }
}
