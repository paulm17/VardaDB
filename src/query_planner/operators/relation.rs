//! Relation traversal operators (Stage 2.2a).
//!
//! Mirrors the legacy `resolve_list_internal` base stages:
//! - [`RelatedIdsSource`] replaces `related_uids_cached`: edge-derived child
//!   ids for one parent, emitted in edge-key order;
//! - [`CosineRerankOperator`] replaces the legacy near-vector branch: children
//!   whose stored embedding is missing or dimension-mismatched are dropped,
//!   distance is `1 - cosine_similarity`, zero-norm vectors sort last, and the
//!   survivors are stable-sorted ascending by distance.

use async_graphql::Value;

use crate::query_planner::ir::EntityId;
use crate::query_planner::operators::{
    CardinalityHint, ExecContext, ExecOperator, FlowResult, OperatorKind, OperatorStat,
    OutputOrdering, PlannerError, RowBatch,
};

fn record(
    ctx: &mut ExecContext,
    kind: &str,
    detail: String,
    rows_in: usize,
    rows_out: usize,
    start: std::time::Instant,
    notes: Vec<String>,
) {
    ctx.explain.record(OperatorStat {
        kind: kind.to_string(),
        detail,
        rows_in,
        rows_out,
        elapsed_us: start.elapsed().as_micros() as u64,
        notes,
    });
}

/// Edge-derived child ids for a single parent relation field.
pub struct RelatedIdsSource {
    pub parent: EntityId,
    pub field: String,
}

impl RelatedIdsSource {
    pub fn new(parent_uid: u64, field: impl Into<String>) -> Self {
        RelatedIdsSource {
            parent: EntityId::from(parent_uid),
            field: field.into(),
        }
    }
}

impl ExecOperator for RelatedIdsSource {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Scan
    }
    fn detail(&self) -> String {
        format!("related_ids parent={} field={}", self.parent.uid, self.field)
    }
    fn cardinality(&self) -> CardinalityHint {
        CardinalityHint::Unbounded
    }
    fn output_ordering(&self) -> OutputOrdering {
        // Edge-prefix iteration order; not expressible as a field ordering.
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let ids = match ctx.runtime.related_ids(&self.parent, &self.field, None, None) {
            Ok(ids) => ids,
            Err(e) => return FlowResult::Error(PlannerError::Storage(e.to_string())),
        };
        let rows = ids.len();
        record(ctx, "scan", self.detail(), 0, rows, start, vec![]);
        FlowResult::Rows(vec![RowBatch(ids)])
    }
}

/// Cosine-distance re-rank of the incoming stream against a query vector.
///
/// Legacy parity notes:
/// - rows without a stored embedding list, or with a dimension mismatch, are
///   dropped entirely (they never appear downstream);
/// - distance = `1 - dot/(norm_a*norm_b)`; zero-norm vectors get `f64::MAX`;
/// - the survivor order is a stable ascending sort by distance.
pub struct CosineRerankOperator {
    pub input: Box<dyn ExecOperator>,
    pub query: Vec<f64>,
    pub embedding_field: String,
}

impl CosineRerankOperator {
    pub fn boxed(input: Box<dyn ExecOperator>, query: Vec<f64>) -> Box<dyn ExecOperator> {
        Box::new(CosineRerankOperator {
            input,
            query,
            embedding_field: "embedding".to_string(),
        })
    }
}

fn embedded_floats(stored: Option<Value>, dim: usize) -> Option<Vec<f64>> {
    let floats = match stored? {
        Value::List(items) => items,
        _ => return None,
    };
    let embed: Vec<f64> = floats
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => n.as_f64(),
            _ => None,
        })
        .collect();
    if embed.len() == dim {
        Some(embed)
    } else {
        None
    }
}

impl ExecOperator for CosineRerankOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Sort
    }
    fn detail(&self) -> String {
        format!(
            "cosine_rerank field={} dim={}",
            self.embedding_field,
            self.query.len()
        )
    }
    fn cardinality(&self) -> CardinalityHint {
        self.input.cardinality()
    }
    fn output_ordering(&self) -> OutputOrdering {
        // Distance order is not expressible as a field-key ordering.
        OutputOrdering::Unordered
    }
    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![self.input.as_ref()]
    }
    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let pulled = match self.input.execute(ctx) {
            FlowResult::Rows(batches) => batches,
            FlowResult::Error(e) => return FlowResult::Error(e),
            flow @ (FlowResult::Break | FlowResult::Continue) => return flow,
        };
        let rows_in: usize = pulled.iter().map(|b| b.len()).sum();
        let mut uid_dists: Vec<(u64, f64)> = Vec::with_capacity(rows_in);
        let mut dropped = 0usize;
        for batch in pulled {
            for e in batch.0 {
                match embedded_floats(
                    ctx.runtime.stored_field(&e, &self.embedding_field),
                    self.query.len(),
                ) {
                    Some(embed) => {
                        let dot: f64 = embed.iter().zip(self.query.iter()).map(|(a, b)| a * b).sum();
                        let norm_a: f64 = embed.iter().map(|a| a * a).sum::<f64>().sqrt();
                        let norm_b: f64 = self.query.iter().map(|b| b * b).sum::<f64>().sqrt();
                        if norm_a > 0.0 && norm_b > 0.0 {
                            uid_dists.push((e.uid, 1.0 - dot / (norm_a * norm_b)));
                        } else {
                            uid_dists.push((e.uid, f64::MAX));
                        }
                    }
                    None => dropped += 1,
                }
            }
        }
        uid_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let out = vec![RowBatch(
            uid_dists.into_iter().map(|(u, _)| EntityId::from(u)).collect(),
        )];
        let rows_out = out[0].len();
        let mut notes = Vec::new();
        if dropped > 0 {
            notes.push(format!("dropped {dropped} rows without usable embedding"));
        }
        record(
            ctx,
            "sort",
            self.detail(),
            rows_in,
            rows_out,
            start,
            notes,
        );
        FlowResult::Rows(out)
    }
}
