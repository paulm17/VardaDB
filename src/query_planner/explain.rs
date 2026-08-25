use crate::query_planner::ir::{ExplainMode, LogicalFilter, LogicalQuery, QueryRoot};
use crate::query_planner::plan::{CandidatePlan, CandidateSource};

pub fn render_candidate_plan(plan: &CandidatePlan) -> String {
    let mut out = String::new();
    out.push_str(&format!("CandidatePlan(type={})\n", plan.type_name));
    for note in &plan.notes {
        out.push_str(&format!("  [{}] {}\n", note.kind, note.detail));
    }
    render_source(&plan.source, 1, &mut out);
    if let Some(residual) = &plan.residual {
        out.push_str(&format!("  residual: {}\n", render_filter(residual)));
    }
    out
}

fn render_source(source: &crate::query_planner::plan::CandidateSource, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth + 1);
    out.push_str(&format!("{}- {}\n", pad, source.describe()));
    match source {
        crate::query_planner::plan::CandidateSource::Intersection(children)
        | crate::query_planner::plan::CandidateSource::Union(children) => {
            for child in children {
                render_source(&child.source, depth + 1, out);
            }
        }
        crate::query_planner::plan::CandidateSource::RelationExpansion { child_plan, .. } => {
            render_source(&child_plan.source, depth + 1, out);
        }
        _ => {}
    }
}

pub fn render_logical_query(query: &LogicalQuery) -> String {
    let mut out = String::new();
    out.push_str("LogicalQuery\n");
    out.push_str(&format!("  root: {}\n", render_root(&query.root)));
    if let Some(filter) = &query.filter {
        out.push_str(&format!("  filter: {}\n", render_filter(filter)));
    } else {
        out.push_str("  filter: none\n");
    }
    if !query.order_by.is_empty() {
        let keys: Vec<String> = query
            .order_by
            .iter()
            .map(|k| {
                format!(
                    "{} {}",
                    k.path,
                    match k.direction {
                        crate::query_planner::ir::SortDirection::Asc => "ASC",
                        crate::query_planner::ir::SortDirection::Desc => "DESC",
                    }
                )
            })
            .collect();
        out.push_str(&format!("  order_by: {}\n", keys.join(", ")));
    }
    if query.pagination.first.is_some() || query.pagination.offset.is_some() {
        out.push_str(&format!(
            "  pagination: first={:?} offset={:?} after={:?}\n",
            query.pagination.first, query.pagination.offset,
            query.pagination.after.as_ref().map(|c| match c {
                crate::query_planner::ir::CursorValue::Entity(e) => e.raw(),
                _ => "?".to_string(),
            })
        ));
    }
    if !query.aggregates.is_empty() {
        let aggs: Vec<String> = query
            .aggregates
            .iter()
            .map(|a| format!("{}()", a.alias))
            .collect();
        out.push_str(&format!("  aggregates: {}\n", aggs.join(", ")));
    }
    out
}

fn render_root(root: &QueryRoot) -> String {
    match root {
        QueryRoot::TypeScan { type_name } => format!("TypeScan({})", type_name),
        QueryRoot::UniqueLookup {
            type_name,
            field,
            value,
        } => format!("UniqueLookup({}.{} = {:?})", type_name, field, value),
        QueryRoot::IdLookup { type_name, id } => {
            format!("IdLookup({}.{})", type_name, id.raw())
        }
        QueryRoot::RelationScan {
            parent_type,
            field,
            ..
        } => format!("RelationScan({} via {})", parent_type, field),
        QueryRoot::CandidateSet { type_name, source } => {
            format!("CandidateSet({}: {})", type_name, source.describe())
        }
    }
}

fn render_filter(filter: &LogicalFilter) -> String {
    match filter {
        LogicalFilter::And(parts) => {
            let inner: Vec<String> = parts.iter().map(render_filter).collect();
            format!("AND({})", inner.join(", "))
        }
        LogicalFilter::Or(parts) => {
            let inner: Vec<String> = parts.iter().map(render_filter).collect();
            format!("OR({})", inner.join(", "))
        }
        LogicalFilter::Not(inner) => format!("NOT({})", render_filter(inner)),
        LogicalFilter::Predicate(p) => {
            format!("{} {} {:?}", p.path, p.op.as_str(), p.value)
        }
        LogicalFilter::Relation {
            field,
            target_type,
            filter,
        } => {
            let target = if target_type.is_empty() {
                String::new()
            } else {
                format!(":{} ", target_type)
            };
            format!("RELATION {} {{ {}{}}}", field, target, render_filter(filter))
        }
    }
}

pub fn wants_explain(mode: ExplainMode) -> bool {
    matches!(mode, ExplainMode::Text | ExplainMode::Json)
}

/// Machine-readable candidate plan for `/debug/query-plans` (Stage 2.3).
/// Mirrors [`render_candidate_plan`] structure: notes, source tree with
/// nested subplans, and the residual filter.
pub fn candidate_plan_json(plan: &CandidatePlan) -> serde_json::Value {
    serde_json::json!({
        "type": plan.type_name,
        "notes": plan.notes.iter().map(|n| serde_json::json!({
            "kind": n.kind,
            "detail": n.detail,
        })).collect::<Vec<_>>(),
        "source": source_json(&plan.source),
        "residual": plan.residual.as_ref().map(render_filter),
    })
}

fn source_json(source: &CandidateSource) -> serde_json::Value {
    let mut node = serde_json::json!({
        "kind": source.kind(),
        "detail": source.describe(),
    });
    let children: Vec<serde_json::Value> = match source {
        CandidateSource::Intersection(children) | CandidateSource::Union(children) => children
            .iter()
            .map(|c| {
                serde_json::json!({ "type": c.type_name, "source": source_json(&c.source) })
            })
            .collect(),
        CandidateSource::RelationExpansion { child_plan, .. } => vec![serde_json::json!({
            "type": child_plan.type_name,
            "source": source_json(&child_plan.source),
        })],
        _ => Vec::new(),
    };
    if !children.is_empty() {
        node["children"] = serde_json::Value::Array(children);
    }
    node
}

/// Maps a GraphQL-level `explain: Boolean` flag to the IR explain mode.
pub fn explain_mode_from_flag(flag: bool) -> ExplainMode {
    if flag {
        ExplainMode::Text
    } else {
        ExplainMode::None
    }
}
