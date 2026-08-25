//! Compiled-expression interning, ported from upstream
//! `exec/expression_registry.rs`.
//!
//! Upstream keys a `ComputePoint`-indexed registry so identical logical
//! expressions across filter/sort/projection positions share one compiled
//! physical tree. VardaDB keeps the same shape with two adaptations:
//!
//! - **Sync compile.** [`PhysicalExpr`] trees are built synchronously, so
//!   `intern` compiles on first sight and hands back the shared `Arc` after.
//! - **Structural dedup key.** Logical IR nodes derive `Debug`, and their
//!   debug rendering is a stable structural fingerprint within a process, so
//!   `(site, rendered-tree)` is the intern key.
//!
//! The process-global instance ([`global`]) lets the planner, operators, and
//! future lowering stages share one cache without threading state.

use std::sync::{Arc, Mutex, OnceLock};

use crate::query_planner::ir::{LogicalExpr, OrderKey, Projection, ProjectField};
use crate::query_planner::physical_expr::{compile_arc, ExprError, PhysicalExpr};

/// Query position an expression was compiled for (upstream `ComputePoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComputeSite {
    Filter,
    Sort,
    Projection,
}

struct Entry {
    site: ComputeSite,
    name: String,
    expr: Arc<dyn PhysicalExpr>,
    key: String,
}

#[derive(Default)]
pub struct ExpressionRegistry {
    entries: Mutex<Vec<Entry>>,
}

impl ExpressionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compile `expr` for `site`, or return the previously compiled tree when
    /// an identical expression already exists under the same name.
    pub fn intern(
        &self,
        site: ComputeSite,
        name_hint: &str,
        expr: &LogicalExpr,
    ) -> Result<Arc<dyn PhysicalExpr>, ExprError> {
        let key = format!("{site:?}:{name_hint}:{expr:?}");
        let mut entries = self.lock();
        if let Some(existing) = entries.iter().find(|e| e.key == key) {
            return Ok(existing.expr.clone());
        }
        let compiled = compile_arc(expr)?;
        entries.push(Entry {
            site,
            name: name_hint.to_string(),
            expr: compiled.clone(),
            key,
        });
        Ok(compiled)
    }

    /// Compiled expression registered under `name` for `site`.
    pub fn get(&self, site: ComputeSite, name: &str) -> Option<Arc<dyn PhysicalExpr>> {
        self.lock()
            .iter()
            .find(|e| e.site == site && e.name == name)
            .map(|e| e.expr.clone())
    }

    pub fn contains_name(&self, name: &str) -> bool {
        self.lock().iter().any(|e| e.name == name)
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        // A panic while interning must not poison every later query.
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }
}

static GLOBAL: OnceLock<ExpressionRegistry> = OnceLock::new();

/// Process-global expression cache shared by all plan compilations.
pub fn global() -> &'static ExpressionRegistry {
    GLOBAL.get_or_init(ExpressionRegistry::new)
}

/// ORDER BY alias resolution (upstream `resolve_order_by_alias` prior art):
/// a single-segment order path naming a `Computed` projection alias resolves
/// to that alias's interned expression instead of a stored field.
pub fn resolve_order_by_alias(
    key: &OrderKey,
    projection: &Projection,
) -> Option<(String, Arc<dyn PhysicalExpr>)> {
    let alias = key.path.single()?;
    for field in &projection.fields {
        if let ProjectField::Computed { alias: candidate, expr } = field {
            if candidate == alias {
                let compiled = global()
                    .intern(ComputeSite::Sort, candidate, expr)
                    .ok()?;
                return Some((candidate.clone(), compiled));
            }
        }
    }
    None
}
