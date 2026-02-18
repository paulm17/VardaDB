
#[tokio::test(flavor = "multi_thread")]
async fn test_deep_mutation() {
    // 1. Setup Engine with Mock Resolver
    // We need a Resolver that stores data. 
    // Ideally use FjallResolver if available, or MockResolver if it supports updates.
    // The previous mocks (mock.rs) were static.
    // We should use `src/engine/mock.rs` if it has state, or define a simple HashMap resolver here.
    
    // For this test, we want to verify schema.rs logic calls create_node recursively.
    // So we can use a Mock that tracks calls.
    
    use std::sync::{Arc, Mutex};
    use std::collections::HashMap;
    use async_graphql::Value;
    use vardadb::engine::resolver::{Resolver, InverseInfo, VectorConfig};

    struct TrackingResolver {
        calls: Arc<Mutex<Vec<String>>>,
        next_uid: Arc<Mutex<u64>>,
    }
    
    impl Resolver for TrackingResolver {
        fn resolve(&self, _uid: u64, _field: &str) -> Option<Value> { None }
        fn find_uid(&self, _index: &str, _value: &str) -> Option<u64> { None }
        fn scan_nodes(&self, _type_name: &str, _filter: HashMap<String, Value>, _sort: HashMap<String, Value>, _first: Option<usize>, _after: Option<String>, _: &[String], _near_vector: Option<Vec<f64>>) -> Vec<u64> { vec![] }
        fn resolve_list(&self, _: u64, _: &str, _: HashMap<String, Value>, _: HashMap<String, Value>, _: Option<usize>, _: Option<String>, _near_vector: Option<Vec<f64>>) -> Result<Vec<u64>, String> { Ok(vec![]) }
        fn create_node(&self, type_name: &str, fields: HashMap<String, Value>, _uniques: &[String], _inverses: &[InverseInfo], _search_fields: &HashMap<String, Vec<String>>, _: Option<&VectorConfig>) -> Result<u64, String> {
            let mut calls = self.calls.lock().unwrap();
            
            // Serialize fields for inspection
            let mut field_keys: Vec<String> = fields.keys().cloned().collect();
            field_keys.sort();
            calls.push(format!("create_node({}, {:?})", type_name, field_keys));
            
            let mut uid_lock = self.next_uid.lock().unwrap();
            let uid = *uid_lock;
            *uid_lock += 1;
            Ok(uid)
        }
        fn update_node(&self, _: &str, _: u64, _: HashMap<String, Value>, _: &[String], _: &[InverseInfo], _: &HashMap<String, Vec<String>>, _: Option<&VectorConfig>) -> Result<(), String> { Ok(()) }
        fn delete_node(&self, _: &str, _: u64, _: &[String], _: &[InverseInfo], _: &HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }
        fn node_exists(&self, _: &str, _: u64) -> bool { true }
        fn get_node_type(&self, _: u64) -> Option<String> { None }
        fn subscribe_events(&self) -> vardadb::realtime::bus::EventBus { vardadb::realtime::bus::EventBus::new() }
        fn search_vectors(&self, _: &[f64], _: usize) -> Vec<(u64, f64)> { vec![] }
        fn search_hybrid(&self, _: &str, _: &str, _: &[f64], _: usize) -> Vec<(u64, f64)> { vec![] }
        fn flush(&self) -> Result<(), String> { Ok(()) }
        fn compact(&self) -> Result<u64, String> { Ok(0) }
        fn needs_compaction(&self) -> bool { false }
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let next_uid = Arc::new(Mutex::new(1));
    let resolver = Box::new(TrackingResolver { calls: calls.clone(), next_uid });

    // 2. Load Schema
    let sdl = "
        type User {
            name: String
            posts: [Post] @hasInverse(field: \"author\")
        }
        type Post {
            title: String
            author: User @hasInverse(field: \"posts\")
        }
    ";
    let schema = vardadb::engine::schema::Schema::load_from_sdl(sdl).unwrap();

    // 3. Execute Deep Mutation
    let mutation = "
        mutation {
            createUser(input: {
                name: \"Alice\",
                posts: [
                    { title: \"Post 1\" },
                    { title: \"Post 2\" }
                ]
            }) {
                uid
            }
        }
    ";
    
    let response = schema.execute_with_resolver(mutation, resolver).await;
    println!("Response: {}", response);
    
    let calls_log = calls.lock().unwrap();
    println!("Calls: {:?}", *calls_log);
    
    // 4. Assertions
    // Expect 3 calls: 2 for Posts, 1 for User.
    assert_eq!(calls_log.len(), 3);
    
    // Order: Dependencies first?
    // recursively `deep_create_node` calls `deep_create_node(child)` THEN `resolver.create_node(parent)`.
    // But `deep_create_node(child)` will complete first.
    // So we expect:
    // create_node(Post, ...)
    // create_node(Post, ...)
    // create_node(User, ...)
    
    assert!(calls_log[0].contains("Post"));
    assert!(calls_log[1].contains("Post"));
    assert!(calls_log[2].contains("User"));
}
