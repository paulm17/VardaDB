use vardadb::engine::schema::Schema;
use serde_json::Value;

#[tokio::test]
async fn test_execution_flow() {
    // 1. Setup Schema
    let sdl = "
        type User {
            name: String
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 2. Define Query
    let query = "{ getUser(id: \"1\") { name } }";

    // 3. Execute with Mock Resolver
    use vardadb::engine::resolver::Resolver;
    use std::sync::Mutex;
    use std::collections::HashMap;
    
    #[derive(Default)]
    struct MockMapResolver {
        #[allow(dead_code)]
        store: Mutex<HashMap<String, Value>>,
    }
    
    impl Resolver for MockMapResolver {
        fn resolve(&self, uid: u64, field: &str) -> Option<async_graphql::Value> {
             if uid == 1 && field == "name" {
                 return Some(async_graphql::Value::from("World"));
             }
             None
        }

        fn find_uid(&self, _field: &str, _value: &str) -> Option<u64> {
            None
        }

        fn create_node(&self, _type_name: &str, _fields: std::collections::HashMap<String, async_graphql::Value>, _uniques: &[String], _inverses: &[vardadb::engine::resolver::InverseInfo], _search: &std::collections::HashMap<String, Vec<String>>) -> Result<u64, String> {
             Ok(100)
        }
        fn scan_nodes(&self, _t: &str, _f: std::collections::HashMap<String, async_graphql::Value>, _sort: std::collections::HashMap<String, async_graphql::Value>, _lim: Option<usize>, _cur: Option<String>) -> Vec<u64> { vec![] }
        fn update_node(&self, _: &str, _: u64, _: std::collections::HashMap<String, async_graphql::Value>, _: &[String], _: &[vardadb::engine::resolver::InverseInfo], _search: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }
        fn delete_node(&self, _: &str, _: u64, _: &[String], _: &[vardadb::engine::resolver::InverseInfo], _search: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }
        fn node_exists(&self, _: &str, _: u64) -> bool { true }
        fn get_node_type(&self, _: u64) -> Option<String> { None }
        fn subscribe_events(&self) -> vardadb::realtime::bus::EventBus { vardadb::realtime::bus::EventBus::new() }
    }
    
    let resolver = Box::new(MockMapResolver::default());
    let response_json = schema.execute_with_resolver(query, resolver).await;
    
    // 4. Verify Result
    let response: Value = serde_json::from_str(&response_json).expect("Response should be valid JSON");
    let data = response.get("data").expect("Response should have data field");
    let user = data.get("getUser").expect("Data should have getUser field");
    let name = user.get("name").expect("User should have name field");
    
    assert_eq!(name.as_str(), Some("World"), "Query should return 'World' from Mock");
}
