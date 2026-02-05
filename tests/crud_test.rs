use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use std::sync::Arc;
use tempfile::tempdir;
use serde_json::Value;

#[tokio::test]
async fn test_crud_flow() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());
    
    // 1. Define Schema
    let sdl = "
        type User {
            id: ID!
            name: String
            email: String @unique
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(FjallResolver::new(storage.clone()));
    
    // 2. Create User
    let m1 = "mutation { createUser(input: {name: \"Alice\", email: \"alice@example.com\"}) { uid } }";
    let r1 = schema.execute_with_resolver(m1, resolver.clone()).await;
    let v1: Value = serde_json::from_str(&r1).unwrap();
    let user_id = v1["data"]["createUser"]["uid"].as_str().unwrap().to_string();
    println!("Created User: {}", user_id);

    // 3. Verify Create
    let q1 = format!("query {{ getUser(uid: \"{}\") {{ name email }} }}", user_id);
    let rq1 = schema.execute_with_resolver(&q1, resolver.clone()).await;
    let vq1: Value = serde_json::from_str(&rq1).unwrap();
    assert_eq!(vq1["data"]["getUser"]["name"], "Alice");

    // 4. Update User (Partial Update)
    let m2 = format!(
        "mutation {{ updateUser(uid: \"{}\", input: {{name: \"Alicia\"}}) }}",
        user_id
    );
    let r2 = schema.execute_with_resolver(&m2, resolver.clone()).await;
    let v2: Value = serde_json::from_str(&r2).unwrap();
    assert_eq!(v2["data"]["updateUser"], true);

    // 5. Verify Update
    let q2 = format!("query {{ getUser(uid: \"{}\") {{ name email }} }}", user_id);
    let rq2 = schema.execute_with_resolver(&q2, resolver.clone()).await;
    let vq2: Value = serde_json::from_str(&rq2).unwrap();
    assert_eq!(vq2["data"]["getUser"]["name"], "Alicia");
    assert_eq!(vq2["data"]["getUser"]["email"], "alice@example.com", "Email should remain unchanged");

    // 6. Test Unique Constraint Update (Fail Duplicate)
    // Create another user
    let m3 = "mutation { createUser(input: {name: \"Bob\", email: \"bob@example.com\"}) { uid } }";
    schema.execute_with_resolver(m3, resolver.clone()).await;

    // Try to update Alice's email to Bob's
    let m4 = format!(
        "mutation {{ updateUser(uid: \"{}\", input: {{email: \"bob@example.com\"}}) }}",
        user_id
    );
    let r4 = schema.execute_with_resolver(&m4, resolver.clone()).await;
    let v4: Value = serde_json::from_str(&r4).unwrap();
    assert!(v4["errors"].is_array());
    println!("Caught expected error: {}", v4["errors"][0]["message"]);

    // 7. Delete User
    let m5 = format!("mutation {{ deleteUser(uid: \"{}\") }}", user_id);
    let r5 = schema.execute_with_resolver(&m5, resolver.clone()).await;
    let v5: Value = serde_json::from_str(&r5).unwrap();
    assert_eq!(v5["data"]["deleteUser"], true);

    // 8. Verify Delete
    let q3 = format!("query {{ getUser(uid: \"{}\") {{ name }} }}", user_id);
    let rq3 = schema.execute_with_resolver(&q3, resolver.clone()).await;
    let vq3: Value = serde_json::from_str(&rq3).unwrap();
    assert!(vq3["data"]["getUser"].is_null());

    // 9. Verify Unique Index Removed (We should be able to reuse email "alice@example.com")
    let m6 = "mutation { createUser(input: {name: \"Alice 2\", email: \"alice@example.com\"}) { uid } }";
    let r6 = schema.execute_with_resolver(m6, resolver.clone()).await;
    let v6: Value = serde_json::from_str(&r6).unwrap();
    assert!(!v6["data"]["createUser"].is_null());
}
