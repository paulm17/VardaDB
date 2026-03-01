use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::config::PlannerConfig;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread")]
async fn test_query_planning_depth_limit() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    
    // Set a very strict planner limit
    let planner_config = Arc::new(PlannerConfig {
        enabled: true,
        mode: "enforce".to_string(),
        max_depth: 3,
        max_estimated_cost: 0.0,
        max_actual_cost: 0.0,
        default_list_size: 10,
    });

    // Initialize Schema
    let resolver = SqliteResolver::new(storage.clone(), "default");
    let sdl = r#"
        type User {
            id: ID!
            friends: [User!]!
        }
    "#;
    
    let schema = Schema::load_with_resolver_and_config(sdl, resolver, planner_config).unwrap();

    // Deep query (User -> friends -> friends -> friends = 4)
    let query = r#"
        query {
            getUser(id: "1") {
                friends {
                    friends {
                        id
                    }
                }
            }
        }
    "#;

    let req_resolver = SqliteResolver::new(storage.clone(), "default");
    let resp_json = schema.execute_with_resolver(query, Box::new(req_resolver)).await;
    
    let v: serde_json::Value = serde_json::from_str(&resp_json).unwrap();
    
    // Check that we got errors
    let errors = v.get("errors").expect("Response should contain errors");
    let error_array = errors.as_array().expect("Errors should be an array");
    assert!(!error_array.is_empty(), "Errors array should not be empty");
    
    let first_error = &error_array[0];
    let msg = first_error.get("message").unwrap().as_str().unwrap();
    
    // Message should look like "Query depth 4 exceeds maximum allowed depth of 3"
    assert!(msg.contains("depth"));
    assert!(msg.contains("exceeds"));
    
    let extensions = first_error.get("extensions").expect("Error should have extensions");
    let code = extensions.get("code").unwrap().as_str().unwrap();
    assert_eq!(code, "DEPTH_LIMIT_EXCEEDED");
}
