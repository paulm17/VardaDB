use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use std::sync::Arc;
use tempfile::tempdir;
use serde_json::Value;

#[tokio::test(flavor = "multi_thread")]
async fn test_unique_indexing() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());
    
    // 1. Define Schema with @unique
    let sdl = "
        type User {
            name: String @unique
            email: String
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    
    // 2. Create Resolver
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));
    
    // 3. Create User "Alice"
    let mutation = "
        mutation {
            createUser(input: {name: \"Alice\", email: \"alice@example.com\"}) {
                name
            } 
        }
    "; 
    let res_json = schema.execute_with_resolver(mutation, resolver.clone()).await;
    println!("Mutation 1: {}", res_json);
    assert!(!res_json.contains("errors"));

    // 4. Query by Unique Field (name)
    // Note: id is NOT provided.
    let query = "
        query {
            getUser(name: \"Alice\") {
                email
            }
        }
    ";
    let res_json = schema.execute_with_resolver(query, resolver.clone()).await;
    println!("Query 1: {}", res_json);
    
    let res: Value = serde_json::from_str(&res_json).unwrap();
    let data = res.get("data").expect("No data");
    let user = data.get("getUser").expect("No getUser");
    assert!(!user.is_null(), "User should be found by name");
    assert_eq!(user["email"], "alice@example.com");

    // 5. Try Duplicate Create
    let mutation_dup = "
        mutation {
            createUser(input: {name: \"Alice\", email: \"other@example.com\"}) {
                name
            } 
        }
    "; 
    let res_json = schema.execute_with_resolver(mutation_dup, resolver.clone()).await;
    println!("Mutation Dup: {}", res_json);
    
    assert!(res_json.contains("Duplicate value"), "Should fail with duplicate error");
}
