use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use std::sync::Arc;
use tempfile::tempdir;
use serde_json::Value;

#[tokio::test]
async fn test_filtering_flow() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());
    
    // 1. Define Schema
    let sdl = "
        type User {
            name: String
            role: String
            age: Int
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(FjallResolver::new(storage.clone()));
    
    // 2. Create Data
    // Alice (Admin), Bob (User), Alice (User)
    // Alice (Admin, 30), Bob (User, 20), Alice (User, 25)
    let ops = vec![
        ("Alice", "Admin", 30),
        ("Bob", "User", 20),
        ("Alice", "User", 25),
    ];
    
    for (name, role, age) in ops {
        let mutation = format!(
            "mutation {{ createUser(input: {{name: \"{}\", role: \"{}\", age: {}}}) {{ name }} }}",
            name, role, age
        );
        schema.execute_with_resolver(&mutation, resolver.clone()).await;
    }

    // 3. Query All (No Filter)
    let query_all = "
        query {
            queryUser {
                name
                role
            }
        }
    ";
    let res_all_json = schema.execute_with_resolver(query_all, resolver.clone()).await;
    let res_all: Value = serde_json::from_str(&res_all_json).unwrap();
    let all_users = res_all["data"]["queryUser"].as_array().expect("Expected array");
    assert_eq!(all_users.len(), 3);
    
    // 4. Filter by Name "Alice"
    let query_alice = "
        query {
            queryUser(filter: {name: {eq: \"Alice\"}}) {
                name
                role
            }
        }
    ";
    let res_alice_json = schema.execute_with_resolver(query_alice, resolver.clone()).await;
    let res_alice: Value = serde_json::from_str(&res_alice_json).unwrap();
    let alice_users = res_alice["data"]["queryUser"].as_array().expect("Expected array");
    assert_eq!(alice_users.len(), 2);
    
    // 5. Filter by Role "User"
    let query_user_role = "
        query {
            queryUser(filter: {role: {eq: \"User\"}}) {
                name
            }
        }
    ";
    let res_role_json = schema.execute_with_resolver(query_user_role, resolver.clone()).await;
    let res_role: Value = serde_json::from_str(&res_role_json).unwrap();
    let check_users = res_role["data"]["queryUser"].as_array().expect("Expected array");
    assert_eq!(check_users.len(), 2); // Bob and Alice(User)

    // 6. Filter by Age > 20
    let query_age = "
        query {
            queryUser(filter: {age: {gt: 20}}) {
                name
                age
            }
        }
    ";
    let res_age_json = schema.execute_with_resolver(query_age, resolver.clone()).await;
    let res_age: Value = serde_json::from_str(&res_age_json).unwrap();
    let age_users = res_age["data"]["queryUser"].as_array().expect("Expected array");
    // Should match Alice(30) and Alice(25). Bob(20) is not > 20.
    assert_eq!(age_users.len(), 2, "Expected 2 users with age > 20");
}
