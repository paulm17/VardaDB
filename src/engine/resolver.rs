use async_graphql::Value;

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InverseInfo {
    pub field: String,
    pub inverse_type: String,
    pub inverse_field: String,
    pub inverse_is_list: bool,
}

pub trait Resolver {
    // Resolve a specific field for an entity (UID)
    fn resolve(&self, uid: u64, field_name: &str) -> Option<Value>;
    
    // Resolve by unique index (e.g. users(name: "Alice")) -> Returns UID
    fn find_uid(&self, index_name: &str, value: &str) -> Option<u64>;

    // Scan nodes of a type with optional filter constraints and pagination
    fn scan_nodes(&self, type_name: &str, filter: std::collections::HashMap<String, Value>, sort: std::collections::HashMap<String, Value>, first: Option<usize>, after: Option<String>) -> Vec<u64>;

    // CRUD with Inverses
    fn create_node(&self, type_name: &str, fields: std::collections::HashMap<String, Value>, uniques: &[String], inverses: &[InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>) -> Result<u64, String>;
    fn update_node(&self, type_name: &str, uid: u64, fields: std::collections::HashMap<String, Value>, uniques: &[String], inverses: &[InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String>;
    fn delete_node(&self, type_name: &str, uid: u64, uniques: &[String], inverses: &[InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String>;
    
    // Check existence
    fn node_exists(&self, type_name: &str, uid: u64) -> bool;
    
    // Polymorphism
    // Polymorphism
    fn get_node_type(&self, uid: u64) -> Option<String>;

    // Realtime Events
    fn subscribe_events(&self) -> crate::realtime::bus::EventBus;
}
