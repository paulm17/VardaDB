use crate::storage::backend::Storage;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct ObsState {
    pub storage: Arc<Storage>,
}

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ObsState: axum::extract::FromRef<S>,
{
    // let state = ObsState { storage }; // Removed

    Router::new()
        .route("/metrics", get(list_metrics))
        .route("/traces", get(list_traces))
        .route("/dashboard", get(dashboard_html))
        .route("/debug/query-plans", get(list_query_plans))
}

#[derive(Deserialize)]
struct QueryPlansParams {
    limit: Option<usize>,
    /// `text` renders the human-readable plans; default is JSON.
    format: Option<String>,
}

/// Stage 2.3 planner debugging endpoint: recent pipeline captures (plan text,
/// machine-readable plan tree, per-operator rows in/out) recorded by the
/// query planner while serving normal traffic.
async fn list_query_plans(Query(params): Query<QueryPlansParams>) -> Response {
    use crate::query_planner::debug_capture;
    let limit = params.limit.unwrap_or(20);
    let captures = debug_capture::recent(limit);

    if params.format.as_deref() == Some("text") {
        let mut body = String::new();
        for c in captures {
            body.push_str(&format!(
                "=== [{}] {} {} {} shape={} elapsed={}us\n{}\n",
                c.captured_at_ms, c.db, c.kind, c.type_name, c.shape, c.elapsed_us, c.text
            ));
            for s in &c.operator_stats {
                body.push_str(&format!(
                    "  op {} {} rows_in={} rows_out={} us={}\n",
                    s.kind, s.detail, s.rows_in, s.rows_out, s.elapsed_us
                ));
            }
            body.push('\n');
        }
        return (
            [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            body,
        )
            .into_response();
    }

    Json(serde_json::json!({
        "enabled": debug_capture::enabled(),
        "count": captures.len(),
        "plans": captures
            .iter()
            .map(crate::query_planner::debug_capture::CapturedPlan::to_json)
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct MetricsQuery {
    start: Option<u64>,
    end: Option<u64>,
}

async fn list_metrics(
    State(state): State<ObsState>,
    Query(params): Query<MetricsQuery>,
) -> Json<serde_json::Value> {
    let start_ts = params.start.unwrap_or(0);
    let end_ts = params.end.unwrap_or(u64::MAX);

    // Scan metrics_keyspace
    // Key Format: "StartWithPrefix" -> We need to iterate over all?
    // Keys are [Type:Name:TS] -> e.g "g:system.cpu:12345678"
    // To scan efficiently we really need "Name" to be separate or known.
    // For now, full table scan of metrics_keyspace is acceptable if retention is short.
    // Or we scan filtering by TS if possible? No, keys are "Name:TS".
    // We will scan everything and group by Name in memory (Prototype style).

    let mut result = serde_json::Map::new();

    // Iterate
    // Note: fjall iter is alphabetical on keys.
    // "c:graphql_requests:17000..."

    for (k, v) in state.storage.metrics_table.iter() {
        if let Ok(key_str) = std::str::from_utf8(&k) {
            // key_str: "type:name:ts"
            let parts: Vec<&str> = key_str.split(':').collect();
            if parts.len() >= 3 {
                // Last part is TS?
                if let Ok(ts) = parts.last().unwrap().parse::<u64>() {
                    if ts >= start_ts && ts <= end_ts {
                        let name = parts[1..parts.len() - 1].join(":");
                        let type_ = parts[0];

                        let val: f64 = if v.len() == 8 {
                            f64::from_bits(u64::from_be_bytes(v[0..8].try_into().unwrap()))
                        } else {
                            0.0
                        };

                        // Add to JSON: { "system.cpu": [ {t, v}, ... ] }
                        let series = result
                            .entry(name)
                            .or_insert(serde_json::Value::Array(Vec::new()));
                        if let serde_json::Value::Array(vec) = series {
                            vec.push(serde_json::json!({ "t": ts, "v": val, "type": type_ }));
                        }
                    }
                }
            }
        }
    }

    Json(serde_json::Value::Object(result))
}

async fn list_traces(State(state): State<ObsState>) -> Json<Vec<serde_json::Value>> {
    // Return last 50 traces
    // Keys are [TS][Duration] -> Sorted by time.
    // keyspace.iter().rev().take(50)

    let mut traces = Vec::new();

    let mut items = state.storage.traces_table.iter();
    items.reverse();
    for (_, v) in items.into_iter().take(50) {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&v) {
            traces.push(json);
        }
    }

    Json(traces)
}

async fn dashboard_html() -> Html<&'static str> {
    Html(crate::observability::ui::HTML)
}
