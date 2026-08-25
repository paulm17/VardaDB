//! Control-flow operators, ported from upstream `exec/operators/{expr,
//! compute,ifelse,foreach,sequence,return}.rs`.
//!
//! Adaptations to the VardaDB runtime (documented deviations):
//!
//! 1. **Value buffering.** Upstream operators stream `ValueBatch`s through
//!    async streams; VardaDB pipelines carry `RowBatch`s of entity ids. The
//!    scalar/value-producing operators here therefore stash their results in
//!    an internal buffer (`take_*` accessors) and emit empty batches — the
//!    same pattern [`HashAggregateOperator`](super::HashAggregateOperator)
//!    already established.
//! 2. **Foreach signals.** Without statement-level `BREAK`/`CONTINUE` syntax
//!    in the GraphQL surface, loop control is expressed with guards: a body
//!    step evaluating to `Bool(false)` acts as `continue` (remaining steps of
//!    that iteration are skipped), and an optional break predicate evaluated
//!    after each iteration terminates the loop.
//! 3. **Sequence** runs child operators in order until one breaks or errors;
//!    `Continue` from a child advances to the next step.
//! 4. **Return** executes its inner operator once, buffers the rows, and
//!    surfaces the terminal signal ([`FlowResult::Break`]); callers retrieve
//!    the payload via [`ReturnOperator::take_rows`].

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::query_planner::ir::{EntityId, FieldPath, QueryValue};
use crate::query_planner::physical_expr::idiom::walk;
use crate::query_planner::physical_expr::{
    EvalContext, FieldSource, PhysicalExpr, StoredSource,
};

use super::{
    ExecContext, ExecOperator, FlowResult, OperatorKind, OperatorStat, OutputOrdering,
    PlannerError, RowBatch,
};

/// Row source with no fields at all — scalar-expression context.
pub struct EmptySource;

impl FieldSource for EmptySource {
    fn resolve(&self, _path: &FieldPath) -> Option<QueryValue> {
        None
    }
}

/// Row source binding one loop variable over a base source; the variable is
/// referenced as a leading field segment named after it.
pub struct BoundSource<'a> {
    base: &'a dyn FieldSource,
    var: String,
    value: QueryValue,
}

impl<'a> BoundSource<'a> {
    pub fn new(base: &'a dyn FieldSource, var: impl Into<String>, value: QueryValue) -> Self {
        BoundSource {
            base,
            var: var.into(),
            value,
        }
    }
}

impl<'a> FieldSource for BoundSource<'a> {
    fn resolve(&self, path: &FieldPath) -> Option<QueryValue> {
        match path.segments.first() {
            Some(crate::query_planner::ir::FieldSegment::Field(name)) if *name == self.var => {
                let rest = FieldPath {
                    segments: path.segments[1..].to_vec(),
                };
                walk(&self.value, &rest)
            }
            _ => self.base.resolve(path),
        }
    }
}

// ---------------------------------------------------------------------------
// Scalar expression evaluation (upstream ExprPlan)
// ---------------------------------------------------------------------------

/// Evaluates a scalar expression without row context (`1 + 1`, `$param`).
pub struct ExprValueOperator {
    expr: Arc<dyn PhysicalExpr>,
    value: RefCell<Option<QueryValue>>,
}

impl ExprValueOperator {
    pub fn new(expr: Arc<dyn PhysicalExpr>) -> Self {
        ExprValueOperator {
            expr,
            value: RefCell::new(None),
        }
    }

    pub fn take_value(&self) -> Option<QueryValue> {
        self.value.borrow_mut().take()
    }
}

impl ExecOperator for ExprValueOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Expr
    }
    fn detail(&self) -> String {
        format!("expr {}", self.expr.describe())
    }
    fn cardinality(&self) -> super::CardinalityHint {
        super::CardinalityHint::AtMostOne
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        Vec::new()
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let source = EmptySource;
        match self.expr.evaluate(&EvalContext::new(&source)) {
            Ok(value) => {
                *self.value.borrow_mut() = Some(value);
                record(ctx, "expr", self.detail(), 1, start);
                FlowResult::Rows(Vec::new())
            }
            Err(err) => FlowResult::Error(PlannerError::Unsupported(err.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-row field computation (upstream Compute)
// ---------------------------------------------------------------------------

/// One materialized row: entity id plus computed field values.
#[derive(Debug, Clone)]
pub struct ComputedRow {
    pub uid: u64,
    pub fields: BTreeMap<String, QueryValue>,
}

/// Computes derived fields for every row of the input pipeline, passing the
/// input batches through unchanged so the operator can sit mid-pipeline.
pub struct ComputeOperator {
    pub input: Box<dyn ExecOperator>,
    /// (alias, expression) pairs evaluated against each row.
    pub fields: Vec<(String, Arc<dyn PhysicalExpr>)>,
    computed: RefCell<Vec<ComputedRow>>,
}

impl ComputeOperator {
    pub fn new(
        input: Box<dyn ExecOperator>,
        fields: Vec<(String, Arc<dyn PhysicalExpr>)>,
    ) -> Self {
        ComputeOperator {
            input,
            fields,
            computed: RefCell::new(Vec::new()),
        }
    }

    pub fn take_computed(&self) -> Vec<ComputedRow> {
        std::mem::take(&mut *self.computed.borrow_mut())
    }
}

impl ExecOperator for ComputeOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Project
    }
    fn detail(&self) -> String {
        let rendered = self
            .fields
            .iter()
            .map(|(alias, expr)| format!("{alias} = {}", expr.describe()))
            .collect::<Vec<_>>()
            .join(", ");
        format!("compute {rendered}")
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
        let start = std::time::Instant::now();
        let batches = match self.input.execute(ctx) {
            FlowResult::Rows(batches) => batches,
            other => return other,
        };

        let mut rows_out = 0usize;
        let mut computed = Vec::new();
        for batch in &batches {
            for id in &batch.0 {
                let mut fields = BTreeMap::new();
                {
                    let source = StoredSource::new(ctx.runtime, EntityId::new(id.uid));
                    let row_ctx = EvalContext::new(&source);
                    for (alias, expr) in &self.fields {
                        match expr.evaluate(&row_ctx) {
                            Ok(value) => {
                                fields.insert(alias.clone(), value);
                            }
                            Err(err) => {
                                return FlowResult::Error(PlannerError::Storage(format!(
                                    "compute {alias} failed for uid {}: {err}",
                                    id.uid
                                )));
                            }
                        }
                    }
                }
                computed.push(ComputedRow { uid: id.uid, fields });
                rows_out += 1;
            }
        }
        *self.computed.borrow_mut() = computed;
        record(ctx, "project", self.detail(), rows_out, start);
        FlowResult::Rows(batches)
    }
}

// ---------------------------------------------------------------------------
// Conditional value selection (upstream IfElsePlan)
// ---------------------------------------------------------------------------

/// First-true-wins conditional over scalar expressions.
pub struct IfElseOperator {
    pub branches: Vec<(Arc<dyn PhysicalExpr>, Arc<dyn PhysicalExpr>)>,
    pub else_body: Option<Arc<dyn PhysicalExpr>>,
    value: RefCell<Option<QueryValue>>,
}

impl IfElseOperator {
    pub fn new(
        branches: Vec<(Arc<dyn PhysicalExpr>, Arc<dyn PhysicalExpr>)>,
        else_body: Option<Arc<dyn PhysicalExpr>>,
    ) -> Self {
        IfElseOperator {
            branches,
            else_body,
            value: RefCell::new(None),
        }
    }

    pub fn take_value(&self) -> Option<QueryValue> {
        self.value.borrow_mut().take()
    }
}

impl ExecOperator for IfElseOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Control
    }
    fn detail(&self) -> String {
        format!(
            "ifelse branches={} else={}",
            self.branches.len(),
            self.else_body.is_some()
        )
    }
    fn cardinality(&self) -> super::CardinalityHint {
        super::CardinalityHint::AtMostOne
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        Vec::new()
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let source = EmptySource;
        let eval_ctx = EvalContext::new(&source);
        let selected = 'selected: {
            for (cond, body) in &self.branches {
                match cond.evaluate(&eval_ctx) {
                    Ok(QueryValue::Bool(true)) => {
                        break 'selected body.evaluate(&eval_ctx).map(Some);
                    }
                    Ok(QueryValue::Bool(false)) | Ok(QueryValue::Null) => continue,
                    Err(err) => return FlowResult::Error(PlannerError::Unsupported(err.to_string())),
                    Ok(other) => {
                        return FlowResult::Error(PlannerError::Unsupported(format!(
                            "ifelse condition must be bool, got {:?}",
                            other
                        )))
                    }
                }
            }
            match &self.else_body {
                Some(body) => body.evaluate(&eval_ctx).map(Some),
                None => Ok(Some(QueryValue::Null)),
            }
        };
        match selected {
            Ok(value) => {
                *self.value.borrow_mut() = value;
                record(ctx, "control", self.detail(), 1, start);
                FlowResult::Rows(Vec::new())
            }
            Err(err) => FlowResult::Error(PlannerError::Unsupported(err.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Iteration (upstream ForeachPlan)
// ---------------------------------------------------------------------------

/// Iterates a list-valued range expression, binding each element to `var`.
///
/// Body steps run in order per element; a step evaluating to `Bool(false)`
/// skips the remaining steps of that iteration (`continue`). When
/// `break_when` is set and evaluates to `Bool(true)` after an iteration
/// completes, the loop stops (`break`). The loop yields Null (upstream NONE).
pub struct ForeachOperator {
    pub range: Arc<dyn PhysicalExpr>,
    pub var: String,
    pub body: Vec<Arc<dyn PhysicalExpr>>,
    pub break_when: Option<Arc<dyn PhysicalExpr>>,
    iterations: RefCell<usize>,
}

impl ForeachOperator {
    pub fn new(range: Arc<dyn PhysicalExpr>, var: impl Into<String>, body: Vec<Arc<dyn PhysicalExpr>>) -> Self {
        ForeachOperator {
            range,
            var: var.into(),
            body,
            break_when: None,
            iterations: RefCell::new(0),
        }
    }

    pub fn with_break_when(mut self, cond: Arc<dyn PhysicalExpr>) -> Self {
        self.break_when = Some(cond);
        self
    }

    pub fn iterations(&self) -> usize {
        *self.iterations.borrow()
    }
}

impl ExecOperator for ForeachOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Control
    }
    fn detail(&self) -> String {
        format!("foreach ${} steps={} range={}", self.var, self.body.len(), self.range.describe())
    }
    fn cardinality(&self) -> super::CardinalityHint {
        super::CardinalityHint::AtMostOne
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        Vec::new()
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let source = EmptySource;
        let eval_ctx = EvalContext::new(&source);
        let range_value = match self.range.evaluate(&eval_ctx) {
            Ok(QueryValue::List(items)) => items,
            Ok(QueryValue::Null) => Vec::new(),
            Ok(other) => {
                return FlowResult::Error(PlannerError::Storage(format!(
                    "foreach range must be a list, got {:?}",
                    other
                )))
            }
            Err(err) => return FlowResult::Error(PlannerError::Unsupported(err.to_string())),
        };

        let mut ran = 0usize;
        for element in range_value {
            let bound = BoundSource::new(&EmptySource, self.var.clone(), element);
            let iter_ctx = EvalContext::new(&bound);
            let mut skip_rest = false;
            for step in &self.body {
                match step.evaluate(&iter_ctx) {
                    // Bool(false) acts as `continue`: skip remaining steps.
                    Ok(QueryValue::Bool(false)) => {
                        skip_rest = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(err) => return FlowResult::Error(PlannerError::Unsupported(err.to_string())),
                }
            }
            if skip_rest {
                continue;
            }
            if let Some(brk) = &self.break_when {
                match brk.evaluate(&iter_ctx) {
                    Ok(QueryValue::Bool(true)) => break,
                    Ok(_) => {}
                    Err(err) => return FlowResult::Error(PlannerError::Unsupported(err.to_string())),
                }
            }
            ran += 1;
        }
        *self.iterations.borrow_mut() = ran;

        record(ctx, "control", self.detail(), 1, start);
        FlowResult::Rows(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// Sequencing (upstream SequencePlan)
// ---------------------------------------------------------------------------

/// Runs child operators in order, accumulating their batches. A child
/// returning `Break` stops the sequence and propagates the signal alongside
/// everything accumulated so far is impossible in a single `FlowResult`, so
/// `Break` wins and accumulated rows are dropped (matching upstream's
/// terminal-signal semantics); `Continue` advances to the next step.
pub struct SequenceOperator {
    pub steps: Vec<Box<dyn ExecOperator>>,
}

impl SequenceOperator {
    pub fn new(steps: Vec<Box<dyn ExecOperator>>) -> Self {
        SequenceOperator { steps }
    }
}

impl ExecOperator for SequenceOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Control
    }
    fn cardinality(&self) -> super::CardinalityHint {
        self
            .steps
            .first()
            .map(|s| s.cardinality())
            .unwrap_or(super::CardinalityHint::Unbounded)
    }
    fn detail(&self) -> String {
        format!("sequence steps={}", self.steps.len())
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        self.steps.iter().map(|s| s.as_ref()).collect()
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let mut accumulated: Vec<RowBatch> = Vec::new();
        for step in &self.steps {
            match step.execute(ctx) {
                FlowResult::Rows(batches) => accumulated.extend(batches),
                FlowResult::Continue => continue,
                flow @ (FlowResult::Break | FlowResult::Error(_)) => return flow,
            }
        }
        record(ctx, "control", self.detail(), accumulated.len(), start);
        FlowResult::Rows(accumulated)
    }
}

// ---------------------------------------------------------------------------
// Terminal marker (upstream ReturnPlan)
// ---------------------------------------------------------------------------

/// Executes the inner operator exactly once, buffers its rows, and returns
/// the terminal [`FlowResult::Break`] signal. Retrieve the payload with
/// [`ReturnOperator::take_rows`].
pub struct ReturnOperator {
    pub inner: Box<dyn ExecOperator>,
    rows: RefCell<Option<Vec<RowBatch>>>,
}

impl ReturnOperator {
    pub fn new(inner: Box<dyn ExecOperator>) -> Self {
        ReturnOperator {
            inner,
            rows: RefCell::new(None),
        }
    }

    pub fn take_rows(&self) -> Option<Vec<RowBatch>> {
        self.rows.borrow_mut().take()
    }
}

impl ExecOperator for ReturnOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Control
    }
    fn cardinality(&self) -> super::CardinalityHint {
        self.inner.cardinality()
    }
    fn detail(&self) -> String {
        format!("return {}", self.inner.detail())
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![self.inner.as_ref()]
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        match self.inner.execute(ctx) {
            FlowResult::Rows(batches) => {
                let rows_out: usize = batches.iter().map(|b| b.0.len()).sum();
                *self.rows.borrow_mut() = Some(batches);
                record(ctx, "control", self.detail(), rows_out, start);
                FlowResult::Break
            }
            other => other,
        }
    }
}

fn record(ctx: &mut ExecContext, kind: &str, detail: String, rows_out: usize, start: std::time::Instant) {
    ctx.explain.record(OperatorStat {
        kind: kind.to_string(),
        detail,
        rows_in: 0,
        rows_out,
        elapsed_us: start.elapsed().as_micros() as u64,
        notes: Vec::new(),
    });
}
