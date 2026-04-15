use async_graphql::Value;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryTypeMetadata {
    pub uniques: Vec<String>,
    pub inverses: Vec<InverseInfo>,
    pub relations: HashMap<String, String>,
}

#[derive(Default)]
pub struct RequestCache {
    resolved_fields: Mutex<HashMap<(u64, String), Option<Value>>>,
    related_uids: Mutex<HashMap<(u64, String), Vec<u64>>>,
    loaded_objects: Mutex<HashMap<u64, HashMap<String, Value>>>,
}

impl RequestCache {
    pub fn get_resolved(&self, uid: u64, field_name: &str) -> Option<Option<Value>> {
        self.resolved_fields
            .lock()
            .unwrap()
            .get(&(uid, field_name.to_string()))
            .cloned()
    }

    pub fn insert_resolved(&self, uid: u64, field_name: &str, value: Option<Value>) {
        self.resolved_fields
            .lock()
            .unwrap()
            .insert((uid, field_name.to_string()), value);
    }

    pub fn get_related_uids(&self, uid: u64, field_name: &str) -> Option<Vec<u64>> {
        self.related_uids
            .lock()
            .unwrap()
            .get(&(uid, field_name.to_string()))
            .cloned()
    }

    pub fn insert_related_uids(&self, uid: u64, field_name: &str, uids: Vec<u64>) {
        self.related_uids
            .lock()
            .unwrap()
            .insert((uid, field_name.to_string()), uids);
    }

    pub fn get_loaded_object(&self, uid: u64) -> Option<HashMap<String, Value>> {
        self.loaded_objects.lock().unwrap().get(&uid).cloned()
    }

    pub fn insert_loaded_object(&self, uid: u64, fields: HashMap<String, Value>) {
        {
            let mut resolved = self.resolved_fields.lock().unwrap();
            for (field_name, value) in &fields {
                resolved.insert((uid, field_name.clone()), Some(value.clone()));
            }
        }
        self.loaded_objects.lock().unwrap().insert(uid, fields);
    }
}

pub trait Resolver {
    // Resolve a specific field for an entity (UID)
    fn resolve(&self, uid: u64, field_name: &str) -> Option<Value>;

    fn resolve_with_cache(
        &self,
        uid: u64,
        field_name: &str,
        _cache: &RequestCache,
    ) -> Option<Value> {
        self.resolve(uid, field_name)
    }

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
        rrf_alpha: Option<f32>,
        query_metadata: &HashMap<String, QueryTypeMetadata>,
    ) -> Vec<u64>;

    fn scan_nodes_with_cache(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        rrf_alpha: Option<f32>,
        query_metadata: &HashMap<String, QueryTypeMetadata>,
        _cache: &RequestCache,
    ) -> Vec<u64> {
        self.scan_nodes(
            type_name,
            filter,
            sort,
            first,
            after,
            offset,
            uniques,
            near_vector,
            rrf_alpha,
            query_metadata,
        )
    }

    fn count_nodes(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        rrf_alpha: Option<f32>,
        query_metadata: &HashMap<String, QueryTypeMetadata>,
    ) -> usize;

    fn count_nodes_with_cache(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        rrf_alpha: Option<f32>,
        query_metadata: &HashMap<String, QueryTypeMetadata>,
        _cache: &RequestCache,
    ) -> usize {
        self.count_nodes(
            type_name,
            filter,
            uniques,
            near_vector,
            rrf_alpha,
            query_metadata,
        )
    }

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

    fn resolve_list_with_cache(
        &self,
        parent_uid: u64,
        field_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        near_vector: Option<Vec<f64>>,
        _cache: &RequestCache,
    ) -> Result<Vec<u64>, String> {
        self.resolve_list(
            parent_uid,
            field_name,
            filter,
            sort,
            first,
            after,
            offset,
            near_vector,
        )
    }

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
    fn search_hybrid(
        &self,
        text: &str,
        field: &str,
        vector: &[f64],
        k: usize,
        alpha: Option<f32>,
    ) -> Vec<(u64, f64)>;

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

    fn get_index_stats(
        &self,
        _db_name: &str,
    ) -> Result<crate::storage::tantivy_search::IndexStats, String> {
        Err("get_index_stats not implemented".to_string())
    }
}
