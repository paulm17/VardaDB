//! Pagination operators: positional cursor skip, offset, and limit.
//!
//! These mirror the legacy tail of `scan_nodes_internal` exactly:
//!
//! - cursor skip is positional: find the `after` uid in the materialized row
//!   sequence and keep everything after it; an absent uid keeps the sequence
//!   unchanged (legacy behavior).
//! - offset skips N rows after the cursor skip.
//! - limit truncates. The pipeline builder orders these
//!   Source -> Filter -> Sort -> CursorSkip -> Offset -> Limit, matching the
//!   legacy application order.

use super::{
    CardinalityHint, ExecContext, ExecOperator, FlowResult, OperatorKind, OperatorStat,
    OutputOrdering, RowBatch,
};
use crate::query_planner::ir::EntityId;

/// Cursor handling mode.
///
/// Legacy VardaDB has two observably different cursor behaviors:
///
/// - `KeepAllIfAbsent`: the unsorted streaming/candidates tail of
///   `scan_nodes_internal` finds the cursor positionally and keeps every row
///   when the cursor uid is not present.
/// - `EmptyIfAbsent`: the sorted-order-index fast path (`sorted_index_scan`)
///   collects only rows strictly after the cursor in index order, producing
///   nothing when the cursor never appears.
///
/// Both agree when the cursor exists: emit the rows strictly after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMode {
    KeepAllIfAbsent,
    EmptyIfAbsent,
}

/// Positional cursor skip over the materialized row sequence.
pub struct CursorSkipOperator {
    input: Box<dyn ExecOperator>,
    after: Option<u64>,
    mode: CursorMode,
}

impl CursorSkipOperator {
    /// Tail-path semantics: an absent cursor keeps the whole sequence.
    pub fn new(input: Box<dyn ExecOperator>, after: Option<u64>) -> Self {
        CursorSkipOperator {
            input,
            after,
            mode: CursorMode::KeepAllIfAbsent,
        }
    }

    pub fn boxed(input: Box<dyn ExecOperator>, after: Option<u64>) -> Box<dyn ExecOperator> {
        Box::new(CursorSkipOperator::new(input, after))
    }

    /// Sorted-seek semantics (`sorted_index_scan` parity): an absent cursor
    /// yields nothing. Use when the input's declared ordering comes from an
    /// ordered index scan.
    pub fn seek(input: Box<dyn ExecOperator>, after: Option<u64>) -> Self {
        CursorSkipOperator {
            input,
            after,
            mode: CursorMode::EmptyIfAbsent,
        }
    }

    pub fn seek_boxed(input: Box<dyn ExecOperator>, after: Option<u64>) -> Box<dyn ExecOperator> {
        Box::new(CursorSkipOperator::seek(input, after))
    }
}

/// Skips the first `offset` rows of the accumulated stream.
pub struct OffsetOperator {
    input: Box<dyn ExecOperator>,
    offset: usize,
}

impl OffsetOperator {
    pub fn new(input: Box<dyn ExecOperator>, offset: usize) -> Self {
        OffsetOperator { input, offset }
    }

    pub fn boxed(input: Box<dyn ExecOperator>, offset: usize) -> Box<dyn ExecOperator> {
        Box::new(OffsetOperator { input, offset })
    }
}

/// Truncates the output to at most `limit` rows.
pub struct LimitOperator {
    input: Box<dyn ExecOperator>,
    limit: usize,
}

impl LimitOperator {
    pub fn new(input: Box<dyn ExecOperator>, limit: usize) -> Self {
        LimitOperator { input, limit }
    }

    pub fn boxed(input: Box<dyn ExecOperator>, limit: usize) -> Box<dyn ExecOperator> {
        Box::new(LimitOperator { input, limit })
    }
}

impl CursorSkipOperator {
    fn execute_inner(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let child = self.input.execute(ctx);
        let batches = match child {
            FlowResult::Rows(b) => b,
            other => return other,
        };

        let rows_in: usize = batches.iter().map(|b| b.len()).sum();
        let mut rows: Vec<EntityId> = batches.into_iter().flat_map(|b| b.0).collect();

        let mut notes = Vec::new();
        if let Some(after_uid) = self.after {
            match rows.iter().position(|id| id.uid == after_uid) {
                Some(pos) => {
                    notes.push(format!("cursor after={after_uid} found at pos {pos}"));
                    rows.drain(..=pos);
                }
                None if self.mode == CursorMode::EmptyIfAbsent => {
                    notes.push(format!(
                        "cursor after={after_uid} absent in ordered stream; yielding none"
                    ));
                    rows.clear();
                }
                None => {
                    notes.push(format!("cursor after={after_uid} not found; kept all"));
                }
            }
        } else {
            notes.push("no cursor".to_string());
        }
        let rows_out = rows.len();

        ctx.explain.record(OperatorStat {
            kind: self.kind().as_str().to_string(),
            detail: self.detail(),
            rows_in,
            rows_out,
            elapsed_us: start.elapsed().as_micros() as u64,
            notes,
        });
        FlowResult::Rows(vec![RowBatch(rows)])
    }
}

impl OffsetOperator {
    fn execute_inner(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let child = self.input.execute(ctx);
        let batches = match child {
            FlowResult::Rows(b) => b,
            other => return other,
        };

        let rows_in: usize = batches.iter().map(|b| b.len()).sum();
        let mut rows: Vec<EntityId> = batches.into_iter().flat_map(|b| b.0).collect();
        let skip = self.offset.min(rows.len());
        rows.drain(..skip);
        let rows_out = rows.len();

        ctx.explain.record(OperatorStat {
            kind: self.kind().as_str().to_string(),
            detail: self.detail(),
            rows_in,
            rows_out,
            elapsed_us: start.elapsed().as_micros() as u64,
            notes: vec![format!("skipped {skip}")],
        });
        FlowResult::Rows(vec![RowBatch(rows)])
    }
}

impl LimitOperator {
    fn execute_inner(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let child = self.input.execute(ctx);
        let batches = match child {
            FlowResult::Rows(b) => b,
            other => return other,
        };

        let rows_in: usize = batches.iter().map(|b| b.len()).sum();
        let mut rows: Vec<EntityId> = batches.into_iter().flat_map(|b| b.0).collect();
        rows.truncate(self.limit);
        let rows_out = rows.len();

        ctx.explain.record(OperatorStat {
            kind: self.kind().as_str().to_string(),
            detail: self.detail(),
            rows_in,
            rows_out,
            elapsed_us: start.elapsed().as_micros() as u64,
            notes: vec![format!("kept {rows_out} of {rows_in}")],
        });
        FlowResult::Rows(vec![RowBatch(rows)])
    }
}

impl ExecOperator for CursorSkipOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Limit
    }

    fn detail(&self) -> String {
        format!(
            "cursor_skip(after={:?}, mode={:?})",
            self.after, self.mode
        )
    }

    // Windowing never reorders; cursor/offset cannot tighten the declared
    // row-count bound, so both metadata declarations inherit from input.
    fn cardinality(&self) -> CardinalityHint {
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

impl ExecOperator for OffsetOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Limit
    }

    fn detail(&self) -> String {
        format!("offset({})", self.offset)
    }

    fn cardinality(&self) -> CardinalityHint {
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

impl ExecOperator for LimitOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Limit
    }

    fn detail(&self) -> String {
        format!("limit({})", self.limit)
    }

    // Limit tightens any declared bound; AtMostOne survives unless limit is 0.
    fn cardinality(&self) -> CardinalityHint {
        match self.input.cardinality() {
            CardinalityHint::AtMostOne if self.limit >= 1 => CardinalityHint::AtMostOne,
            CardinalityHint::Bounded(n) => CardinalityHint::Bounded(n.min(self.limit)),
            _ => CardinalityHint::Bounded(self.limit),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::adapters::runtime_for_test_stub;

    struct FixedSource(Vec<u64>);

    impl ExecOperator for FixedSource {
        fn kind(&self) -> OperatorKind {
            OperatorKind::Scan
        }
        fn detail(&self) -> String {
            "fixed".to_string()
        }
        fn cardinality(&self) -> CardinalityHint {
            CardinalityHint::Unbounded
        }
        fn output_ordering(&self) -> OutputOrdering {
            OutputOrdering::Unordered
        }
        fn children(&self) -> Vec<&dyn ExecOperator> {
            vec![]
        }
        fn execute(&self, _ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
            FlowResult::Rows(vec![RowBatch(
                self.0.iter().copied().map(EntityId::from).collect(),
            )])
        }
    }

    fn run(op: &dyn ExecOperator) -> Vec<u64> {
        let runtime = runtime_for_test_stub();
        let mut ctx = ExecContext::new(&runtime, "db");
        match op.execute(&mut ctx) {
            FlowResult::Rows(batches) => batches
                .into_iter()
                .flat_map(|b| b.0.into_iter().map(|e| e.uid))
                .collect(),
            other => panic!("expected rows, got error={}", other.is_error()),
        }
    }

    #[test]
    fn cursor_skip_is_positional_and_tolerates_absent_cursor() {
        let src = || Box::new(FixedSource(vec![3, 1, 4, 1, 5]));
        assert_eq!(
            run(&CursorSkipOperator::new(src(), Some(4))),
            vec![1, 5],
            "keeps rows strictly after the matched position"
        );
        assert_eq!(
            run(&CursorSkipOperator::new(src(), Some(99))),
            vec![3, 1, 4, 1, 5],
            "absent cursor keeps everything (legacy parity)"
        );
        assert_eq!(run(&CursorSkipOperator::new(src(), None)), vec![3, 1, 4, 1, 5]);
    }

    #[test]
    fn offset_skips_then_limit_truncates() {
        let src = || Box::new(FixedSource(vec![1, 2, 3, 4, 5])) as Box<dyn ExecOperator>;
        assert_eq!(run(&OffsetOperator::new(src(), 2)), vec![3, 4, 5]);
        assert_eq!(run(&LimitOperator::new(src(), 2)), vec![1, 2]);
        assert_eq!(run(&LimitOperator::new(src(), 0)), Vec::<u64>::new());
        assert_eq!(run(&OffsetOperator::new(src(), 9)), Vec::<u64>::new());

        let combined =
            LimitOperator::boxed(
                OffsetOperator::boxed(Box::new(FixedSource(vec![1, 2, 3, 4, 5])), 1),
                2,
            );
        assert_eq!(run(&*combined), vec![2, 3]);
    }

    #[test]
    fn limit_narrows_declared_cardinality() {
        use super::CardinalityHint as C;

        // Unbounded input becomes Bounded(limit).
        let unbounded = LimitOperator::new(Box::new(FixedSource(vec![])), 3);
        assert_eq!(unbounded.cardinality(), C::Bounded(3));

        // A tighter declared bound shrinks to min(input, limit).
        struct BoundedSource;
        impl ExecOperator for BoundedSource {
            fn kind(&self) -> OperatorKind {
                OperatorKind::Scan
            }
            fn detail(&self) -> String {
                "bounded".to_string()
            }
            fn cardinality(&self) -> CardinalityHint {
                C::Bounded(5)
            }
            fn output_ordering(&self) -> OutputOrdering {
                OutputOrdering::Unordered
            }
            fn children(&self) -> Vec<&dyn ExecOperator> {
                vec![]
            }
            fn execute(&self, _ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
                FlowResult::Rows(vec![])
            }
        }
        assert_eq!(
            LimitOperator::new(Box::new(BoundedSource), 2).cardinality(),
            C::Bounded(2)
        );
        assert_eq!(
            LimitOperator::new(Box::new(BoundedSource), 9).cardinality(),
            C::Bounded(5)
        );
    }
}
