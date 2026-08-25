use crate::engine::resolver::QueryTypeMetadata;
use crate::query_planner::context::PlanContext;
use crate::query_planner::ir::{
    FieldPath, FilterOp, FilterPredicate, LogicalFilter, QueryValue,
};
use crate::query_planner::plan::{AccessPathNote, CandidatePlan, CandidateSource, RawFilterMap};
use async_graphql::Value;
use std::collections::HashMap;

const PUSHDOWN_OPS: [(&str, FilterOp); 6] = [
    ("gt", FilterOp::Gt),
    ("ge", FilterOp::Ge),
    ("lt", FilterOp::Lt),
    ("le", FilterOp::Le),
    ("ne", FilterOp::Ne),
    ("in", FilterOp::In),
];

const TEXT_OPS: [(&str, FilterOp, bool); 4] = [
    ("allofterms", FilterOp::AllOfTerms, false),
    ("anyofterms", FilterOp::AnyOfTerms, false),
    ("alloftext", FilterOp::AllOfText, true),
    ("anyoftext", FilterOp::AnyOfText, true),
];

pub fn build_candidate_plan(
    ctx: &PlanContext,
    filter: &RawFilterMap,
) -> CandidatePlan {
    let lowered = crate::query_planner::lowering::lower_filter_map(filter);

    let mut sources: Vec<CandidateSource> = Vec::new();
    let mut notes: Vec<AccessPathNote> = Vec::new();
    // Fields whose nested filter is enforced authoritatively by a
    // relation-expansion semi-join (child subplan + inverse edge), so the
    // matching `Relation` conjunct can be elided from the row-level residual.
    let mut consumed_relations: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    let mut keys: Vec<&String> = filter.keys().collect();
    keys.sort();

    for key in keys {
        let condition = &filter[key.as_str()];
        if let Some((source, note)) =
            source_for_field(ctx, key, condition, &lowered)
        {
            if let CandidateSource::RelationExpansion { field, .. } = &source {
                consumed_relations.insert(field.clone());
            }
            notes.push(note);
            sources.push(source);
        }
    }

    let source = match sources.len() {
        0 => {
            notes.push(AccessPathNote {
                kind: "full_type_scan",
                detail:
                    "no candidate-producing conjuncts; residual filtering handles all predicates"
                        .to_string(),
            });
            CandidateSource::FullTypeScan
        }
        1 => sources.pop().unwrap(),
        _ => CandidateSource::Intersection(
            sources
                .into_iter()
                .map(|src| CandidatePlan {
                    type_name: ctx.type_name.to_string(),
                    source: src,
                    residual: None,
                    notes: Vec::new(),
                })
                .collect(),
        ),
    };

    let residual = if consumed_relations.is_empty() {
        if lowered.is_empty_conjunction() {
            None
        } else {
            Some(lowered)
        }
    } else {
        match strip_consumed_relations(lowered, &consumed_relations) {
            Some(f) if !f.is_empty_conjunction() => Some(f),
            _ => None,
        }
    };

    CandidatePlan {
        type_name: ctx.type_name.to_string(),
        source,
        residual,
        notes,
    }
}

/// Remove top-level `Relation` conjuncts whose field was narrowed by a
/// relation-expansion semi-join. Only descends through `And`: predicates
/// under `Or`/`Not` were never planned as expansions and stay untouched.
///
/// Soundness: the expansion pipeline verifies every child against the full
/// nested map (child `FilterOperator`, legacy `check_condition` parity) and
/// inverse-expands exactly the parents owning a matching child — identical
/// semantics to the per-row residual walk restricted to the candidate set,
/// which is how the legacy candidate path behaved as well.
fn strip_consumed_relations(
    filter: LogicalFilter,
    consumed: &std::collections::HashSet<String>,
) -> Option<LogicalFilter> {
    match filter {
        LogicalFilter::Relation { field, .. } if consumed.contains(&field) => None,
        LogicalFilter::And(parts) => Some(LogicalFilter::And(
            parts
                .into_iter()
                .filter_map(|part| strip_consumed_relations(part, consumed))
                .collect(),
        )),
        other => Some(other),
    }
}

#[allow(clippy::too_many_lines)]
fn source_for_field(
    ctx: &PlanContext,
    field: &str,
    condition: &Value,
    _lowered: &LogicalFilter,
) -> Option<(CandidateSource, AccessPathNote)> {
    if matches!(field, "and" | "or" | "not") {
        return None;
    }

    let type_meta = ctx.metadata.get(ctx.type_name);
    let uniques = ctx.uniques;

    match condition {
        Value::Object(map) => {
            let eq_value = map.get("eq");
            if let Some(val) = eq_value {
                if uniques.iter().any(|u| u == field) {
                    return Some((
                        CandidateSource::UniqueLookup {
                            field: field.to_string(),
                            value: QueryValue::from(val),
                        },
                        AccessPathNote {
                            kind: "unique_index",
                            detail: format!(
                                "unique equality `{}` -> unique index lookup (miss yields empty result)",
                                field
                            ),
                        },
                    ));
                }
                return Some((
                    CandidateSource::PredicatePushdown(FilterPredicate {
                        path: FieldPath::field(field),
                        op: FilterOp::Eq,
                        value: QueryValue::from(val),
                    }),
                    AccessPathNote {
                        kind: "sql_pushdown",
                        detail: format!("equality `{}` -> sql pushdown", field),
                    },
                ));
            }

            for (key, op) in PUSHDOWN_OPS {
                if let Some(val) = map.get(key) {
                    return Some((
                        CandidateSource::PredicatePushdown(FilterPredicate {
                            path: FieldPath::field(field),
                            op,
                            value: QueryValue::from(val),
                        }),
                        AccessPathNote {
                            kind: "sql_pushdown",
                            detail: format!("`{} {}` -> sql pushdown", field, key),
                        },
                    ));
                }
            }

            if let Some(Value::String(substr)) = map.get("contains") {
                return Some((
                    CandidateSource::PredicatePushdown(FilterPredicate {
                        path: FieldPath::field(field),
                        op: FilterOp::Contains,
                        value: QueryValue::String(substr.clone()),
                    }),
                    AccessPathNote {
                        kind: "sql_pushdown",
                        detail: format!("contains `{}` -> sql pushdown", field),
                    },
                ));
            }

            for (key, op, fulltext) in TEXT_OPS {
                if let Some(Value::String(q)) = map.get(key) {
                    return Some((
                        CandidateSource::TextIndex {
                            field: field.to_string(),
                            op,
                            query: q.clone(),
                        },
                        AccessPathNote {
                            kind: if fulltext { "fulltext_index" } else { "term_index" },
                            detail: format!("`{} {}` -> term index lookup", field, key),
                        },
                    ));
                }
            }

            // Geo/vector predicates stay residual in Phase 1.
            if map.contains_key("near") || map.contains_key("within") || map.contains_key("intersects")
            {
                return None;
            }

            plan_relation_expansion(ctx, field, map, type_meta)
        }
        scalar => {
            if uniques.iter().any(|u| u == field) {
                Some((
                    CandidateSource::UniqueLookup {
                        field: field.to_string(),
                        value: QueryValue::from(scalar),
                    },
                    AccessPathNote {
                        kind: "unique_index",
                        detail: format!("unique scalar equality `{}` -> unique index", field),
                    },
                ))
            } else {
                Some((
                    CandidateSource::PredicatePushdown(FilterPredicate {
                        path: FieldPath::field(field),
                        op: FilterOp::Eq,
                        value: QueryValue::from(scalar),
                    }),
                    AccessPathNote {
                        kind: "sql_pushdown",
                        detail: format!("scalar equality `{}` -> sql pushdown", field),
                    },
                ))
            }
        }
    }
}

fn plan_relation_expansion(
    ctx: &PlanContext,
    field: &str,
    map: &async_graphql::indexmap::IndexMap<async_graphql::Name, Value>,
    type_meta: Option<&QueryTypeMetadata>,
) -> Option<(CandidateSource, AccessPathNote)> {
    let meta = type_meta?;
    let target_type = meta.relations.get(field)?;
    let inverse = meta.inverses.iter().find(|info| info.field == field)?;

    let child_map: RawFilterMap = map
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let child_uniques: Vec<String> = ctx
        .metadata
        .get(target_type)
        .map(|m| m.uniques.clone())
        .unwrap_or_default();

    let child_ctx = PlanContext {
        db_name: ctx.db_name,
        type_name: target_type,
        uniques: &child_uniques,
        metadata: ctx.metadata,
    };
    let child_plan = build_candidate_plan(&child_ctx, &child_map);

    Some((
        CandidateSource::RelationExpansion {
            field: field.to_string(),
            target_type: target_type.clone(),
            inverse_field: inverse.inverse_field.clone(),
            child_plan: Box::new(child_plan),
            child_raw_filter: Some(child_map),
            child_uniques,
        },
        AccessPathNote {
            kind: "relation_expansion",
            detail: format!(
                "nested filter on relation `{}` -> plan children of {} then expand inverse `{}`",
                field, target_type, inverse.inverse_field
            ),
        },
    ))
}

/// Convenience wrapper used by the resolver integration points.
pub fn plan_candidates<'a>(
    db_name: &'a str,
    type_name: &str,
    filter: &RawFilterMap,
    uniques: &'a [String],
    metadata: &'a HashMap<String, QueryTypeMetadata>,
) -> CandidatePlan {
    let ctx = PlanContext {
        db_name,
        type_name,
        uniques,
        metadata,
    };
    build_candidate_plan(&ctx, filter)
}

// ---------------------------------------------------------------------------
// Stage 3.2: aggregate compilation
// ---------------------------------------------------------------------------

/// Compile logical aggregate specifications into executable operator specs.
///
/// Maps the IR enum onto registry names (`Count`->`count`, `Sum`->
/// `math::sum`, `Mean`->`math::mean`, `Min`/`Max`->`math::min`/`math::max`).
/// A `Count` without an argument compiles to the constant `Int(1)` so the
/// accumulator counts rows rather than values.
pub fn compile_aggregates(
    specs: &[crate::query_planner::ir::AggregateSpec],
) -> Result<Vec<crate::query_planner::operators::aggregate::AggregateSpec>, crate::query_planner::operators::PlannerError> {
    use crate::query_planner::function::default_aggregate_registry;
    use crate::query_planner::ir::AggregateFunction;
    use crate::query_planner::operators::PlannerError;
    use crate::query_planner::physical_expr::compile_arc;

    let registry = default_aggregate_registry();
    let mut compiled = Vec::with_capacity(specs.len());
    for spec in specs {
        let name = match spec.function {
            AggregateFunction::Count => "count",
            AggregateFunction::Sum => "math::sum",
            AggregateFunction::Mean => "math::mean",
            AggregateFunction::Min => "math::min",
            AggregateFunction::Max => "math::max",
        };
        let Some(func) = registry.get(name) else {
            return Err(PlannerError::Unsupported(format!(
                "aggregate function {name:?} is not registered"
            )));
        };
        let arg = match &spec.expr {
            Some(expr) => Some(
                compile_arc(expr)
                    .map_err(|e| PlannerError::Unsupported(e.to_string()))?,
            ),
            None => {
                if matches!(spec.function, AggregateFunction::Count) {
                    Some(compile_arc(&crate::query_planner::ir::LogicalExpr::Value(
                        QueryValue::Int(1),
                    ))
                    .map_err(|e| PlannerError::Unsupported(e.to_string()))?)
                } else {
                    None
                }
            }
        };
        compiled.push(crate::query_planner::operators::aggregate::AggregateSpec {
            func,
            arg,
            alias: spec.alias.clone(),
        });
    }
    Ok(compiled)
}
