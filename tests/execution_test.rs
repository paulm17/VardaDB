use serde_json::Value;
use vardadb::engine::schema::Schema;

#[tokio::test(flavor = "multi_thread")]
async fn test_execution_flow() {
    // 1. Setup Schema
    let sdl = "
        type User {
            name: String
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 2. Define Query
    let query = "{ getUser(uid: \"1\") { name } }";

    // 3. Execute with Mock Resolver
    use std::collections::HashMap;
    use std::sync::Mutex;
    use vardadb::engine::resolver::{Resolver, VectorConfig};

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

        fn create_node(
            &self,
            _type_name: &str,
            _fields: std::collections::HashMap<String, async_graphql::Value>,
            _uniques: &[String],
            _inverses: &[vardadb::engine::resolver::InverseInfo],
            _search: &std::collections::HashMap<String, Vec<String>>,
            _facet_fields: &[String],
            _: Option<&VectorConfig>,
        ) -> Result<u64, String> {
            Ok(100)
        }
        fn scan_nodes(
            &self,
            _t: &str,
            _f: std::collections::HashMap<String, async_graphql::Value>,
            _sort: std::collections::HashMap<String, async_graphql::Value>,
            _lim: Option<usize>,
            _cur: Option<String>,
            _offset: Option<usize>,
            _: &[String],
            _near_vector: Option<Vec<f64>>,
            _rrf_alpha: Option<f32>,
            _: &std::collections::HashMap<
                String,
                vardadb::engine::resolver::QueryTypeMetadata,
            >,
        ) -> Vec<u64> {
            vec![]
        }
        fn count_nodes(
            &self,
            _: &str,
            _: std::collections::HashMap<String, async_graphql::Value>,
            _: &[String],
            _: Option<Vec<f64>>,
            _: Option<f32>,
            _: &std::collections::HashMap<
                String,
                vardadb::engine::resolver::QueryTypeMetadata,
            >,
        ) -> usize {
            0
        }
        fn resolve_list(
            &self,
            _: u64,
            _: &str,
            _: std::collections::HashMap<String, async_graphql::Value>,
            _: std::collections::HashMap<String, async_graphql::Value>,
            _: Option<usize>,
            _: Option<String>,
            _: Option<usize>,
            _near_vector: Option<Vec<f64>>,
        ) -> Result<Vec<u64>, String> {
            Ok(vec![])
        }
        fn update_node(
            &self,
            _: &str,
            _: u64,
            _: std::collections::HashMap<String, async_graphql::Value>,
            _: &[String],
            _: &[vardadb::engine::resolver::InverseInfo],
            _search: &std::collections::HashMap<String, Vec<String>>,
            _facet_fields: &[String],
            _: Option<&VectorConfig>,
        ) -> Result<(), String> {
            Ok(())
        }
        fn delete_node(
            &self,
            _: &str,
            _: u64,
            _: &[String],
            _: &[vardadb::engine::resolver::InverseInfo],
            _search: &std::collections::HashMap<String, Vec<String>>,
            _facet_fields: &[String],
        ) -> Result<(), String> {
            Ok(())
        }
        fn node_exists(&self, _: &str, _: u64) -> bool {
            true
        }
        fn get_node_type(&self, _: u64) -> Option<String> {
            None
        }
        fn subscribe_events(&self) -> vardadb::realtime::bus::EventBus {
            vardadb::realtime::bus::EventBus::new()
        }
        fn search_vectors(&self, _: &[f64], _: usize) -> Vec<(u64, f64)> {
            vec![]
        }
        fn search_hybrid(&self, _: &str, _: &str, _: &[f64], _: usize, _: Option<f32>) -> Vec<(u64, f64)> {
            vec![]
        }
        fn flush(&self) -> Result<(), String> {
            Ok(())
        }
        fn bulk_check_permission(
            &self,
            _ctx: &async_graphql::dynamic::ResolverContext<'_>,
            _checks: Vec<(String, String, String)>,
        ) -> async_graphql::Result<Vec<(String, String, String, bool)>> {
            Ok(vec![])
        }
        fn compact(&self) -> Result<u64, String> {
            Ok(0)
        }
        fn needs_compaction(&self) -> bool {
            false
        }
        fn get_facet_counts(&self, _db_name: &str, _field: &str) -> Vec<(String, u64)> {
            vec![]
        }
    }

    let resolver = Box::new(MockMapResolver::default());
    let response_json = schema.execute_with_resolver(query, resolver).await;

    // 4. Verify Result
    let response: Value =
        serde_json::from_str(&response_json).expect("Response should be valid JSON");
    let data = response
        .get("data")
        .expect("Response should have data field");
    let user = data.get("getUser").expect("Data should have getUser field");
    let name = user.get("name").expect("User should have name field");

    assert_eq!(
        name.as_str(),
        Some("World"),
        "Query should return 'World' from Mock"
    );
}
