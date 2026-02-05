use vardadb::engine::schema::Schema;
use vardadb::engine::resolver::Resolver;
use async_graphql::Value;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

// Mock Resolver for testing Dynamic Schema
#[derive(Default)]
struct MockDynamicResolver {
    data: Mutex<HashMap<String, Value>>,
}

impl Resolver for MockDynamicResolver {
    fn resolve(&self, uid: u64, field: &str) -> Option<Value> {
        // Simple mock: store data as "uid:field" -> Value
        let key = format!("{}:{}", uid, field);
        let map = self.data.lock().unwrap();
        map.get(&key).cloned()
    }
    
    fn find_uid(&self, _field: &str, _value: &str) -> Option<u64> {
        None 
    }

    fn create_node(&self, _type_name: &str, _fields: std::collections::HashMap<String, Value>, _uniques: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>) -> Result<u64, String> {
        Ok(200)
    }
    
    fn scan_nodes(&self, _t: &str, _f: std::collections::HashMap<String, Value>, _sort: std::collections::HashMap<String, Value>, _lim: Option<usize>, _cur: Option<String>) -> Vec<u64> { vec![] }
    fn update_node(&self, _: &str, _: u64, _: std::collections::HashMap<String, Value>, _: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }
    fn delete_node(&self, _: &str, _: u64, _: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }
    fn node_exists(&self, _: &str, _: u64) -> bool { true }
    fn get_node_type(&self, _: u64) -> Option<String> { None }
    fn subscribe_events(&self) -> vardadb::realtime::bus::EventBus { vardadb::realtime::bus::EventBus::new() }
    fn search_vectors(&self, _: &[f64], _: usize) -> Vec<(u64, f64)> { vec![] }
    fn search_hybrid(&self, _: &str, _: &str, _: &[f64], _: usize) -> Vec<(u64, f64)> { vec![] }
}

impl MockDynamicResolver {
    fn set(&self, uid: u64, field: &str, val: Value) {
        let key = format!("{}:{}", uid, field);
        self.data.lock().unwrap().insert(key, val);
    }
}

#[tokio::test]
async fn test_dynamic_schema_execution() {
    let sdl = "
        type User {
            id: ID!
            name: String
            age: Int
        }
    ";

    // 1. Load Schema
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load SDL");

    // 2. Setup Mock Data
    let resolver = Box::new(MockDynamicResolver::default());
    resolver.set(1, "name", Value::from("Alice"));
    resolver.set(1, "age", Value::from(30));

    // 3. Execute Query: getUser(id: "1")
    let query = "{ getUser(id: \"1\") { name age } }";
    let json = schema.execute_with_resolver(query, resolver).await;
    
    // 4. Verify Output
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let data = &val["data"]["getUser"];
    
    assert_eq!(data["name"], "Alice");
    assert_eq!(data["age"], 30);
}
