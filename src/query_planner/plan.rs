use crate::query_planner::ir::{EntityId, FilterOp, FilterPredicate, LogicalFilter, QueryValue, SortDirection};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CandidatePlan {
    pub type_name: String,
    pub source: CandidateSource,
    pub residual: Option<LogicalFilter>,
    pub notes: Vec<AccessPathNote>,
}

#[derive(Debug, Clone)]
pub enum CandidateSource {
    FullTypeScan,
    UniqueLookup {
        field: String,
        value: QueryValue,
    },
    OrderedIndexScan {
        field: String,
        direction: SortDirection,
    },
    PredicatePushdown(FilterPredicate),
    TextIndex {
        field: String,
        op: FilterOp,
        query: String,
    },
    VectorIndex {
        field: String,
        query: Vec<f64>,
    },
    RelationExpansion {
        field: String,
        target_type: String,
        child_plan: Box<CandidatePlan>,
        inverse_field: String,
        /// Phase-1 parity bridge: raw nested filter executed through the
        /// legacy child pipeline (which applies residual verification).
        child_raw_filter: Option<RawFilterMap>,
        child_uniques: Vec<String>,
    },
    Intersection(Vec<CandidatePlan>),
    Union(Vec<CandidatePlan>),
}

impl CandidateSource {
    pub fn kind(&self) -> &'static str {
        match self {
            CandidateSource::FullTypeScan => "full_type_scan",
            CandidateSource::UniqueLookup { .. } => "unique_lookup",
            CandidateSource::OrderedIndexScan { .. } => "ordered_index_scan",
            CandidateSource::PredicatePushdown(_) => "predicate_pushdown",
            CandidateSource::TextIndex { .. } => "text_index",
            CandidateSource::VectorIndex { .. } => "vector_index",
            CandidateSource::RelationExpansion { .. } => "relation_expansion",
            CandidateSource::Intersection(_) => "intersection",
            CandidateSource::Union(_) => "union",
        }
    }

    pub fn describe(&self) -> String {
        match self {
            CandidateSource::FullTypeScan => "full type scan".to_string(),
            CandidateSource::UniqueLookup { field, .. } => {
                format!("unique index lookup on `{}`", field)
            }
            CandidateSource::OrderedIndexScan { field, direction } => format!(
                "ordered index scan on `{}` ({})",
                field,
                match direction {
                    SortDirection::Asc => "asc",
                    SortDirection::Desc => "desc",
                }
            ),
            CandidateSource::PredicatePushdown(p) => {
                format!("sql pushdown `{} {} {:?}`", p.path, p.op.as_str(), p.value)
            }
            CandidateSource::TextIndex { field, op, query } => {
                format!("{} index lookup on `{}` for {:?}", op.as_str(), field, query)
            }
            CandidateSource::VectorIndex { field, .. } => {
                format!("vector index top-k on `{}`", field)
            }
            CandidateSource::RelationExpansion {
                field,
                target_type,
                inverse_field,
                ..
            } => format!(
                "relation expansion `{}` -> {} via inverse `{}`",
                field, target_type, inverse_field
            ),
            CandidateSource::Intersection(children) => {
                format!("intersection of {} candidate sets", children.len())
            }
            CandidateSource::Union(children) => {
                format!("union of {} candidate sets", children.len())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AccessPathNote {
    pub kind: &'static str,
    pub detail: String,
}

/// Outcome mirrors legacy `Option<HashSet<u64>>` semantics from
/// `SqliteResolver::get_candidates`: `NoNarrowing` lets the caller fall back
/// to its streaming range scan; `Narrowed(vec![])` is an authoritative empty
/// result (e.g. unique-key miss).
#[derive(Debug, Clone, PartialEq)]
pub enum CandidateOutcome {
    NoNarrowing,
    Narrowed(Vec<EntityId>),
}

impl CandidateOutcome {
    pub fn uids(&self) -> Option<Vec<u64>> {
        match self {
            CandidateOutcome::NoNarrowing => None,
            CandidateOutcome::Narrowed(ids) => Some(ids.iter().map(|e| e.uid).collect()),
        }
    }
}

impl CandidatePlan {
    pub fn execute(&self, runtime: &dyn crate::query_planner::traits::PlannerRuntime) -> CandidateOutcome {
        self.exec_inner(runtime, false)
    }

    /// Fast path returning raw uids; `None` maps to `NoNarrowing`.
    pub fn execute_uids(
        &self,
        runtime: &dyn crate::query_planner::traits::PlannerRuntime,
    ) -> Option<Vec<u64>> {
        self.execute(runtime).uids()
    }

    fn exec_inner(
        &self,
        runtime: &dyn crate::query_planner::traits::PlannerRuntime,
        eager_full_scan: bool,
    ) -> CandidateOutcome {
        let ids = |outcomes: Vec<CandidateOutcome>| -> Option<Vec<EntityId>> {
            outcomes
                .into_iter()
                .map(|o| match o {
                    CandidateOutcome::NoNarrowing => None,
                    CandidateOutcome::Narrowed(ids) => Some(ids),
                })
                .try_fold(None::<std::collections::HashSet<u64>>, |acc, item| {
                    match (acc, item) {
                        (None, Some(set)) => Some(Some(set.into_iter().map(|e| e.uid).collect())),
                        (Some(cur), Some(set)) => Some(Some(
                            cur.into_iter().filter(|u| set.iter().any(|e| e.uid == *u)).collect(),
                        )),
                        (acc, None) => acc.map(Some),
                    }
                })?
                .map(|set| set.into_iter().map(EntityId::new).collect())
        };

        let narrowed = |uids: Vec<u64>| -> CandidateOutcome {
            CandidateOutcome::Narrowed(uids.into_iter().map(EntityId::new).collect())
        };

        match &self.source {
            CandidateSource::FullTypeScan => {
                if !eager_full_scan {
                    return CandidateOutcome::NoNarrowing;
                }
                match runtime.scan_type(&self.type_name, None, None) {
                    Ok(list) => CandidateOutcome::Narrowed(list),
                    Err(_) => narrowed(vec![]),
                }
            }
            CandidateSource::UniqueLookup { field, value } => {
                match runtime.lookup_unique(&self.type_name, field, value) {
                    Ok(Some(id)) => CandidateOutcome::Narrowed(vec![id]),
                    Ok(None) => CandidateOutcome::Narrowed(vec![]),
                    Err(_) => CandidateOutcome::NoNarrowing,
                }
            }
            CandidateSource::OrderedIndexScan { field, direction } => {
                match runtime.ordered_scan(&self.type_name, field, *direction, None, None) {
                    Ok(list) => CandidateOutcome::Narrowed(list),
                    Err(_) => CandidateOutcome::NoNarrowing,
                }
            }
            CandidateSource::PredicatePushdown(predicate) => {
                match runtime.candidate_ids(&self.type_name, predicate) {
                    Ok(Some(list)) => narrowed(list.into_iter().map(|e| e.uid).collect()),
                    _ => CandidateOutcome::NoNarrowing,
                }
            }
            CandidateSource::TextIndex { field, op, query } => {
                match runtime.text_search(&self.type_name, field, *op, query, None) {
                    Ok(list) => CandidateOutcome::Narrowed(list),
                    Err(_) => CandidateOutcome::NoNarrowing,
                }
            }
            CandidateSource::VectorIndex { field, query } => {
                match runtime.vector_search(&self.type_name, field, query, None) {
                    Ok(list) => CandidateOutcome::Narrowed(list.into_iter().map(|(e, _)| e).collect()),
                    Err(_) => CandidateOutcome::NoNarrowing,
                }
            }
            CandidateSource::RelationExpansion {
                field: _,
                target_type,
                child_plan,
                inverse_field,
                child_raw_filter,
                child_uniques,
            } => {
                let child_ids: Vec<EntityId> =
                    if let Some(raw) = child_raw_filter {
                        use crate::query_planner::traits::NestedCandidateRequest;
                        let req = NestedCandidateRequest {
                            target_type: target_type.clone(),
                            filter: raw.clone(),
                            uniques: child_uniques.clone(),
                        };
                        match runtime.nested_candidates(&req) {
                            Some(uids) => uids.into_iter().map(EntityId::new).collect(),
                            None => match runtime.scan_type(target_type, None, None) {
                                Ok(list) => list,
                                Err(_) => return narrowed(vec![]),
                            },
                        }
                    } else {
                        let child_outcome = child_plan.exec_inner(runtime, true);
                        match child_outcome {
                            CandidateOutcome::Narrowed(ids) => ids,
                            CandidateOutcome::NoNarrowing => {
                                match runtime.scan_type(target_type, None, None) {
                                    Ok(list) => list,
                                    Err(_) => return narrowed(vec![]),
                                }
                            }
                        }
                    };
                match runtime.reverse_related_ids(target_type, inverse_field, &child_ids) {
                    Ok(parents) => {
                        metrics::counter!("vardadb_planner_relation_expansion_total").increment(1);
                        CandidateOutcome::Narrowed(parents)
                    }
                    Err(_) => CandidateOutcome::NoNarrowing,
                }
            }
            CandidateSource::Intersection(children) => {
                let outcomes: Vec<CandidateOutcome> =
                    children.iter().map(|c| c.exec_inner(runtime, true)).collect();
                match ids(outcomes) {
                    Some(list) => CandidateOutcome::Narrowed(list),
                    None => CandidateOutcome::Narrowed(vec![]),
                }
            }
            CandidateSource::Union(children) => {
                let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
                let mut merged: Vec<EntityId> = Vec::new();
                let mut any_narrowing = false;
                for child in children {
                    if let CandidateOutcome::Narrowed(ids) = child.exec_inner(runtime, true) {
                        any_narrowing = true;
                        for id in ids {
                            if seen.insert(id.uid) {
                                merged.push(id);
                            }
                        }
                    }
                }
                if any_narrowing {
                    CandidateOutcome::Narrowed(merged)
                } else {
                    CandidateOutcome::NoNarrowing
                }
            }
        }
    }
}

pub(crate) type RawFilterMap = HashMap<String, async_graphql::Value>;
