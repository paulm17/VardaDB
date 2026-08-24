//! Residual filter operator.
//!
//! Evaluates a [`LogicalFilter`] against rows pulled from its input. The
//! structural IR (and/or/not/relation traversal) is evaluated here; every
//! leaf condition comparison delegates to [`PlannerFieldEval`], which bridges
//! directly to the legacy `SqliteResolver::check_condition` semantics so the
//! two paths can never drift apart.

use super::{
    ExecContext, ExecOperator, FlowResult, OperatorKind, OperatorStat, OutputOrdering, RowBatch,
};
use crate::query_planner::ir::{EntityId, FilterOp, LogicalFilter};

/// Filters incoming batches down to rows whose entity satisfies the predicate
/// tree. Filtering preserves input order and can only shrink cardinality, so
/// both metadata declarations inherit from the input operator.
pub struct FilterOperator {
    input: Box<dyn ExecOperator>,
    filter: LogicalFilter,
}

impl FilterOperator {
    pub fn new(input: Box<dyn ExecOperator>, filter: LogicalFilter) -> Self {
        FilterOperator { input, filter }
    }

    pub fn boxed(
        input: Box<dyn ExecOperator>,
        filter: LogicalFilter,
    ) -> Box<dyn ExecOperator> {
        Box::new(FilterOperator { input, filter })
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
                .filter(|id| eval(ctx, id.uid, &self.filter))
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
                count_conditions(&self.filter)
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
        format!("residual({} conditions)", count_conditions(&self.filter))
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

/// Structural evaluation of one row against the filter tree.
fn eval(ctx: &ExecContext, uid: u64, filter: &LogicalFilter) -> bool {
    match filter {
        LogicalFilter::And(parts) => parts.iter().all(|f| eval(ctx, uid, f)),
        LogicalFilter::Or(parts) => parts.iter().any(|f| eval(ctx, uid, f)),
        LogicalFilter::Not(inner) => !eval(ctx, uid, inner),
        LogicalFilter::Predicate(pred) => eval_predicate(ctx, uid, pred.op, &pred.value, pred.path.single()),
        LogicalFilter::Relation { field, filter, .. } => {
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
    sub: &LogicalFilter,
) -> bool {
    let stored = ctx.runtime.stored_field(&EntityId::from(uid), field);
    match stored {
        Some(async_graphql::Value::String(s)) => match s.parse::<u64>() {
            Ok(child_uid) => eval(ctx, child_uid, sub),
            Err(_) => false,
        },
        Some(async_graphql::Value::Number(n)) => match n.as_u64() {
            Some(child_uid) => eval(ctx, child_uid, sub),
            None => false,
        },
        Some(async_graphql::Value::List(list)) => {
            let mut matched = false;
            for item in &list {
                if let Some(child_uid) = local_value_to_uid(item) {
                    if eval(ctx, child_uid, sub) {
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
        LogicalFilter::Relation { filter, .. } => 1 + count_conditions(filter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::adapters::runtime_for_test_stub;
    use crate::query_planner::ir::{FieldPath, FieldSegment, FilterPredicate, QueryValue};
    use crate::query_planner::operators::CardinalityHint;

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
        assert!(eval(&ctx, 1, &f_and));

        let f_or_empty = LogicalFilter::Or(vec![]);
        assert!(!eval(&ctx, 1, &f_or_empty));

        let f_not = LogicalFilter::Not(Box::new(eq_pred("a", 1)));
        assert!(!eval(&ctx, 1, &f_not));
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
        assert!(eval(&ctx, 5, &f));
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
        assert!(eval(&ctx, 9, &f));

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
        assert!(!eval(&ctx, 9, &nested));
    }
}
