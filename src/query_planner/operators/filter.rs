//! Residual filter operator.
//!
//! Evaluates a [`LogicalFilter`] against rows pulled from its input. The
//! structural IR (and/or/not/relation traversal) is evaluated here; every
//! leaf condition comparison delegates to [`PlannerFieldEval`], which bridges
//! directly to the legacy `SqliteResolver::check_condition` semantics so the
//! two paths can never drift apart.

use std::sync::Arc;

use super::{
    ExecContext, ExecOperator, FlowResult, OperatorKind, OperatorStat, OutputOrdering, RowBatch,
};
use crate::query_planner::ir::{EntityId, FilterOp, FilterPredicate, LogicalFilter};
use crate::query_planner::physical_expr::{
    EvalContext, ExprError, PhysicalExpr, StoredSource,
};

/// Precompiled filter tree: [`LogicalFilter::Expr`] nodes hold their compiled
/// physical expression; every other node mirrors the logical shape 1:1.
pub enum CompiledFilter {
    And(Vec<CompiledFilter>),
    Or(Vec<CompiledFilter>),
    Not(Box<CompiledFilter>),
    Predicate(FilterPredicate),
    /// Rows pass when the expression evaluates to `Bool(true)`; any other
    /// value or an evaluation error drops the row (strict-typing runtime).
    Expr(Arc<dyn PhysicalExpr>),
    Relation {
        field: String,
        target_type: String,
        filter: Box<CompiledFilter>,
    },
}

/// Compile a logical filter tree, resolving every computed-expression node
/// through `physical_expr::compile`.
pub fn compile_filter(filter: &LogicalFilter) -> Result<CompiledFilter, ExprError> {
    Ok(match filter {
        LogicalFilter::And(parts) => CompiledFilter::And(
            parts.iter().map(compile_filter).collect::<Result<_, _>>()?,
        ),
        LogicalFilter::Or(parts) => CompiledFilter::Or(
            parts.iter().map(compile_filter).collect::<Result<_, _>>()?,
        ),
        LogicalFilter::Not(inner) => CompiledFilter::Not(Box::new(compile_filter(inner)?)),
        LogicalFilter::Predicate(pred) => CompiledFilter::Predicate(pred.clone()),
        LogicalFilter::Expr(expr) => {
            CompiledFilter::Expr(crate::query_planner::physical_expr::compile_arc(expr)?)
        }
        LogicalFilter::Relation {
            field,
            target_type,
            filter,
        } => CompiledFilter::Relation {
            field: field.clone(),
            target_type: target_type.clone(),
            filter: Box::new(compile_filter(filter)?),
        },
    })
}

/// Filters incoming batches down to rows whose entity satisfies the predicate
/// tree. Filtering preserves input order and can only shrink cardinality, so
/// both metadata declarations inherit from the input operator.
pub struct FilterOperator {
    input: Box<dyn ExecOperator>,
    filter: CompiledFilter,
}

impl FilterOperator {
    /// Panics only if a computed-expression node fails to compile. Today's
    /// lowering never emits `Expr` nodes, so this is unreachable in practice;
    /// fallible construction goes through [`compile_filter`] directly.
    pub fn new(input: Box<dyn ExecOperator>, filter: LogicalFilter) -> Self {
        let compiled = compile_filter(&filter).expect("residual filter compiles");
        FilterOperator {
            input,
            filter: compiled,
        }
    }

    pub fn boxed(
        input: Box<dyn ExecOperator>,
        filter: LogicalFilter,
    ) -> Box<dyn ExecOperator> {
        Box::new(FilterOperator::new(input, filter))
    }

    /// Fallible construction for user-authored filters (subqueries,
    /// expression syntax) where an `Expr` node may fail to compile.
    pub fn try_boxed(
        input: Box<dyn ExecOperator>,
        filter: LogicalFilter,
    ) -> Result<Box<dyn ExecOperator>, crate::query_planner::physical_expr::ExprError> {
        let compiled = compile_filter(&filter)?;
        Ok(Box::new(FilterOperator { input, filter: compiled }))
    }

    fn execute_inner(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let child = self.input.execute(ctx);
        let batches = match child {
            FlowResult::Rows(b) => b,
            other => return other,
        };

        let mut rows_in = 0usize;
        let mut rows_out = 0usize;
        let mut kept: Vec<RowBatch> = Vec::new();
        for batch in batches {
            rows_in += batch.len();
            let remaining: Vec<EntityId> = batch
                .0
                .into_iter()
                .filter(|id| eval_compiled(ctx, id.uid, &self.filter))
                .collect();
            rows_out += remaining.len();
            if !remaining.is_empty() {
                kept.push(RowBatch(remaining));
            }
        }

        ctx.explain.record(OperatorStat {
            kind: self.kind().as_str().to_string(),
            detail: self.detail(),
            rows_in,
            rows_out,
            elapsed_us: start.elapsed().as_micros() as u64,
            notes: vec![format!(
                "residual conditions: {}",
                count_compiled(&self.filter)
            )],
        });

        FlowResult::Rows(kept)
    }
}

impl ExecOperator for FilterOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Filter
    }

    fn detail(&self) -> String {
        format!("residual({} conditions)", count_compiled(&self.filter))
    }

    fn cardinality(&self) -> super::CardinalityHint {
        self.input.cardinality()
    }

    fn output_ordering(&self) -> OutputOrdering {
        self.input.output_ordering()
    }

    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![self.input.as_ref()]
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        self.execute_inner(ctx)
    }
}

/// Structural evaluation of one row against the compiled filter tree.
fn eval_compiled(ctx: &ExecContext, uid: u64, filter: &CompiledFilter) -> bool {
    match filter {
        CompiledFilter::And(parts) => parts.iter().all(|f| eval_compiled(ctx, uid, f)),
        CompiledFilter::Or(parts) => parts.iter().any(|f| eval_compiled(ctx, uid, f)),
        CompiledFilter::Not(inner) => !eval_compiled(ctx, uid, inner),
        CompiledFilter::Predicate(pred) => {
            eval_predicate(ctx, uid, pred.op, &pred.value, pred.path.single())
        }
        CompiledFilter::Expr(expr) => expr
            .evaluate(&EvalContext::with_runtime(ctx.runtime, ctx.db_name, &StoredSource::new(
                ctx.runtime,
                EntityId::from(uid),
            )))
            .is_ok_and(|v| matches!(v, crate::query_planner::ir::QueryValue::Bool(true))),
        CompiledFilter::Relation { field, filter, .. } => {
            eval_relation(ctx, uid, field, filter)
        }
    }
}

fn eval_predicate(
    ctx: &ExecContext,
    uid: u64,
    op: FilterOp,
    value: &crate::query_planner::ir::QueryValue,
    field: Option<&str>,
) -> bool {
    // Text-search predicates are authoritative at their source operator
    // (BM25 scan); legacy residual evaluation ignores them, so we do too.
    if matches!(
        op,
        FilterOp::AllOfTerms | FilterOp::AnyOfTerms | FilterOp::AllOfText | FilterOp::AnyOfText
    ) {
        return true;
    }

    let Some(field) = field else {
        return false;
    };
    let stored = ctx.runtime.stored_field(&EntityId::from(uid), field);
    let cond_key = match op {
        FilterOp::Eq => "eq",
        FilterOp::Ne => "ne",
        FilterOp::Gt => "gt",
        FilterOp::Ge => "ge",
        FilterOp::Lt => "lt",
        FilterOp::Le => "le",
        FilterOp::In => "in",
        FilterOp::Contains => "contains",
        // Geo ops keep their legacy condition-object shape.
        FilterOp::Within => "within",
        FilterOp::Intersects => "intersects",
        // In-filter vector predicates do not exist in VardaDB GraphQL; the
        // only producer of NearVector is the geo `near` op (see lowering).
        FilterOp::NearVector => "near",
        FilterOp::AllOfTerms
        | FilterOp::AnyOfTerms
        | FilterOp::AllOfText
        | FilterOp::AnyOfText => unreachable!("handled above"),
    };
    let mut cond_map = async_graphql::indexmap::IndexMap::new();
    cond_map.insert(
        async_graphql::Name::new(cond_key),
        async_graphql::Value::from(value),
    );
    let condition = async_graphql::Value::Object(cond_map);
    ctx.runtime.eval_condition(&stored, &condition)
}

/// Relation traversal mirroring the legacy `check_filter_recursive_cached`
/// relation arm exactly: single-valued stored references recurse into that
/// child; list-typed stored values pass when any referenced child matches.
fn eval_relation(
    ctx: &ExecContext,
    uid: u64,
    field: &str,
    sub: &CompiledFilter,
) -> bool {
    let stored = ctx.runtime.stored_field(&EntityId::from(uid), field);
    match stored {
        Some(async_graphql::Value::String(s)) => match s.parse::<u64>() {
            Ok(child_uid) => eval_compiled(ctx, child_uid, sub),
            Err(_) => false,
        },
        Some(async_graphql::Value::Number(n)) => match n.as_u64() {
            Some(child_uid) => eval_compiled(ctx, child_uid, sub),
            None => false,
        },
        Some(async_graphql::Value::List(list)) => {
            let mut matched = false;
            for item in &list {
                if let Some(child_uid) = local_value_to_uid(item) {
                    if eval_compiled(ctx, child_uid, sub) {
                        matched = true;
                        break;
                    }
                }
            }
            matched
        }
        _ => false,
    }
}

/// Parity copy of the legacy `value_to_uid` coercion rules.
fn local_value_to_uid(value: &async_graphql::Value) -> Option<u64> {
    match value {
        async_graphql::Value::String(s) => s.parse::<u64>().ok(),
        async_graphql::Value::Number(n) => n.as_u64(),
        async_graphql::Value::Object(map) => map
            .get("uid")
            .or_else(|| map.get("id"))
            .and_then(local_value_to_uid),
        _ => None,
    }
}

pub fn count_conditions(filter: &LogicalFilter) -> usize {
    match filter {
        LogicalFilter::And(parts) | LogicalFilter::Or(parts) => {
            parts.iter().map(count_conditions).sum::<usize>()
        }
        LogicalFilter::Not(inner) => count_conditions(inner),
        LogicalFilter::Predicate(_) => 1,
        LogicalFilter::Expr(_) => 1,
        LogicalFilter::Relation { filter, .. } => 1 + count_conditions(filter),
    }
}

fn count_compiled(filter: &CompiledFilter) -> usize {
    match filter {
        CompiledFilter::And(parts) | CompiledFilter::Or(parts) => {
            parts.iter().map(count_compiled).sum::<usize>()
        }
        CompiledFilter::Not(inner) => count_compiled(inner),
        CompiledFilter::Predicate(_) | CompiledFilter::Expr(_) => 1,
        CompiledFilter::Relation { filter, .. } => 1 + count_compiled(filter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::adapters::runtime_for_test_stub;
    use crate::query_planner::ir::{FieldPath, FieldSegment, FilterPredicate, QueryValue};

    fn eq_pred(field: &str, v: i64) -> LogicalFilter {
        LogicalFilter::Predicate(FilterPredicate {
            path: FieldPath::field(field),
            op: FilterOp::Eq,
            value: QueryValue::Int(v),
        })
    }

    #[test]
    fn structural_and_or_not_evaluation() {
        let runtime = runtime_for_test_stub();
        let ctx = ExecContext::new(&runtime, "db");

        // Stub runtime passes every leaf condition, so And/Or/Not structure
        // drives outcomes.
        let f_and = LogicalFilter::And(vec![eq_pred("a", 1)]);
        assert!(eval_compiled(&ctx, 1, &compile_filter(&f_and).unwrap()));

        let f_or_empty = LogicalFilter::Or(vec![]);
        assert!(!eval_compiled(
            &ctx,
            1,
            &compile_filter(&f_or_empty).unwrap()
        ));

        let f_not = LogicalFilter::Not(Box::new(eq_pred("a", 1)));
        assert!(!eval_compiled(&ctx, 1, &compile_filter(&f_not).unwrap()));
    }

    #[test]
    fn text_predicates_pass_residual() {
        let runtime = runtime_for_test_stub();
        let ctx = ExecContext::new(&runtime, "db");
        let f = LogicalFilter::Predicate(FilterPredicate {
            path: FieldPath::field("title"),
            op: FilterOp::AllOfTerms,
            value: QueryValue::String("ignored here".into()),
        });
        assert!(eval_compiled(&ctx, 5, &compile_filter(&f).unwrap()));
    }

    #[test]
    fn single_segment_paths_evaluate_multi_segment_paths_reject() {
        // Lowering always produces single-segment paths (one GraphQL field).
        let f = LogicalFilter::Predicate(FilterPredicate {
            path: FieldPath::field("age"),
            op: FilterOp::Eq,
            value: QueryValue::Int(1),
        });
        let runtime = runtime_for_test_stub();
        let ctx = ExecContext::new(&runtime, "db");
        // Stub returns None stored + true eval_condition: still passes.
        assert!(eval_compiled(&ctx, 9, &compile_filter(&f).unwrap()));

        // Multi-segment paths have no legacy equivalent; they fail closed.
        let nested = LogicalFilter::Predicate(FilterPredicate {
            path: FieldPath {
                segments: vec![
                    FieldSegment::Field("age".to_string()),
                    FieldSegment::Index(0),
                ],
            },
            op: FilterOp::Eq,
            value: QueryValue::Int(1),
        });
        assert!(!eval_compiled(&ctx, 9, &compile_filter(&nested).unwrap()));
    }
}
