use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_pagination_flow() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    // Add ID to schema
    let sdl = "
        type User {
            name: String
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    for i in 1..=5 {
        let mutation = format!(
            "mutation {{ createUser(input: {{name: \"User{}\"}}) {{ uid name }} }}",
            i
        );
        schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
        // Sleep to ensure order
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // 1. Page 1: First 2
    let query_p1 = "
        query {
            queryUser(first: 2) {
                uid
                name
            }
        }
    ";
    let res_p1 = schema
        .execute_with_resolver(query_p1, resolver.clone())
        .await;
    let v_p1: Value = serde_json::from_str(&res_p1).unwrap();
    let users_p1 = v_p1["data"]["queryUser"]
        .as_array()
        .expect("Expected array");
    assert_eq!(users_p1.len(), 2);
    assert_eq!(users_p1[0]["name"], "User1");
    assert_eq!(users_p1[1]["name"], "User2");

    let cursor = users_p1[1]["uid"].as_str().unwrap(); // Should be UID of User2

    // 2. Page 2: Next 2 after User2
    let query_p2 = format!(
        "query {{
            queryUser(first: 2, after: \"{}\") {{
                uid
                name
            }}
        }}",
        cursor
    );
    let res_p2 = schema
        .execute_with_resolver(&query_p2, resolver.clone())
        .await;
    let v_p2: Value = serde_json::from_str(&res_p2).unwrap();
    let users_p2 = v_p2["data"]["queryUser"]
        .as_array()
        .expect("Expected array");
    assert_eq!(users_p2.len(), 2);
    assert_eq!(users_p2[0]["name"], "User3");
    assert_eq!(users_p2[1]["name"], "User4");

    let cursor_2 = users_p2[1]["uid"].as_str().unwrap();

    // 3. Page 3: Next 2 after User4 (Should assume 1)
    let query_p3 = format!(
        "query {{
            queryUser(first: 2, after: \"{}\") {{
                uid
                name
            }}
        }}",
        cursor_2
    );
    let res_p3 = schema
        .execute_with_resolver(&query_p3, resolver.clone())
        .await;
    let v_p3: Value = serde_json::from_str(&res_p3).unwrap();
    let users_p3 = v_p3["data"]["queryUser"]
        .as_array()
        .expect("Expected array");
    assert_eq!(users_p3.len(), 1);
    assert_eq!(users_p3[0]["name"], "User5");
}
