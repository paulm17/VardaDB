//! Leaf source operators: every row-producing access path in the pipeline.
//!
//! Mirrors the upstream `exec/operators/scan/*` family plus union/set
//! composition. Sources are the only operators that talk to storage through
//! the [`PlannerRuntime`](crate::query_planner::traits::PlannerRuntime)
//! adapter; everything downstream consumes batches blindly.
//!
//! Ordering contracts preserved from the legacy resolver:
//! - `VectorKNNScan` / `HybridSearchScan` / `TextBM25Scan` produce results in
//!   relevance/distance order. They still *declare* [`OutputOrdering::Unordered`]
//!   because relevance is not expressible as a field-key ordering; pipeline
//!   builders must not insert re-sorts unless the user asked for one.
//! - `OrderedIndexScan` declares [`OutputOrdering::Sorted`] so downstream sorts
//!   on the same key are eliminated.
//! - Set-composition sources (`UnionSources`, `IntersectionSources`) emit
//!   deduplicated ascending-uid order, matching the legacy candidate-set
//!   behavior of sorting by uid when no explicit sort is requested.

use crate::query_planner::ir::{CursorValue, EntityId, FilterOp, FilterPredicate, QueryValue, SortDirection};
use crate::query_planner::lower_filter_map;
use crate::query_planner::operators::{
    CardinalityHint, ExecContext, ExecOperator, FilterOperator, FlowResult, OperatorKind,
    OperatorStat, OutputOrdering, PlannerError, RowBatch,
};
use crate::query_planner::plan::{CandidatePlan, CandidateSource};

fn record(ctx: &mut ExecContext, kind: &str, detail: String, rows_out: usize, start: std::time::Instant) {
    ctx.explain.record(OperatorStat {
        kind: kind.to_string(),
        detail,
        rows_in: 0,
        rows_out,
        elapsed_us: start.elapsed().as_micros() as u64,
        notes: vec![],
    });
}

/// Full type scan over the storage type-prefix index.
pub struct FullTypeScan {
    pub type_name: String,
    pub cursor: Option<CursorValue>,
    pub limit: Option<usize>,
}

impl FullTypeScan {
    pub fn new(type_name: impl Into<String>) -> Self {
        FullTypeScan {
            type_name: type_name.into(),
            cursor: None,
            limit: None,
        }
    }
}

impl ExecOperator for FullTypeScan {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!("full_type_scan type={}", self.type_name)
    }
    fn cardinality(&self) -> CardinalityHint {
        CardinalityHint::Unbounded
    }
    fn output_ordering(&self) -> OutputOrdering {
        // Type-prefix iteration is ascending by uid.
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let ids = match ctx.runtime.scan_type(&self.type_name, self.cursor.as_ref(), self.limit) {
            Ok(ids) => ids,
            Err(e) => return FlowResult::Error(PlannerError::Storage(e.to_string())),
        };
        let out = vec![RowBatch(ids)];
        let rows = out[0].len();
        record(ctx, "scan", self.detail(), rows, start);
        FlowResult::Rows(out)
    }
}

/// Unique-index equality lookup. At most one row; a miss is authoritative.
pub struct UniqueLookupSource {
    pub type_name: String,
    pub field: String,
    pub value: QueryValue,
}

impl ExecOperator for UniqueLookupSource {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!(
            "unique_lookup type={} field={} value={:?}",
            self.type_name, self.field, self.value
        )
    }
    fn cardinality(&self) -> CardinalityHint {
        CardinalityHint::AtMostOne
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let found = match ctx
            .runtime
            .lookup_unique(&self.type_name, &self.field, &self.value)
        {
            Ok(found) => found,
            Err(e) => return FlowResult::Error(PlannerError::Storage(e.to_string())),
        };
        let rows = found.into_iter().collect::<Vec<_>>();
        let out = vec![RowBatch(rows)];
        let n = out[0].len();
        record(ctx, "scan", self.detail(), n, start);
        FlowResult::Rows(out)
    }
}

/// SQL-pushdown predicate scan (`eq/ne/gt/ge/lt/le/in/contains` on scalar
/// fields). A `None` from the runtime means "this predicate cannot be pushed
/// down", surfaced as [`PlannerError::Unsupported`] so builders can substitute
/// a full scan plus residual filter.
pub struct PredicatePushdownSource {
    pub type_name: String,
    pub predicate: FilterPredicate,
}

impl ExecOperator for PredicatePushdownSource {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!(
            "predicate_pushdown type={} {} {} {:?}",
            self.type_name,
            self.predicate.path,
            self.predicate.op.as_str(),
            self.predicate.value
        )
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
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        match ctx.runtime.candidate_ids(&self.type_name, &self.predicate) {
            Ok(Some(mut ids)) => {
                // Legacy parity: candidate sets are emitted in ascending uid
                // order (the old candidates branch sorted the HashSet before
                // pagination). Set-based sources are the only ones allowed to
                // reorder like this; ordered scans keep their declared order.
                ids.sort_by_key(|id| id.uid);
                let out = vec![RowBatch(ids)];
                let n = out[0].len();
                record(ctx, "scan", self.detail(), n, start);
                FlowResult::Rows(out)
            }
            Ok(None) => {
                record(
                    ctx,
                    "scan",
                    format!("{} [unsupported]", self.detail()),
                    0,
                    start,
                );
                FlowResult::Error(PlannerError::Unsupported(format!(
                    "predicate not pushable: {} {:?}",
                    self.predicate.path, self.predicate.op
                )))
            }
            Err(e) => FlowResult::Error(PlannerError::Storage(e.to_string())),
        }
    }
}

/// BM25 text search over the FTS tables. Results arrive in relevance order.
pub struct TextBM25Scan {
    pub type_name: String,
    pub field: String,
    pub op: FilterOp,
    pub query: String,
    pub limit: Option<usize>,
}

impl TextBM25Scan {
    pub fn new(type_name: impl Into<String>, field: impl Into<String>, op: FilterOp, query: impl Into<String>) -> Self {
        TextBM25Scan {
            type_name: type_name.into(),
            field: field.into(),
            op,
            query: query.into(),
            limit: None,
        }
    }
}

impl ExecOperator for TextBM25Scan {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!(
            "text_bm25_scan type={} field={} op={} k={:?}",
            self.type_name,
            self.field,
            self.op.as_str(),
            self.limit
        )
    }
    fn cardinality(&self) -> CardinalityHint {
        match self.limit {
            Some(k) => CardinalityHint::Bounded(k),
            None => CardinalityHint::Unbounded,
        }
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let scored = match ctx
            .runtime
            .text_search(&self.type_name, &self.field, self.op.clone(), &self.query, self.limit)
        {
            Ok(scored) => scored,
            Err(e) => return FlowResult::Error(PlannerError::Storage(e.to_string())),
        };
        for (id, score) in &scored {
            ctx.scores.insert(id.uid, *score);
        }
        // Capture the FTS context so `_snippet` can render excerpts lazily.
        if ctx.snippet_ctx.is_none() {
            let (strategy, require_all) = match self.op {
                FilterOp::AllOfTerms => ("term", true),
                FilterOp::AnyOfTerms => ("term", false),
                FilterOp::AllOfText => ("fulltext", true),
                _ => ("fulltext", false),
            };
            if let Some(match_expr) =
                crate::bridge::fts_query::build_fts_match_query(&self.query, require_all)
            {
                let (table, plain) = match strategy {
                    "term" => ("fts_term_data", true),
                    "trigram" => ("fts_trigram_data", true),
                    _ => ("fts_data", false),
                };
                let index_field = if plain {
                    self.field.clone()
                } else {
                    format!("{}.{}", self.field, strategy)
                };
                ctx.snippet_ctx = Some(crate::engine::resolver::SnippetContext {
                    table,
                    index_field,
                    match_expr,
                });
            }
        }
        let rows = scored.into_iter().map(|(id, _score)| id).collect::<Vec<_>>();
        let out = vec![RowBatch(rows)];
        let n = out[0].len();
        record(ctx, "scan", self.detail(), n, start);
        FlowResult::Rows(out)
    }
}

/// Weighted multi-field text search: fuses several [`TextQuerySpec`]
/// rankings via weighted RRF (see
/// [`PlannerIndexAccess::text_search_weighted`](crate::query_planner::traits::PlannerIndexAccess::text_search_weighted)).
/// Output is fused-relevance order; scores land in the exec context.
pub struct MultiTextScan {
    pub type_name: String,
    pub specs: Vec<crate::query_planner::traits::TextQuerySpec>,
    pub limit: Option<usize>,
}

impl ExecOperator for MultiTextScan {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        let fields = self
            .specs
            .iter()
            .map(|s| format!("{}.{}^{}", s.field, s.strategy, s.boost))
            .collect::<Vec<_>>()
            .join(",");
        format!("multi_text_scan type={} specs=[{fields}]", self.type_name)
    }
    fn cardinality(&self) -> CardinalityHint {
        match self.limit {
            Some(k) => CardinalityHint::Bounded(k),
            None => CardinalityHint::Unbounded,
        }
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let scored = match ctx
            .runtime
            .text_search_weighted(&self.type_name, &self.specs, self.limit)
        {
            Ok(scored) => scored,
            Err(e) => return FlowResult::Error(PlannerError::Storage(e.to_string())),
        };
        for (id, score) in &scored {
            ctx.scores.insert(id.uid, *score);
        }
        let rows = scored.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        let out = vec![RowBatch(rows)];
        let n = out[0].len();
        record(ctx, "scan", self.detail(), n, start);
        FlowResult::Rows(out)
    }
}

/// K-nearest vector scan. Results arrive in ascending-distance order and MUST
/// NOT be reordered downstream unless the user explicitly sorted.
pub struct VectorKNNScan {
    pub type_name: String,
    pub field: String,
    pub query: Vec<f64>,
    pub limit: Option<usize>,
}

impl VectorKNNScan {
    pub fn new(type_name: impl Into<String>, query: Vec<f64>, limit: Option<usize>) -> Self {
        VectorKNNScan {
            type_name: type_name.into(),
            field: String::new(),
            query,
            limit,
        }
    }
}

impl ExecOperator for VectorKNNScan {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!(
            "vector_knn_scan type={} dims={} k={:?}",
            self.type_name,
            self.query.len(),
            self.limit
        )
    }
    fn cardinality(&self) -> CardinalityHint {
        match self.limit {
            Some(k) => CardinalityHint::Bounded(k),
            None => CardinalityHint::Unbounded,
        }
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let scored = match ctx.runtime.vector_search(
            &self.type_name,
            &self.field,
            &self.query,
            self.limit,
        ) {
            Ok(scored) => scored,
            Err(e) => return FlowResult::Error(PlannerError::Storage(e.to_string())),
        };
        // Distances are lower-better; normalize into the shared `_score`
        // convention (similarity, higher-better).
        for (id, dist) in &scored {
            ctx.scores.insert(id.uid, 1.0 / (1.0 + *dist));
        }
        let rows = scored.into_iter().map(|(id, _dist)| id).collect::<Vec<_>>();
        let out = vec![RowBatch(rows)];
        let n = out[0].len();
        record(ctx, "scan", self.detail(), n, start);
        FlowResult::Rows(out)
    }
}

/// Combined BM25 + cosine search (legacy `search_hybrid`). Distance/relevance
/// ordered, best first.
pub struct HybridSearchScan {
    pub type_name: String,
    pub field: String,
    pub text_query: String,
    pub require_all: bool,
    pub vector: Vec<f64>,
    pub limit: Option<usize>,
}

impl ExecOperator for HybridSearchScan {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!(
            "hybrid_scan type={} field={} require_all={} dims={} k={:?}",
            self.type_name,
            self.field,
            self.require_all,
            self.vector.len(),
            self.limit
        )
    }
    fn cardinality(&self) -> CardinalityHint {
        match self.limit {
            Some(k) => CardinalityHint::Bounded(k),
            None => CardinalityHint::Unbounded,
        }
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let scored = match ctx.runtime.hybrid_search(
            &self.type_name,
            &self.field,
            &self.text_query,
            self.require_all,
            &self.vector,
            self.limit,
        ) {
            Ok(scored) => scored,
            Err(e) => return FlowResult::Error(PlannerError::Storage(e.to_string())),
        };
        for (id, rrf_score) in &scored {
            ctx.scores.insert(id.uid, *rrf_score);
        }
        // The hybrid CTE matches the fulltext table; capture that context so
        // `_snippet` works on hybrid results too.
        if ctx.snippet_ctx.is_none() {
            if let Some(match_expr) =
                crate::bridge::fts_query::build_fts_match_query(&self.text_query, self.require_all)
            {
                ctx.snippet_ctx = Some(crate::engine::resolver::SnippetContext {
                    table: "fts_data",
                    index_field: format!("{}.fulltext", self.field),
                    match_expr,
                });
            }
        }
        let rows = scored.into_iter().map(|(id, _dist)| id).collect::<Vec<_>>();
        let out = vec![RowBatch(rows)];
        let n = out[0].len();
        record(ctx, "scan", self.detail(), n, start);
        FlowResult::Rows(out)
    }
}

/// Ordered index scan. Declares its ordering so downstream sorts on the same
/// key are eliminated. Falls back via [`PlannerError::Unsupported`] when the
/// order index cannot be materialized even after a rebuild attempt (legacy
/// `sorted_index_scan` returning `None`).
pub struct OrderedIndexScan {
    pub type_name: String,
    pub field: String,
    pub direction: SortDirection,
    pub cursor: Option<CursorValue>,
    pub limit: Option<usize>,
}

impl ExecOperator for OrderedIndexScan {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!(
            "ordered_index_scan type={} field={} dir={}",
            self.type_name,
            self.field,
            match self.direction {
                SortDirection::Asc => "asc",
                SortDirection::Desc => "desc",
            }
        )
    }
    fn cardinality(&self) -> CardinalityHint {
        CardinalityHint::Unbounded
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Sorted {
            field: self.field.clone(),
            direction: self.direction.clone(),
        }
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        match ctx.runtime.ordered_scan_with_fallback(
            &self.type_name,
            &self.field,
            self.direction.clone(),
            self.cursor.as_ref(),
            self.limit,
        ) {
            Ok(Some(ids)) => {
                let out = vec![RowBatch(ids)];
                let n = out[0].len();
                record(ctx, "scan", self.detail(), n, start);
                FlowResult::Rows(out)
            }
            Ok(None) => {
                record(
                    ctx,
                    "scan",
                    format!("{} [no_index]", self.detail()),
                    0,
                    start,
                );
                FlowResult::Error(PlannerError::Unsupported(format!(
                    "order index missing for {}.{}",
                    self.type_name, self.field
                )))
            }
            Err(e) => FlowResult::Error(PlannerError::Storage(e.to_string())),
        }
    }
}

/// Union of source outputs: concatenated, deduplicated by uid, emitted in
/// ascending-uid order.
pub struct UnionSources {
    pub sources: Vec<Box<dyn ExecOperator>>,
}

impl ExecOperator for UnionSources {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Union
    }
    fn detail(&self) -> String {
        format!("union sources={}", self.sources.len())
    }
    fn cardinality(&self) -> CardinalityHint {
        CardinalityHint::combine(self.sources.iter().map(|s| s.cardinality()))
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        self.sources.iter().map(|s| s.as_ref()).collect()
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<EntityId> = Vec::new();
        for source in &self.sources {
            match source.execute(ctx) {
                FlowResult::Rows(batches) => {
                    for batch in batches {
                        for id in batch.0 {
                            if seen.insert(id.uid) {
                                merged.push(id);
                            }
                        }
                    }
                }
                FlowResult::Continue => continue,
                other => return other.map_rows(|_| Vec::new()),
            }
        }
        merged.sort_by_key(|id| id.uid);
        let out = vec![RowBatch(merged)];
        let n = out[0].len();
        record(ctx, "union", self.detail(), n, start);
        FlowResult::Rows(out)
    }
}

/// Intersection of source outputs: rows present in every child, emitted in
/// ascending-uid order.
pub struct IntersectionSources {
    pub sources: Vec<Box<dyn ExecOperator>>,
}

impl IntersectionSources {
    /// Tightest child bound wins (result is a subset of every input).
    fn tightest(hints: impl IntoIterator<Item = CardinalityHint>) -> CardinalityHint {
        hints
            .into_iter()
            .fold(CardinalityHint::Unbounded, |acc, h| {
                let rank = |c: &CardinalityHint| match c {
                    CardinalityHint::AtMostOne => 0u8,
                    CardinalityHint::Bounded(_) => 1,
                    CardinalityHint::Unbounded => 2,
                };
                if rank(&h) < rank(&acc) {
                    h
                } else {
                    acc
                }
            })
    }
}

impl ExecOperator for IntersectionSources {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Union
    }
    fn detail(&self) -> String {
        format!("intersection sources={}", self.sources.len())
    }
    fn cardinality(&self) -> CardinalityHint {
        Self::tightest(self.sources.iter().map(|s| s.cardinality()))
    }
    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        self.sources.iter().map(|s| s.as_ref()).collect()
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let mut pulled = 0usize;
        let mut acc: Option<std::collections::HashSet<u64>> = None;
        for source in &self.sources {
            match source.execute(ctx) {
                FlowResult::Rows(batches) => {
                    let set: std::collections::HashSet<u64> = batches
                        .iter()
                        .flat_map(|b| b.0.iter().map(|e| e.uid))
                        .collect();
                    pulled += set.len();
                    acc = Some(match acc {
                        None => set,
                        Some(prev) => prev.intersection(&set).copied().collect(),
                    });
                }
                FlowResult::Continue => continue,
                other => return other.map_rows(|_| Vec::new()),
            }
        }
        let mut uids: Vec<EntityId> = acc
            .unwrap_or_default()
            .into_iter()
            .map(EntityId::from)
            .collect();
        uids.sort_by_key(|id| id.uid);
        let out = vec![RowBatch(uids)];
        let n = out[0].len();
        ctx.explain.record(OperatorStat {
            kind: "union".to_string(),
            detail: self.detail(),
            rows_in: pulled,
            rows_out: n,
            elapsed_us: start.elapsed().as_micros() as u64,
            notes: vec![],
        });
        FlowResult::Rows(out)
    }
}

trait FlowResultExt<T> {
    fn map_rows<U>(self, f: impl FnOnce(T) -> U) -> FlowResult<U>;
}

impl<T> FlowResultExt<T> for FlowResult<T> {
    fn map_rows<U>(self, f: impl FnOnce(T) -> U) -> FlowResult<U> {
        match self {
            FlowResult::Rows(t) => FlowResult::Rows(f(t)),
            FlowResult::Break => FlowResult::Break,
            FlowResult::Continue => FlowResult::Continue,
            FlowResult::Error(e) => FlowResult::Error(e),
        }
    }
}

/// Inverse edge expansion over an executed child pipeline: pulls child ids
/// from its input subtree, then maps them to parents through
/// [`PlannerRelations::reverse_related_ids`](crate::query_planner::traits::PlannerRelations).
/// Replaces the legacy recursive `scan_nodes_internal` re-entry inside
/// candidate generation (Stage 2.2). Emits deduplicated ascending-uid order,
/// matching the other set-composition sources.
pub struct RelationExpandSource {
    /// Child pipeline producing the nested-side rows.
    pub input: Box<dyn ExecOperator>,
    /// Type of the child side (used to resolve inverse edges).
    pub target_type: String,
    /// Field on child rows that stores parent references.
    pub inverse_field: String,
}

impl ExecOperator for RelationExpandSource {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!(
            "relation_expand target={} inverse_field={}",
            self.target_type, self.inverse_field
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
        let start = std::time::Instant::now();
        // Cycle guard: bounded nesting depth for pathological self-referential
        // plans (upstream `cycle_guard.rs` analog; full graph traversal lands
        // in Stage 3.3).
        if !ctx.enter_nested() {
            return FlowResult::Error(PlannerError::Unsupported(format!(
                "relation expansion nesting exceeds depth {}",
                ExecContext::MAX_DEPTH
            )));
        }
        let mut child_ids: Vec<u64> = Vec::new();
        let flow = match self.input.execute(ctx) {
            FlowResult::Rows(batches) => {
                for batch in batches {
                    for id in batch.0 {
                        child_ids.push(id.uid);
                    }
                }
                FlowResult::Rows(Vec::new())
            }
            FlowResult::Break => FlowResult::Rows(Vec::new()),
            FlowResult::Continue => FlowResult::Rows(Vec::new()),
            flow @ FlowResult::Error(_) => flow,
        };
        ctx.exit_nested();
        let flow = match flow {
            FlowResult::Error(e) => return FlowResult::Error(e),
            other => other,
        };

        let child_refs: Vec<EntityId> = child_ids.iter().map(|uid| EntityId::new(*uid)).collect();
        let mut parents: Vec<EntityId> =
            match ctx.runtime.reverse_related_ids(&self.target_type, &self.inverse_field, &child_refs) {
                Ok(list) => list,
                Err(err) => return FlowResult::Error(PlannerError::Storage(err.to_string())),
            };
        parents.sort_by_key(|e| e.uid);
        parents.dedup_by_key(|e| e.uid);

        let rows_in = child_ids.len();
        let rows_out = parents.len();
        ctx.explain.record(OperatorStat {
            kind: "scan".to_string(),
            detail: self.detail(),
            rows_in,
            rows_out,
            elapsed_us: start.elapsed().as_micros() as u64,
            notes: vec![format!(
                "expanded {} children via inverse field {}",
                rows_in, self.inverse_field
            )],
        });

        match flow {
            _ => FlowResult::Rows(vec![RowBatch(parents)]),
        }
    }
}

/// Compile a planner [`CandidateSource`] into an executable source-operator
/// tree for `type_name`. `RelationExpansion` composes its child subplan in
/// place (Stage 2.2): child source tree plus residual nested filter, wrapped
/// in inverse-edge expansion.
pub fn build_source_tree(
    type_name: &str,
    source: &CandidateSource,
) -> Result<Box<dyn ExecOperator>, PlannerError> {
    match source {
        CandidateSource::FullTypeScan => Ok(Box::new(FullTypeScan::new(type_name))),
        CandidateSource::UniqueLookup { field, value } => Ok(Box::new(UniqueLookupSource {
            type_name: type_name.to_string(),
            field: field.clone(),
            value: value.clone(),
        })),
        CandidateSource::OrderedIndexScan { field, direction } => Ok(Box::new(OrderedIndexScan {
            type_name: type_name.to_string(),
            field: field.clone(),
            direction: *direction,
            cursor: None,
            limit: None,
        })),
        CandidateSource::PredicatePushdown(pred) => Ok(Box::new(PredicatePushdownSource {
            type_name: type_name.to_string(),
            predicate: pred.clone(),
        })),
        CandidateSource::TextIndex { field, op, query } => Ok(Box::new(TextBM25Scan {
            type_name: type_name.to_string(),
            field: field.clone(),
            op: *op,
            query: query.clone(),
            limit: None,
        })),
        CandidateSource::VectorIndex { field, query } => Ok(Box::new(VectorKNNScan {
            type_name: type_name.to_string(),
            field: field.clone(),
            query: query.clone(),
            limit: None,
        })),
        CandidateSource::RelationExpansion {
            target_type,
            child_plan,
            inverse_field,
            child_raw_filter,
            ..
        } => {
            // Stage 2.2: compose the child subplan (its own source applies
            // text/pushdown narrowing) plus the residual nested filter, then
            // invert through the parent edge. Candidate-level execution uses
            // the same composition via `plan.rs`, keeping both paths aligned.
            let child_tree = build_source_tree(&child_plan.type_name, &child_plan.source)?;
            let node: Box<dyn ExecOperator> = match child_raw_filter
                .as_ref()
                .map(lower_filter_map)
                .filter(|filter| !filter.is_empty_conjunction())
            {
                Some(filter) => FilterOperator::boxed(child_tree, filter),
                None => child_tree,
            };
            Ok(Box::new(RelationExpandSource {
                input: node,
                target_type: target_type.clone(),
                inverse_field: inverse_field.clone(),
            }))
        }
        CandidateSource::Intersection(children) => {
            let sources = children
                .iter()
                .map(|c: &CandidatePlan| build_source_tree(&c.type_name, &c.source))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Box::new(IntersectionSources { sources }))
        }
        CandidateSource::Union(children) => {
            let sources = children
                .iter()
                .map(|c: &CandidatePlan| build_source_tree(&c.type_name, &c.source))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Box::new(UnionSources { sources }))
        }
    }
}

/// Materialized id set emitted as a single batch. Used by the pipeline builder
/// for sources that are resolved eagerly (relation-expansion parity bridge in
/// M5, ordered-index probe results). Can declare an ordering so downstream
/// sort elimination still applies.
pub struct VecSource {
    pub label: String,
    pub ids: Vec<EntityId>,
    pub ordering: OutputOrdering,
}

impl VecSource {
    pub fn new(label: impl Into<String>, ids: Vec<u64>) -> Self {
        VecSource {
            label: label.into(),
            ids: ids.into_iter().map(EntityId::from).collect(),
            ordering: OutputOrdering::Unordered,
        }
    }

    pub fn ordered(
        label: impl Into<String>,
        ids: Vec<u64>,
        field: impl Into<String>,
        direction: SortDirection,
    ) -> Self {
        VecSource {
            label: label.into(),
            ids: ids.into_iter().map(EntityId::from).collect(),
            ordering: OutputOrdering::Sorted {
                field: field.into(),
                direction,
            },
        }
    }
}

impl ExecOperator for VecSource {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!("vec_source {} n={}", self.label, self.ids.len())
    }
    fn cardinality(&self) -> CardinalityHint {
        CardinalityHint::Bounded(self.ids.len())
    }
    fn output_ordering(&self) -> OutputOrdering {
        self.ordering.clone()
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let out = vec![RowBatch(self.ids.clone())];
        record(ctx, "scan", self.detail(), out[0].len(), start);
        FlowResult::Rows(out)
    }
}
