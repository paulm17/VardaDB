//! Hash-grouped aggregation operator (upstream `exec/operators/aggregate.rs`
//! analog).
//!
//! Consumes uid batches from its input, evaluates optional group-key
//! expressions and per-spec argument expressions against each row through the
//! stored-field bridge, and folds values into [`Accumulator`] state per
//! group.
//!
//! Results are scalar rows, not uid sets — the operator stores them in
//! `groups` for the caller (dispatcher or test) to read after `execute`.
//! Group rows come back sorted by their key tuple so output ordering is
//! deterministic regardless of hash iteration order (mirroring upstream's
//! BTreeMap-ordered finalize).
//!
//! Null semantics: argument values reach accumulators unchanged and each
//! accumulator applies SQL-style Null skipping itself; a group key evaluating
//! to Null groups all such rows together under the Null key.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use crate::query_planner::function::{Accumulator, AggregateFunction};
use crate::query_planner::ir::QueryValue;
use crate::query_planner::physical_expr::{
    EvalContext, PhysicalExpr, StoredSource,
};

use super::{
    CardinalityHint, ExecContext, ExecOperator, FlowResult, OperatorKind, OperatorStat,
    OutputOrdering, PlannerError, RowBatch,
};

/// One planned aggregate output: registry function + compiled argument.
#[derive(Debug)]
pub struct AggregateSpec {
    pub func: Arc<dyn AggregateFunction>,
    /// Argument expression; `None` only for functions that ignore arguments.
    pub arg: Option<Arc<dyn PhysicalExpr>>,
    pub alias: String,
}

/// Materialized result for one group.
#[derive(Debug, Clone)]
pub struct AggGroupRow {
    /// Evaluated group-by key values (empty for global aggregation).
    pub key: Vec<QueryValue>,
    /// `(alias, value)` pairs in spec order.
    pub outputs: Vec<(String, QueryValue)>,
}

struct GroupState {
    #[allow(dead_code)]
    key: Vec<QueryValue>,
    accs: Vec<Box<dyn Accumulator>>,
}

pub struct HashAggregateOperator {
    input: Box<dyn ExecOperator>,
    specs: Vec<AggregateSpec>,
    group_by: Vec<Arc<dyn PhysicalExpr>>,
    /// Filled by `execute`. The [`ExecOperator`] trait executes through
    /// `&self`, so results live behind a `RefCell` for the caller to drain
    /// afterwards (execution is single-threaded).
    groups: RefCell<Vec<AggGroupRow>>,
}

impl std::fmt::Debug for HashAggregateOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HashAggregateOperator")
            .field(
                "specs",
                &self.specs.iter().map(|s| s.alias.clone()).collect::<Vec<_>>(),
            )
            .field("group_by", &self.group_by.len())
            .finish_non_exhaustive()
    }
}

impl HashAggregateOperator {
    pub fn new(
        input: Box<dyn ExecOperator>,
        specs: Vec<AggregateSpec>,
        group_by: Vec<Arc<dyn PhysicalExpr>>,
    ) -> Self {
        HashAggregateOperator {
            input,
            specs,
            group_by,
            groups: RefCell::new(Vec::new()),
        }
    }

    pub fn boxed(
        input: Box<dyn ExecOperator>,
        specs: Vec<AggregateSpec>,
        group_by: Vec<Arc<dyn PhysicalExpr>>,
    ) -> Box<dyn ExecOperator> {
        Box::new(HashAggregateOperator::new(input, specs, group_by))
    }

    /// Drain the materialized group rows from the most recent execution
    /// (sorted by key), leaving the operator empty for reuse.
    pub fn take_groups(&self) -> Vec<AggGroupRow> {
        std::mem::take(&mut *self.groups.borrow_mut())
    }

    /// Borrowed view of current group rows.
    pub fn groups(&self) -> std::cell::Ref<'_, [AggGroupRow]> {
        std::cell::Ref::map(self.groups.borrow(), |g| g.as_slice())
    }

    /// Convenience for the single-count pipeline shape: first spec's output
    /// of the first group as i64.
    pub fn first_count(&self) -> Option<i64> {
        let groups = self.groups.borrow();
        match groups.first()?.outputs.first()?.1 {
            QueryValue::Int(n) => Some(n),
            _ => None,
        }
    }

    fn describe_exprs(&self) -> String {
        self.specs
            .iter()
            .map(|s| {
                let arg = s
                    .arg
                    .as_ref()
                    .map(|a| a.describe())
                    .unwrap_or_else(|| "*".to_string());
                format!("{}({}) AS {}", s.func.name(), arg, s.alias)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn feed_row(
        specs: &[AggregateSpec],
        state: &mut GroupState,
        eval_ctx: &EvalContext,
    ) -> Result<(), crate::query_planner::physical_expr::ExprError> {
        for (spec, acc) in specs.iter().zip(state.accs.iter_mut()) {
            let Some(arg) = &spec.arg else { continue };
            let value = arg.evaluate(eval_ctx)?;
            acc.update(&value)?;
        }
        Ok(())
    }
}

impl ExecOperator for HashAggregateOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Aggregate
    }

    fn detail(&self) -> String {
        format!(
            "hash_aggregate {} group_by={}",
            self.describe_exprs(),
            self.group_by.len()
        )
    }

    fn cardinality(&self) -> CardinalityHint {
        CardinalityHint::Unbounded
    }

    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }

    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![self.input.as_ref()]
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        use crate::query_planner::physical_expr::ExprError;

        let start = std::time::Instant::now();
        let batches = match self.input.execute(ctx) {
            FlowResult::Rows(batches) => batches,
            other => return other,
        };

        // Group state: hash key -> accumulators. Keys hash through their
        // deterministic Debug representation with exact-match verification on
        // collision; map size is the group count, never the row count.
        let mut map: HashMap<String, GroupState> = HashMap::new();

        let mut rows_in = 0usize;
        for batch in batches {
            rows_in += batch.0.len();
            for id in batch.0 {
                let source = StoredSource::new(ctx.runtime, id);
                let eval_ctx = EvalContext::new(&source);
                let key_values: Vec<QueryValue> = self
                    .group_by
                    .iter()
                    .map(|expr| expr.evaluate(&eval_ctx).unwrap_or(QueryValue::Null))
                    .collect();
                let hash_key = format!("{key_values:?}");
                if !map.contains_key(&hash_key) {
                    let accs = self
                        .specs
                        .iter()
                        .map(|s| s.func.create_accumulator())
                        .collect();
                    map.insert(hash_key.clone(), GroupState { key: key_values, accs });
                }
                let state = map.get_mut(&hash_key).expect("group inserted above");
                if let Err(err) = Self::feed_row(&self.specs, state, &eval_ctx) {
                    let detail = match err {
                        ExprError::TypeMismatch { op, left, right } => format!(
                            "aggregate argument type mismatch for {op}: {left} vs {right}"
                        ),
                        other => format!("aggregate argument evaluation failed: {other}"),
                    };
                    return FlowResult::Error(PlannerError::Storage(detail));
                }
            }
        }

        // Global aggregation over zero rows still yields one group (the
        // all-defaults row: count=0, sum/min/max=Null), matching SQL
        // `SELECT count(*) FROM t WHERE false` semantics.
        if self.group_by.is_empty() && map.is_empty() {
            let accs = self
                .specs
                .iter()
                .map(|s| s.func.create_accumulator())
                .collect();
            map.insert(String::new(), GroupState {
                key: Vec::new(),
                accs,
            });
        }

        // Deterministic finalize order: sort by the canonical key tuple.
        let mut rows: Vec<(Vec<QueryValue>, Vec<(String, QueryValue)>)> =
            Vec::with_capacity(map.len());
        for (_, mut state) in map {
            let mut outputs = Vec::with_capacity(self.specs.len());
            for (spec, acc) in self.specs.iter().zip(state.accs.iter_mut()) {
                let value = match acc.finalize() {
                    Ok(v) => v,
                    Err(err) => {
                        return FlowResult::Error(PlannerError::Storage(format!(
                            "aggregate finalize failed: {err}"
                        )))
                    }
                };
                outputs.push((spec.alias.clone(), value));
            }
            rows.push((std::mem::take(&mut state.key), outputs));
        }
        rows.sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
        *self.groups.borrow_mut() = rows
            .into_iter()
            .map(|(key, outputs)| AggGroupRow { key, outputs })
            .collect();

        ctx.explain.record(OperatorStat {
            kind: "aggregate".to_string(),
            detail: self.detail(),
            rows_in,
            rows_out: self.groups.borrow().len(),
            elapsed_us: start.elapsed().as_micros() as u64,
            notes: vec![format!(
                "{} group(s) from {} row(s)",
                self.groups.borrow().len(),
                rows_in
            )],
        });

        FlowResult::Rows(Vec::new())
    }
}
