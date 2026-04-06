use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_sorting() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    // 1. Define Schema
    let sdl = "
        type User {
            id: ID
            name: String
            age: Int
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // 2. Create Users: Alice(30), Bob(20), Charlie(25)
    let ops = vec![("Alice", 30), ("Bob", 20), ("Charlie", 25)];

    for (name, age) in ops {
        let mutation = format!(
            "mutation {{ createUser(input: {{name: \"{}\", age: {}}}) {{ uid }} }}",
            name, age
        );
        schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
    }

    // 3. Sort by Age ASC (Expect: Bob, Charlie, Alice)
    let query_asc = "
        query {
            queryUser(sort: {age: ASC}) {
                name
                age
            }
        }
    ";
    let res_asc_json = schema
        .execute_with_resolver(query_asc, resolver.clone())
        .await;
    println!("ASC Response: {}", res_asc_json);
    let res_asc: Value = serde_json::from_str(&res_asc_json).unwrap();
    let users_asc = res_asc["data"]["queryUser"]
        .as_array()
        .expect("Expected array");
    assert_eq!(users_asc.len(), 3);
    assert_eq!(users_asc[0]["name"], "Bob");
    assert_eq!(users_asc[1]["name"], "Charlie");
    assert_eq!(users_asc[2]["name"], "Alice");

    // 4. Sort by Age DESC (Expect: Alice, Charlie, Bob)
    let query_desc = "
        query {
            queryUser(sort: {age: DESC}) {
                name
                age
            }
        }
    ";
    let res_desc_json = schema
        .execute_with_resolver(query_desc, resolver.clone())
        .await;
    let res_desc: Value = serde_json::from_str(&res_desc_json).unwrap();
    let users_desc = res_desc["data"]["queryUser"]
        .as_array()
        .expect("Expected array");
    assert_eq!(users_desc.len(), 3);
    assert_eq!(users_desc[0]["name"], "Alice");
    assert_eq!(users_desc[1]["name"], "Charlie");
    assert_eq!(users_desc[2]["name"], "Bob");

    // 5. Sort + Filter
    // Filter age > 22 (Alice, Charlie), Sort Name ASC (Alice, Charlie)
    let query_filter = "
        query {
            queryUser(filter: {age: {gt: 22}}, sort: {name: ASC}) {
                name
            }
        }
    ";
    let res_f_json = schema
        .execute_with_resolver(query_filter, resolver.clone())
        .await;
    let res_f: Value = serde_json::from_str(&res_f_json).unwrap();
    let users_f = res_f["data"]["queryUser"]
        .as_array()
        .expect("Expected array");
    assert_eq!(users_f.len(), 2);
    assert_eq!(users_f[0]["name"], "Alice");
    assert_eq!(users_f[1]["name"], "Charlie");

    // 6. Sort + Pagionation
    // Sort Age ASC (Bob, Charlie, Alice). First 1 (Bob). After (Bob's ID).
    // Getting Bob's ID first requires query.
    // Let's just do First 2.
    let query_limit = "
        query {
            queryUser(sort: {age: ASC}, first: 2) {
                name
            }
        }
    ";
    let res_limit_json = schema
        .execute_with_resolver(query_limit, resolver.clone())
        .await;
    let res_limit: Value = serde_json::from_str(&res_limit_json).unwrap();
    let users_limit = res_limit["data"]["queryUser"]
        .as_array()
        .expect("Expected array");
    assert_eq!(users_limit.len(), 2);
    assert_eq!(users_limit[0]["name"], "Bob");
    assert_eq!(users_limit[1]["name"], "Charlie");
}
