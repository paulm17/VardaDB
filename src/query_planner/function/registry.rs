//! Name-keyed scalar-function registry, ported from upstream
//! `exec/function/registry.rs` (`register`/`get`/`contains` shape preserved).

use std::collections::HashMap;
use std::sync::Arc;

use super::ScalarFunction;

#[derive(Default)]
pub struct FunctionRegistry {
    functions: HashMap<&'static str, Arc<dyn ScalarFunction>>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, func: impl ScalarFunction + 'static) {
        self.register_arc(Arc::new(func));
    }

    pub fn register_arc(&mut self, func: Arc<dyn ScalarFunction>) {
        self.functions.insert(func.name(), func);
    }

    /// Look up a function by its exact (lowercase) registration name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn ScalarFunction>> {
        self.functions.get(name).cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.functions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}
