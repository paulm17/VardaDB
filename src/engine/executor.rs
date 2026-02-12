use crate::engine::resolver::Resolver;
use async_graphql::Value;

pub struct Executor {
    pub resolver: Box<dyn Resolver + Send + Sync>,
}

impl Executor {
    pub fn new(resolver: Box<dyn Resolver + Send + Sync>) -> Self {
        Self { resolver }
    }
}

// Dummy Resolver for scaffolding
pub struct DummyResolver;
impl Resolver for DummyResolver {
    fn resolve(&self, _uid: u64, _field: &str) -> Option<Value> {
        None
    }
    fn find_uid(&self, _index: &str, _value: &str) -> Option<u64> {
        None
    }
    fn create_node(&self, _type: &str, _fields: std::collections::HashMap<String, Value>, _uniques: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>, _: Option<&crate::engine::resolver::VectorConfig>) -> Result<u64, String> {
        Ok(0)
    }
    fn scan_nodes(&self, _: &str, _: std::collections::HashMap<String, Value>, _: std::collections::HashMap<String, Value>, _: Option<usize>, _: Option<String>, _: &[String], _: Option<Vec<f64>>) -> Vec<u64> {
        vec![]
    }
    fn resolve_list(&self, _: u64, _: &str, _: std::collections::HashMap<String, Value>, _: std::collections::HashMap<String, Value>, _: Option<usize>, _: Option<String>, _: Option<Vec<f64>>) -> Result<Vec<u64>, String> {
        Ok(vec![])
    }
    fn update_node(&self, _: &str, _: u64, _: std::collections::HashMap<String, Value>, _: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>, _: Option<&crate::engine::resolver::VectorConfig>) -> Result<(), String> { Ok(()) }
    fn delete_node(&self, _: &str, _: u64, _: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }
    fn node_exists(&self, _: &str, _: u64) -> bool { false }
    fn get_node_type(&self, _: u64) -> Option<String> { None }
    fn subscribe_events(&self) -> crate::realtime::bus::EventBus { crate::realtime::bus::EventBus::new() }
    fn search_vectors(&self, _: &[f64], _: usize) -> Vec<(u64, f64)> { vec![] }
    fn search_hybrid(&self, _: &str, _: &str, _: &[f64], _: usize) -> Vec<(u64, f64)> { vec![] }
    fn flush(&self) -> Result<(), String> { Ok(()) }
    fn compact(&self) -> Result<u64, String> { Ok(0) }
    fn needs_compaction(&self) -> bool { false }
}
