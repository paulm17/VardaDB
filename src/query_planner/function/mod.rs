//! Scalar-function infrastructure: [`ScalarFunction`] trait, name-keyed
//! [`FunctionRegistry`], builder [`Signature`]s, and the Stage 3.1b builtin
//! set (`lower`, `upper`, `len`, `concat`, `trim`, `abs`, `ceil`, `floor`,
//! `round`).
//!
//! Ported from upstream `exec/function/*`. Deviation: functions are pure and
//! synchronous (no script/I-O functions), so `evaluate` is a plain `fn`
//! returning `Result<QueryValue, ExprError>`.
//!
//! The default registry is process-global ([`default_registry`]) and lazily
//! populated with [`builtins::all`]; additional functions can be registered in
//! a custom registry instance, which later stages can thread through the
//! planner the same way.

use std::fmt;
use std::sync::OnceLock;

use crate::query_planner::ir::QueryValue;
use crate::query_planner::physical_expr::ExprError;

pub mod builtins;
pub mod registry;
pub mod signature;

pub use registry::FunctionRegistry;
pub use signature::{Param, Signature};

/// A pure, strictly-typed scalar function.
pub trait ScalarFunction: fmt::Debug + Send + Sync {
    /// Exact registration key (lowercase).
    fn name(&self) -> &'static str;

    fn signature(&self) -> Signature;

    /// Evaluate against already-evaluated, Null-free arguments.
    ///
    /// Implementations must still treat unexpected argument kinds as
    /// `ExprError::TypeMismatch` rather than coercing silently.
    fn evaluate(&self, args: &[QueryValue]) -> Result<QueryValue, ExprError>;
}

static DEFAULT_REGISTRY: OnceLock<FunctionRegistry> = OnceLock::new();

/// Process-global registry containing every builtin.
pub fn default_registry() -> &'static FunctionRegistry {
    DEFAULT_REGISTRY.get_or_init(|| {
        let mut registry = FunctionRegistry::new();
        for func in builtins::all() {
            registry.register_arc(func);
        }
        registry
    })
}
