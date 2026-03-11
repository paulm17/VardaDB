use async_graphql::Value;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InverseInfo {
    pub field: String,
    pub inverse_type: String,
    pub inverse_field: String,
    pub inverse_is_list: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorConfig {
    pub field: String,
    pub source: String,
    // Future: pub model: String
}

pub trait Resolver {
    // Resolve a specific field for an entity (UID)
    fn resolve(&self, uid: u64, field_name: &str) -> Option<Value>;

    // Resolve by unique index (e.g. users(name: "Alice")) -> Returns UID
    fn find_uid(&self, index_name: &str, value: &str) -> Option<u64>;

    // Scan nodes of a type with optional filter constraints and pagination
    fn scan_nodes(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
    ) -> Vec<u64>;

    // Resolve a list of related nodes (1:M) with filter/sort/pagination
    fn resolve_list(
        &self,
        parent_uid: u64,
        field_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        near_vector: Option<Vec<f64>>,
    ) -> Result<Vec<u64>, String>;

    // CRUD with Inverses
    fn create_node(
        &self,
        type_name: &str,
        fields: std::collections::HashMap<String, Value>,
        uniques: &[String],
        inverses: &[InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
        vector_config: Option<&VectorConfig>,
    ) -> Result<u64, String>;
    fn update_node(
        &self,
        type_name: &str,
        uid: u64,
        fields: std::collections::HashMap<String, Value>,
        uniques: &[String],
        inverses: &[InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
        vector_config: Option<&VectorConfig>,
    ) -> Result<(), String>;
    fn delete_node(
        &self,
        type_name: &str,
        uid: u64,
        uniques: &[String],
        inverses: &[InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
    ) -> Result<(), String>;

    // Check existence
    fn node_exists(&self, type_name: &str, uid: u64) -> bool;

    // Polymorphism
    // Polymorphism
    fn get_node_type(&self, uid: u64) -> Option<String>;

    // Realtime Events
    fn subscribe_events(&self) -> crate::realtime::bus::EventBus;

    // Vector Search
    fn search_vectors(&self, query: &[f64], k: usize) -> Vec<(u64, f64)>;

    // Advanced Search
    fn search_hybrid(&self, text: &str, field: &str, vector: &[f64], k: usize) -> Vec<(u64, f64)>;

    // Maintenance
    fn flush(&self) -> Result<(), String>;
    fn compact(&self) -> Result<u64, String>; // Returns duration_ms
    fn needs_compaction(&self) -> bool;

    // Authorization
    fn bulk_check_permission(
        &self,
        ctx: &async_graphql::dynamic::ResolverContext<'_>,
        checks: Vec<(String, String, String)>,
    ) -> async_graphql::Result<Vec<(String, String, String, bool)>>;
}
