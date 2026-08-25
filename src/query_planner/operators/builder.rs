//! Pipeline assembly: turns raw GraphQL read arguments into an executable
//! operator tree.
//!
//! This is the Stage 2.1 cutover seam. `scan_nodes_internal` /
//! `count_nodes_internal` are thin dispatchers over [`build_scan_pipeline`] /
//! [`build_count_pipeline`]; every legacy branch (vector, hybrid, text,
//! candidate narrowing, ordered-index fast path) is expressed here as operator
//! composition with observationally identical semantics:
//!
//! - sort overlay probes the order index first and declares its ordering so
//!   the downstream `SortOperator` is eliminated (legacy `sorted_index_scan`
//!   fast path, including "authoritative empty" on success);
//! - absent cursors yield empty for full-scan streams ([`CursorMode::Seek`])
//!   but keep all rows for set-based sources ([`CursorMode::KeepAllIfAbsent`])
//!   — the legacy behaviors;
//! - vector/text results keep relevance/distance order unless explicitly
//!   sorted;
//! - set-based sources emit ascending uids when unsorted.

use std::collections::HashMap;

use async_graphql::Value;

use crate::engine::resolver::QueryTypeMetadata;
use crate::query_planner::ir::{FieldSegment, FilterOp, LogicalFilter};
use crate::query_planner::operators::filter::FilterOperator;
use crate::query_planner::operators::pagination::{
    CursorSkipOperator, LimitOperator, OffsetOperator,
};
use crate::query_planner::operators::sort::SortOperator;
use crate::query_planner::operators::source::{
    build_source_tree, HybridSearchScan, OrderedIndexScan, TextBM25Scan, VecSource, VectorKNNScan,
};
use crate::query_planner::operators::{ExecContext, ExecOperator, FlowResult};
use crate::query_planner::plan::{CandidatePlan, RawFilterMap};
use crate::query_planner::planner::plan_candidates;
use crate::query_planner::traits::PlannerRuntime;
use crate::query_planner::{lower_filter_map, lower_sort_map};

/// A fully assembled pipeline plus the metadata the dispatcher needs for
/// metrics/debug logs.
pub struct BuiltPipeline {
    pub root: Box<dyn ExecOperator>,
    /// Access-shape tag for `vardadb_planner_access_total` (legacy strings).
    pub shape: String,
    /// True when the planner narrowed candidates (legacy `used_candidates`).
    pub used_candidates: bool,
    /// The candidate plan when planner-based access was chosen (debug/explain).
    pub plan: Option<CandidatePlan>,
}

fn text_search_op(
    type_name: &str,
    field: &str,
    strategy: &str,
    query: &str,
    require_all: bool,
    k: usize,
) -> TextBM25Scan {
    let op = match (strategy, require_all) {
        ("term", true) => FilterOp::AllOfTerms,
        ("term", false) => FilterOp::AnyOfTerms,
        ("fulltext", true) => FilterOp::AllOfText,
        _ => FilterOp::AnyOfText,
    };
    let mut scan = TextBM25Scan::new(type_name, field, op, query);
    scan.limit = Some(k);
    scan
}

enum BaseSource {
    Search(Box<dyn ExecOperator>, &'static str),
    Planned(CandidatePlan),
}

#[allow(clippy::too_many_arguments)]
fn build_base_source(
    db_name: &str,
    type_name: &str,
    filter: &RawFilterMap,
    uniques: &[String],
    metadata: &HashMap<String, QueryTypeMetadata>,
    near_vector: Option<&Vec<f64>>,
    text_search: Option<&(String, String, String, bool)>,
    k: usize,
    hybrid_shape: &'static str,
) -> BaseSource {
    match (near_vector, text_search) {
        (Some(vec), Some((field, _, query, require_all))) => BaseSource::Search(
            Box::new(HybridSearchScan {
                type_name: type_name.to_string(),
                field: field.clone(),
                text_query: query.clone(),
                require_all: *require_all,
                vector: vec.clone(),
                limit: Some(k),
            }),
            hybrid_shape,
        ),
        (Some(vec), None) => BaseSource::Search(
            Box::new(VectorKNNScan {
                type_name: type_name.to_string(),
                field: String::new(),
                query: vec.clone(),
                limit: Some(k),
            }),
            "vector_search",
        ),
        (None, Some((field, strategy, query, require_all))) => BaseSource::Search(
            Box::new(text_search_op(
                type_name,
                field,
                strategy,
                query,
                *require_all,
                k,
            )),
            "text_bm25",
        ),
        (None, None) => BaseSource::Planned(plan_candidates(db_name, type_name, filter, uniques, metadata)),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_scan_pipeline(
    db_name: &str,
    type_name: &str,
    filter: &RawFilterMap,
    sort: &HashMap<String, Value>,
    first: Option<usize>,
    after: Option<&str>,
    offset: Option<usize>,
    near_vector: Option<&Vec<f64>>,
    text_search: Option<&(String, String, String, bool)>,
    uniques: &[String],
    metadata: &HashMap<String, QueryTypeMetadata>,
    runtime: &dyn PlannerRuntime,
    ctx: &mut ExecContext,
) -> BuiltPipeline {
    let keys = lower_sort_map(sort);
    let base = build_base_source(
        db_name,
        type_name,
        filter,
        uniques,
        metadata,
        near_vector,
        text_search,
        first.unwrap_or(50) * 4,
        "hybrid_search",
    );

    // Row-level residual: full lowered filter for search-driven sources;
    // for planned bases the plan's own (semi-join-stripped) residual.
    let residual: Option<LogicalFilter> = match &base {
        BaseSource::Search(..) => {
            if filter.is_empty() {
                None
            } else {
                Some(lower_filter_map(filter))
            }
        }
        BaseSource::Planned(plan) => plan.residual.clone(),
    };

    let shape: String;
    let mut used_candidates = false;
    let mut planned_full_scan = false;
    let mut kept_plan: Option<CandidatePlan> = None;
    let source: Box<dyn ExecOperator> = match base {
        BaseSource::Search(op, s) => {
            shape = s.to_string();
            op
        }
        BaseSource::Planned(plan) => {
            let narrowed = plan.source.kind() != "full_type_scan";
            used_candidates = narrowed;
            planned_full_scan = !narrowed;
            shape = if narrowed {
                plan.source.kind().to_string()
            } else {
                "full_type_scan_streaming".to_string()
            };
            kept_plan = Some(plan);
            let p = kept_plan.as_ref().expect("planned base captured");
            match build_source_tree(type_name, &p.source) {
                Ok(op) => op,
                Err(_) => Box::new(VecSource::new(
                    "relation_bridge",
                    p.execute_uids(runtime).unwrap_or_default(),
                )),
            }
        }
    };

    // Ordered-index fast path: legacy attempted sorted_index_scan BEFORE any
    // other execution whenever a sort was requested. A successful probe is
    // authoritative even when it returns zero rows; only a missing index
    // falls through to the general chain.
    if !keys.is_empty()
        && keys[0]
            .path
            .segments
            .iter()
            .all(|s| matches!(s, FieldSegment::Field(_)))
    {
        let field = match keys[0].path.segments.first() {
            Some(FieldSegment::Field(f)) => f.clone(),
            _ => unreachable!("non-Field segments excluded above"),
        };
        let probe = OrderedIndexScan {
            type_name: type_name.to_string(),
            field: field.clone(),
            direction: keys[0].direction.clone(),
            cursor: None,
            limit: None,
        };
        if let FlowResult::Rows(batches) = probe.execute(ctx) {
            let uids = batches
                .into_iter()
                .flat_map(|b| b.0.into_iter().map(|e| e.uid))
                .collect();
            metrics::counter!(
                "vardadb_planner_access_total",
                "shape" => "ordered_index_scan".to_string()
            )
            .increment(1);
            let src = VecSource::ordered(
                format!("ordered_index_probe {}.{}", type_name, field),
                uids,
                field,
                keys[0].direction.clone(),
            );
            return BuiltPipeline {
                root: finish_chain(
                    Box::new(src),
                    Some(lower_filter_map(filter)),
                    &keys,
                    after,
                    offset,
                    first,
                    true,
                ),
                shape: "ordered_index_scan".to_string(),
                used_candidates: true,
                plan: kept_plan.take(),
            };
        }
    }

    // Seek semantics (absent cursor yields nothing) apply exactly when legacy
    // streamed the type prefix range WITHOUT sorting: no narrowing plan and no
    // sort keys. When a sort was requested but the ordered probe failed,
    // legacy's tail applied positional keep-all-if-absent semantics instead.
    BuiltPipeline {
        root: finish_chain(
            source,
            residual,
            &keys,
            after,
            offset,
            first,
            planned_full_scan && keys.is_empty(),
        ),
        shape,
        used_candidates,
        plan: kept_plan,
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_chain(
    source: Box<dyn ExecOperator>,
    residual: Option<LogicalFilter>,
    keys: &[crate::query_planner::ir::OrderKey],
    after: Option<&str>,
    offset: Option<usize>,
    first: Option<usize>,
    seek_cursor: bool,
) -> Box<dyn ExecOperator> {
    let mut node = source;
    if let Some(f) = residual {
        if !f.is_empty_conjunction() {
            node = FilterOperator::boxed(node, f);
        }
    }
    if !keys.is_empty() {
        node = SortOperator::boxed(node, keys.to_vec());
    }
    let after_uid = after.and_then(|s| s.parse::<u64>().ok());
    node = if seek_cursor {
        CursorSkipOperator::seek_boxed(node, after_uid)
    } else {
        CursorSkipOperator::boxed(node, after_uid)
    };
    if let Some(skip) = offset {
        node = OffsetOperator::boxed(node, skip);
    }
    if let Some(limit) = first {
        node = LimitOperator::boxed(node, limit);
    }
    node
}

/// Relation list pipelines (Stage 2.2a): edge scan -> optional cosine re-rank
/// -> residual filter -> sort -> cursor -> offset -> limit. Mirrors legacy
/// `resolve_list_internal` exactly; its cursor is positional keep-all-if-absent.
#[allow(clippy::too_many_arguments)]
pub fn build_relation_pipeline(
    parent_uid: u64,
    field_name: &str,
    filter: &RawFilterMap,
    sort: &HashMap<String, Value>,
    first: Option<usize>,
    after: Option<&str>,
    offset: Option<usize>,
    near_vector: Option<Vec<f64>>,
) -> BuiltPipeline {
    use crate::query_planner::operators::relation::{CosineRerankOperator, RelatedIdsSource};

    let mut shape = "related_ids".to_string();
    let mut source: Box<dyn ExecOperator> = Box::new(RelatedIdsSource::new(parent_uid, field_name));
    if let Some(vec) = near_vector {
        source = CosineRerankOperator::boxed(source, vec);
        shape = "relation_cosine_rerank".to_string();
    }
    let keys = lower_sort_map(sort);
    let residual = if filter.is_empty() {
        None
    } else {
        Some(lower_filter_map(filter))
    };
    BuiltPipeline {
        root: finish_chain(source, residual, &keys, after, offset, first, false),
        shape,
        used_candidates: false,
        plan: None,
    }
}

/// Count pipelines carry no pagination and never take the ordered fast path.
/// Shape tags mirror legacy `count_nodes_internal`: hybrid counts under
/// `vector_search`, text-only under `text_bm25`.
pub fn build_count_pipeline(
    db_name: &str,
    type_name: &str,
    filter: &RawFilterMap,
    near_vector: Option<&Vec<f64>>,
    text_search: Option<&(String, String, String, bool)>,
    uniques: &[String],
    metadata: &HashMap<String, QueryTypeMetadata>,
    runtime: &dyn PlannerRuntime,
) -> BuiltPipeline {
    let base = build_base_source(
        db_name,
        type_name,
        filter,
        uniques,
        metadata,
        near_vector,
        text_search,
        10_000,
        "vector_search",
    );
    let mut kept_plan: Option<CandidatePlan> = None;
    let (shape, used_candidates) = match &base {
        BaseSource::Search(_, s) => ((*s).to_string(), false),
        BaseSource::Planned(plan) => {
            let narrowed = plan.source.kind() != "full_type_scan";
            (
                if narrowed {
                    plan.source.kind().to_string()
                } else {
                    "full_type_scan_streaming".to_string()
                },
                narrowed,
            )
        }
    };
    let source: Box<dyn ExecOperator> = match base {
        BaseSource::Search(op, _) => op,
        BaseSource::Planned(plan) => {
            kept_plan = Some(plan);
            let p = kept_plan.as_ref().expect("planned base captured");
            match build_source_tree(type_name, &p.source) {
                Ok(op) => op,
                Err(_) => Box::new(VecSource::new(
                    "relation_bridge",
                    p.execute_uids(runtime).unwrap_or_default(),
                )),
            }
        }
    };
    let residual = match &kept_plan {
        Some(plan) => plan.residual.clone(),
        None => {
            if filter.is_empty() {
                None
            } else {
                Some(lower_filter_map(filter))
            }
        }
    };
    let root = match residual {
        Some(f) if !f.is_empty_conjunction() => FilterOperator::boxed(source, f),
        _ => source,
    };
    BuiltPipeline {
        root,
        shape,
        used_candidates,
        plan: kept_plan,
    }
}
