//! Global ring buffer of recent planner pipeline captures (Stage 2.3).
//!
//! Every planner-driven pipeline execution (root scan, count, relation) is
//! recorded here: the human-readable candidate plan, its machine-readable
//! form, and the per-operator stats gathered through `ExplainCapture`. The
//! HTTP endpoint `/debug/query-plans` drains this buffer, so a slow GraphQL
//! query can be explained from VardaDB itself without restarting with debug
//! logging enabled.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::query_planner::operators::OperatorStat;

/// Bounded history so the endpoint always reflects the most recent queries.
const CAPACITY: usize = 100;

static ENABLED: AtomicBool = AtomicBool::new(true);
static CAPTURES: Mutex<Option<VecDeque<CapturedPlan>>> = Mutex::new(None);

/// One recorded planner pipeline execution.
#[derive(Debug, Clone)]
pub struct CapturedPlan {
    /// Wall-clock capture time (unix ms).
    pub captured_at_ms: u64,
    pub db: String,
    /// Pipeline family: `scan`, `count`, or `relation`.
    pub kind: String,
    /// Root type (or `Parent.field` for relation pipelines).
    pub type_name: String,
    /// Access-shape tag matching `vardadb_planner_access_total`.
    pub shape: String,
    /// Human-readable plan render (`render_candidate_plan`, or a fallback
    /// line for search-source pipelines that bypass candidate planning).
    pub text: String,
    /// Machine-readable plan tree when candidate planning produced one.
    pub plan_json: Option<serde_json::Value>,
    /// Per-operator rows in/out and timings from the executed pipeline.
    pub operator_stats: Vec<OperatorStat>,
    /// Total pipeline wall time in microseconds.
    pub elapsed_us: u64,
}

impl CapturedPlan {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "captured_at_ms": self.captured_at_ms,
            "db": self.db,
            "kind": self.kind,
            "type": self.type_name,
            "shape": self.shape,
            "elapsed_us": self.elapsed_us,
            "plan_text": self.text,
            "plan": self.plan_json,
            "operators": operator_stats_json(&self.operator_stats),
        })
    }
}

/// Serialize per-operator explain stats for machine-readable output.
pub fn operator_stats_json(stats: &[OperatorStat]) -> serde_json::Value {
    serde_json::Value::Array(
        stats
            .iter()
            .map(|s| {
                serde_json::json!({
                    "kind": s.kind,
                    "detail": s.detail,
                    "rows_in": s.rows_in,
                    "rows_out": s.rows_out,
                    "elapsed_us": s.elapsed_us,
                    "notes": s.notes,
                })
            })
            .collect(),
    )
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Record one pipeline execution into the bounded ring buffer. No-ops when
/// capturing is disabled.
pub fn record(capture: CapturedPlan) {
    if !enabled() {
        return;
    }
    let mut guard = CAPTURES.lock().expect("debug capture poisoned");
    let queue = guard.get_or_insert_with(VecDeque::new);
    if queue.len() >= CAPACITY {
        queue.pop_front();
    }
    queue.push_back(capture);
}

/// Most recent captures, oldest first. `limit == 0` returns everything.
pub fn recent(limit: usize) -> Vec<CapturedPlan> {
    let guard = CAPTURES.lock().expect("debug capture poisoned");
    match guard.as_ref() {
        Some(queue) => {
            let start = if limit == 0 || limit >= queue.len() {
                0
            } else {
                queue.len() - limit
            };
            queue.iter().skip(start).cloned().collect()
        }
        None => Vec::new(),
    }
}

/// Drop all captured plans (used by tests and by operators resetting state).
pub fn clear() {
    let mut guard = CAPTURES.lock().expect("debug capture poisoned");
    *guard = None;
}
