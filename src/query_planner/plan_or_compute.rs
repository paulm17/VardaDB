//! The single Stage 3.4 fallback bridge between the planner-operator runtime
//! and legacy resolver behavior (upstream `plan_or_compute` analog).
//!
//! Contract: whenever the operator pipeline cannot express a construct, it
//! surfaces [`PlannerError::Unsupported`]. [`plan_or_compute`] routes exactly
//! that variant to a caller-provided fallback while storage failures
//! propagate — a broken backend must never silently degrade into legacy
//! code paths.
//!
//! Two concrete bridges ship here:
//!
//! - [`field_value`]: evaluate an expression against one stored row.
//!   Bare single-segment field references keep legacy semantics for free
//!   because [`StoredSource`] already resolves root segments through
//!   `PlannerFieldEval::stored_field` (relation edge-fallback included).
//! - [`candidates_or_legacy`]: run a candidate plan through the operator
//!   source tree; `Unsupported` (at build or execute time) yields `None`,
//!   telling callers to fall back to legacy scanning.

use crate::query_planner::ir::{EntityId, LogicalExpr, QueryValue};
use crate::query_planner::operators::{
    build_source_tree, ExecContext, ExecOperator, FilterOperator, FlowResult, PlannerError,
};
use crate::query_planner::physical_expr::{compile_arc, EvalContext, StoredSource};
use crate::query_planner::plan::CandidatePlan;
use crate::query_planner::traits::{PlannerFieldEval, PlannerRuntime};

/// Run `plan`, routing only [`PlannerError::Unsupported`] to `fallback`.
pub fn plan_or_compute<T>(
    plan: impl FnOnce() -> Result<T, PlannerError>,
    fallback: impl FnOnce(PlannerError) -> Result<T, PlannerError>,
) -> Result<T, PlannerError> {
    match plan() {
        Ok(value) => Ok(value),
        Err(err @ PlannerError::Unsupported(_)) => fallback(err),
        Err(err) => Err(err),
    }
}

/// Evaluate `expr` against the stored fields of one entity.
///
/// Errors from expression compilation/evaluation map to
/// [`PlannerError::Unsupported`]; missing fields evaluate to Null.
pub fn field_value(
    runtime: &dyn PlannerFieldEval,
    id: EntityId,
    expr: &LogicalExpr,
) -> Result<QueryValue, PlannerError> {
    let compiled = compile_arc(expr).map_err(|e| PlannerError::Unsupported(e.to_string()))?;
    let source = StoredSource::new(runtime, id);
    compiled
        .evaluate(&EvalContext::new(&source))
        .map_err(|e| PlannerError::Unsupported(e.to_string()))
}

/// Execute a candidate plan's source tree, or report legacy fallback.
///
/// `Ok(None)` means "no operator-level narrowing available" (`Unsupported`
/// surfaced anywhere in the tree); `Ok(Some(ids))` is the deduplicated,
/// uid-ascending narrowed set, matching candidate-set conventions elsewhere
/// in the planner.
pub fn candidates_or_legacy(
    plan: &CandidatePlan,
    runtime: &dyn PlannerRuntime,
    db: &str,
) -> Result<Option<Vec<EntityId>>, PlannerError> {
    plan_or_compute(
        || {
            let source = build_source_tree(&plan.type_name, &plan.source)?;
            let pipeline: Box<dyn ExecOperator> = match &plan.residual {
                Some(filter) if !filter.is_empty_conjunction() => {
                    FilterOperator::boxed(source, filter.clone())
                }
                _ => source,
            };
            let mut ctx = ExecContext::new(runtime, db);
            match pipeline.execute(&mut ctx) {
                FlowResult::Rows(batches) => {
                    let mut ids: Vec<EntityId> = batches.into_iter().flat_map(|b| b.0).collect();
                    ids.sort_by_key(|e| e.uid);
                    ids.dedup_by_key(|e| e.uid);
                    Ok(Some(ids))
                }
                FlowResult::Break | FlowResult::Continue => Ok(Some(Vec::new())),
                FlowResult::Error(err) => Err(err),
            }
        },
        |_unsupported| Ok(None),
    )
}
