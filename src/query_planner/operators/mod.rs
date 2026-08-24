//! Streaming operator pipeline for VardaDB read queries.
//!
//! This module is the simplified, synchronous analog of the upstream SurrealDB
//! `exec` pipeline contract:
//!
//! - `ExecOperator` mirrors the upstream `ExecOperator` trait: every operator
//!   declares its [`CardinalityHint`] and [`OutputOrdering`], exposes its child
//!   operators for explain-tree walking, and executes against an
//!   [`ExecContext`].
//! - `RowBatch` mirrors the upstream `ValueBatch` (`Vec<Value>`): operators
//!   consume and produce batches of rows instead of single rows. In Phase 2 a
//!   row is an [`EntityId`]; the batch type is the seam where columnar or full
//!   record payloads land later.
//! - [`FlowResult`] mirrors the upstream `FlowResult<T>` control-flow signal
//!   model: control-flow outcomes travel through the operator tree as values,
//!   never as panics or sentinel rows.
//! - Execution is a pull-based batch iterator tree: a parent pulls
//!   `Vec<RowBatch>` from its children. The upstream engine is push-based over
//!   async streams; the batching seams (`ValueBatch`, buffering hints,
//!   ordering declarations) are preserved so the shape ports cleanly.
//!
//! Design rules adopted from upstream `exec/CLAUDE.md`:
//! - no legacy compute() calls except through the plan_or_compute bridge
//! - access-mode / cardinality / ordering must be declared per operator
//! - sort elimination happens via [`OutputOrdering`] declarations

use crate::query_planner::ir::{EntityId, OrderKey, SortDirection};
use crate::query_planner::traits::PlannerRuntime;

pub mod filter;
pub mod pagination;
pub mod sort;
pub mod source;
pub use filter::{count_conditions, FilterOperator};
pub use pagination::{CursorSkipOperator, LimitOperator, OffsetOperator};
pub use sort::{compare_stored, SortOperator};
pub use source::{
    build_source_tree, FullTypeScan, HybridSearchScan, IntersectionSources, OrderedIndexScan,
    PredicatePushdownSource, TextBM25Scan, UniqueLookupSource, UnionSources, VectorKNNScan,
};

/// A batch of rows flowing between operators.
///
/// Phase 2 rows are entity IDs; materialization stays in the resolver layer
/// exactly as it does today. This is the `Vec<Value>` batch analog.
#[derive(Debug, Clone, Default)]
pub struct RowBatch(pub Vec<EntityId>);

impl RowBatch {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<EntityId>> for RowBatch {
    fn from(rows: Vec<EntityId>) -> Self {
        RowBatch(rows)
    }
}

/// Errors surfaced by operator execution.
#[derive(Debug, Clone)]
pub enum PlannerError {
    /// The planner/runtime cannot execute this shape; callers should fall back
    /// to the legacy path (plan_or_compute bridge).
    Unsupported(String),
    /// Storage or runtime failure during execution.
    Storage(String),
}

impl std::fmt::Display for PlannerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlannerError::Unsupported(m) => write!(f, "planner unsupported: {m}"),
            PlannerError::Storage(m) => write!(f, "storage error: {m}"),
        }
    }
}

impl std::error::Error for PlannerError {}

/// Control-flow signal returned by [`ExecOperator::execute`].
///
/// Upstream model: `return`/`break`/`continue`/errors propagate through the
/// operator tree as `FlowResult<T>` values. Here `Rows` is the normal output,
/// `Break` short-circuits a pipeline (e.g. limit satisfied upstream),
/// `Continue` skips the current unit of work without aborting, and `Error`
/// carries a failure with fallback semantics attached.
#[derive(Debug)]
pub enum FlowResult<T> {
    Rows(T),
    Break,
    Continue,
    Error(PlannerError),
}

impl<T> FlowResult<T> {
    /// Unwraps normal output; `Break`/`Continue` degrade to empty output so
    /// simple pipelines can ignore control flow until Stage 3.4 lands.
    pub fn into_rows(self) -> Option<T> {
        match self {
            FlowResult::Rows(t) => Some(t),
            _ => None,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, FlowResult::Error(_))
    }
}

impl<T> From<Result<T, PlannerError>> for FlowResult<T> {
    fn from(r: Result<T, PlannerError>) -> Self {
        match r {
            Ok(t) => FlowResult::Rows(t),
            Err(e) => FlowResult::Error(e),
        }
    }
}

/// Declared row-count hint for an operator's output.
///
/// Mirrors the upstream `CardinalityHint` (`AtMostOne | Bounded(n) |
/// Unbounded`) which drives upstream buffering decisions (`buffer.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardinalityHint {
    AtMostOne,
    Bounded(usize),
    Unbounded,
}

impl CardinalityHint {
    pub fn is_at_most_one(&self) -> bool {
        matches!(self, CardinalityHint::AtMostOne)
    }

    /// Combine hints across children of one operator: the parent inherits the
    /// loosest bound among its inputs.
    pub fn combine(hints: impl IntoIterator<Item = CardinalityHint>) -> CardinalityHint {
        hints.into_iter().fold(CardinalityHint::AtMostOne, |acc, h| match (acc, h) {
            (CardinalityHint::Unbounded, _) | (_, CardinalityHint::Unbounded) => {
                CardinalityHint::Unbounded
            }
            (CardinalityHint::Bounded(a), CardinalityHint::Bounded(b)) => {
                CardinalityHint::Bounded(a.saturating_add(b))
            }
            (a, b) if a.is_at_most_one() => b,
            (a, _) => a,
        })
    }

    /// Suggested pre-allocation capacity when collecting this operator's
    /// output, mirroring the role of upstream `buffer.rs`.
    pub fn suggested_capacity(&self) -> Option<usize> {
        match self {
            CardinalityHint::AtMostOne => Some(1),
            CardinalityHint::Bounded(n) => Some(*n),
            CardinalityHint::Unbounded => None,
        }
    }
}

/// Declared output ordering of an operator.
///
/// Mirrors the upstream `OutputOrdering` (`Unordered | Sorted`). Downstream
/// sorts are eliminated when the requested keys are already satisfied by the
/// input ordering declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputOrdering {
    Unordered,
    Sorted {
        field: String,
        direction: SortDirection,
    },
}

impl Default for OutputOrdering {
    fn default() -> Self {
        OutputOrdering::Unordered
    }
}

impl OutputOrdering {
    /// True when this declared ordering already satisfies the requested key,
    /// enabling downstream sort elimination.
    pub fn satisfies(&self, key: &OrderKey) -> bool {
        let first_segment_is = |field: &str, path: &crate::query_planner::ir::FieldPath| {
            path.segments
                .first()
                .map(|s| s == &crate::query_planner::ir::FieldSegment::Field(field.to_string()))
                .unwrap_or(false)
        };
        match self {
            OutputOrdering::Sorted {
                field,
                direction,
            } => first_segment_is(field, &key.path) && *direction == key.direction,
            OutputOrdering::Unordered => false,
        }
    }

    pub fn satisfies_all(&self, keys: &[OrderKey]) -> bool {
        keys.first().map(|k| self.satisfies(k)).unwrap_or(true)
    }
}

/// Statistics captured per operator invocation for explain output.
#[derive(Debug, Clone, Default)]
pub struct OperatorStat {
    pub kind: String,
    pub detail: String,
    pub rows_in: usize,
    pub rows_out: usize,
    pub elapsed_us: u64,
    pub notes: Vec<String>,
}

/// Collects per-operator execution statistics.
///
/// Stub for M8: today it records stats and hands them to debug logs; the M8
/// milestone turns it into structured text/JSON explain trees.
#[derive(Debug, Default)]
pub struct ExplainCapture {
    enabled: bool,
    stats: Vec<OperatorStat>,
}

impl ExplainCapture {
    pub fn new(enabled: bool) -> Self {
        ExplainCapture {
            enabled,
            stats: Vec::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn record(&mut self, stat: OperatorStat) {
        if self.enabled {
            self.stats.push(stat);
        }
    }

    pub fn stats(&self) -> &[OperatorStat] {
        &self.stats
    }

    pub fn take_stats(&mut self) -> Vec<OperatorStat> {
        std::mem::take(&mut self.stats)
    }
}

/// Per-execution context threaded through the operator tree.
///
/// Holds the adapter-backed runtime, the target database name, the explain
/// capture, and a recursion depth guard (used from M7 onward for cycle
/// protection, mirroring the upstream `cycle_guard.rs` concept).
pub struct ExecContext<'a> {
    pub runtime: &'a dyn PlannerRuntime,
    pub db_name: &'a str,
    pub explain: ExplainCapture,
    depth: usize,
}

impl<'a> ExecContext<'a> {
    pub fn new(runtime: &'a dyn PlannerRuntime, db_name: &'a str) -> Self {
        let enabled = crate::debug_logging();
        ExecContext {
            runtime,
            db_name,
            explain: ExplainCapture::new(enabled),
            depth: 0,
        }
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub const MAX_DEPTH: usize = 64;

    /// Enter a nested subplan scope; returns false when the recursion budget
    /// is exhausted (cycle protection for nested relation planning).
    pub fn enter_nested(&mut self) -> bool {
        if self.depth >= Self::MAX_DEPTH {
            return false;
        }
        self.depth += 1;
        true
    }

    pub fn exit_nested(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

/// Operator kinds, kept as an enum for cheap matching and metric labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorKind {
    Scan,
    Filter,
    Sort,
    Limit,
    Project,
    Fetch,
    Union,
    Aggregate,
    Explain,
}

impl OperatorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperatorKind::Scan => "scan",
            OperatorKind::Filter => "filter",
            OperatorKind::Sort => "sort",
            OperatorKind::Limit => "limit",
            OperatorKind::Project => "project",
            OperatorKind::Fetch => "fetch",
            OperatorKind::Union => "union",
            OperatorKind::Aggregate => "aggregate",
            OperatorKind::Explain => "explain",
        }
    }
}

/// The core operator trait, structurally derived from the upstream
/// `ExecOperator`: declare metadata, expose children, execute in batches.
pub trait ExecOperator {
    fn kind(&self) -> OperatorKind;

    /// Human-readable detail line for explain output (e.g. the scan source).
    fn detail(&self) -> String;

    fn cardinality(&self) -> CardinalityHint;

    fn output_ordering(&self) -> OutputOrdering;

    fn children(&self) -> Vec<&dyn ExecOperator>;

    /// Execute this operator, producing zero or more row batches.
    ///
    /// Pull-based: parents call this on children and merge batches. Operators
    /// that need per-batch processing should iterate the child batches rather
    /// than concatenating eagerly, to preserve the streaming seams.
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(start: u64, count: usize) -> Vec<EntityId> {
        (start..start + count as u64)
            .map(EntityId::from)
            .collect()
    }

    /// Mock source operator producing fixed batches.
    struct MockSource {
        batches: Vec<RowBatch>,
    }

    impl ExecOperator for MockSource {
        fn kind(&self) -> OperatorKind {
            OperatorKind::Scan
        }
        fn detail(&self) -> String {
            "mock_source".to_string()
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
            FlowResult::Rows(self.batches.clone())
        }
    }

    /// Passthrough operator recording stats, exercising child pulls.
    struct Passthrough {
        input: MockSource,
    }

    impl ExecOperator for Passthrough {
        fn kind(&self) -> OperatorKind {
            OperatorKind::Filter
        }
        fn detail(&self) -> String {
            "passthrough".to_string()
        }
        fn cardinality(&self) -> CardinalityHint {
            self.input.cardinality()
        }
        fn output_ordering(&self) -> OutputOrdering {
            self.input.output_ordering()
        }
        fn children(&self) -> Vec<&dyn ExecOperator> {
            vec![&self.input]
        }
        fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
            let start = std::time::Instant::now();
            let child = self.input.execute(ctx);
            match child {
                FlowResult::Rows(batches) => {
                    let rows_in: usize = batches.iter().map(|b| b.len()).sum();
                    let rows_out = rows_in;
                    ctx.explain.record(OperatorStat {
                        kind: self.kind().as_str().to_string(),
                        detail: self.detail(),
                        rows_in,
                        rows_out,
                        elapsed_us: start.elapsed().as_micros() as u64,
                        notes: vec![],
                    });
                    FlowResult::Rows(batches)
                }
                other => other,
            }
        }
    }

    #[test]
    fn batch_chain_flattens_rows() {
        let op = Passthrough {
            input: MockSource {
                batches: vec![
                    RowBatch(ids(1, 3)),
                    RowBatch(ids(10, 2)),
                ],
            },
        };
        let runtime = crate::query_planner::adapters::runtime_for_test_stub();
        let mut ctx = ExecContext::new(&runtime, "test_db");
        ctx.explain = ExplainCapture::new(true);
        match op.execute(&mut ctx) {
            FlowResult::Rows(batches) => {
                let all: Vec<u64> = batches.iter().flat_map(|b| b.0.iter().map(|e| e.uid)).collect();
                assert_eq!(all, vec![1, 2, 3, 10, 11]);
            }
            other => panic!("expected rows, got {:?}", other.is_error()),
        }
        assert_eq!(ctx.explain.stats().len(), 1);
        assert_eq!(ctx.explain.stats()[0].rows_in, 5);
    }

    #[test]
    fn explain_capture_disabled_records_nothing() {
        let op = Passthrough {
            input: MockSource {
                batches: vec![RowBatch(ids(7, 1))],
            },
        };
        let runtime = crate::query_planner::adapters::runtime_for_test_stub();
        let mut ctx = ExecContext::new(&runtime, "test_db");
        ctx.explain = ExplainCapture::new(false);
        let _ = op.execute(&mut ctx).into_rows().unwrap();
        assert!(ctx.explain.stats().is_empty());
    }

    #[test]
    fn cardinality_hints_combine_to_loosest_bound() {
        assert_eq!(
            CardinalityHint::combine([CardinalityHint::Bounded(2), CardinalityHint::Bounded(3)]),
            CardinalityHint::Bounded(5)
        );
        assert_eq!(
            CardinalityHint::combine([CardinalityHint::AtMostOne, CardinalityHint::Bounded(4)]),
            CardinalityHint::Bounded(4)
        );
        assert_eq!(
            CardinalityHint::combine([CardinalityHint::Unbounded, CardinalityHint::AtMostOne]),
            CardinalityHint::Unbounded
        );
        assert_eq!(CardinalityHint::AtMostOne.suggested_capacity(), Some(1));
        assert_eq!(CardinalityHint::Bounded(9).suggested_capacity(), Some(9));
        assert_eq!(CardinalityHint::Unbounded.suggested_capacity(), None);
    }

    #[test]
    fn output_ordering_satisfies_requested_keys() {
        use crate::query_planner::ir::{FieldPath, FieldSegment};

        let path = |name: &str| FieldPath {
            segments: vec![FieldSegment::Field(name.to_string())],
        };
        let sorted_asc = OutputOrdering::Sorted {
            field: "age".to_string(),
            direction: SortDirection::Asc,
        };

        // Exact match eliminates a downstream sort.
        assert!(sorted_asc.satisfies(&OrderKey {
            path: path("age"),
            direction: SortDirection::Asc,
        }));
        // Direction mismatch does not.
        assert!(!sorted_asc.satisfies(&OrderKey {
            path: path("age"),
            direction: SortDirection::Desc,
        }));
        // Different field does not.
        assert!(!sorted_asc.satisfies(&OrderKey {
            path: path("name"),
            direction: SortDirection::Asc,
        }));
        // Unordered never satisfies.
        assert!(!OutputOrdering::Unordered.satisfies(&OrderKey {
            path: path("age"),
            direction: SortDirection::Asc,
        }));
        // Empty key list is trivially satisfied.
        assert!(sorted_asc.satisfies_all(&[]));
    }

    #[test]
    fn flow_result_control_flow_semantics() {
        let ok: FlowResult<Vec<RowBatch>> = FlowResult::Rows(vec![]);
        assert!(!ok.is_error());
        assert!(ok.into_rows().is_some());

        let brk: FlowResult<Vec<RowBatch>> = FlowResult::Break;
        assert!(brk.into_rows().is_none());

        let err: FlowResult<Vec<RowBatch>> =
            FlowResult::Error(PlannerError::Unsupported("geo".into()));
        assert!(err.is_error());
        assert!(matches!(Err::<(), _>(PlannerError::Storage("x".into())).into(), FlowResult::Error(_)));
    }

    #[test]
    fn exec_context_depth_guard_blocks_runaway_recursion() {
        let runtime = crate::query_planner::adapters::runtime_for_test_stub();
        let mut ctx = ExecContext::new(&runtime, "test_db");
        for _ in 0..ExecContext::MAX_DEPTH {
            assert!(ctx.enter_nested());
        }
        assert!(!ctx.enter_nested());
        ctx.exit_nested();
        assert!(ctx.enter_nested());
    }
}
