use std::sync::Arc;
use tempfile::TempDir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_vector_api_search() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());

    // 1. Inject Data: 3 Vectors
    let mut v1 = vec![0.0; 384];
    v1[0] = 1.0;
    let mut v2 = vec![0.0; 384];
    v2[0] = 0.9;
    v2[1] = 0.1;
    let mut v3 = vec![0.0; 384];
    v3[1] = 1.0;

    storage.put_vector("default", 100, v1.clone()).unwrap();
    storage.put_vector("default", 101, v2).unwrap();
    storage.put_vector("default", 102, v3).unwrap();

    // Give async vector worker time to insert
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 2. Initialize Schema
    let resolver = SqliteResolver::new(storage.clone(), "default");
    let sdl = "type User { name: String }"; // Minimal SDL
    let schema = Schema::load_with_resolver(sdl, resolver).unwrap();

    // 3. Search for neighbors of v1
    let query_vector = format!("{:?}", v1);
    let query = format!(
        r#"
        query {{
            search(vector: {}, k: 2) {{
                uid
                distance
            }}
        }}
    "#,
        query_vector
    );

    // We use execute_with_resolver but pass a NEW resolver (sharing the storage)
    // because execute_with_resolver requires passing a resolver instance.
    // In a real app, the server does this for every request.
    let req_resolver = SqliteResolver::new(storage.clone(), "default");
    let resp_json = schema
        .execute_with_resolver(&query, Box::new(req_resolver))
        .await;

    println!("Response: {}", resp_json);

    // 4. Verify Results
    let v: serde_json::Value = serde_json::from_str(&resp_json).unwrap();
    let data = v.get("data").expect("No data in response");
    let search = data
        .get("search")
        .expect("No search field")
        .as_array()
        .expect("Search is not list");

    assert_eq!(search.len(), 2);

    // First result should be 100 (distance ~0)
    let first = &search[0];
    let uid1 = first.get("uid").unwrap().as_str().unwrap();
    let dist1 = first.get("distance").unwrap().as_f64().unwrap();

    assert_eq!(uid1, "100");
    assert!(dist1 < 0.0001);

    // Second result should be 101 (distance small)
    let second = &search[1];
    let uid2 = second.get("uid").unwrap().as_str().unwrap();
    // let dist2 = second.get("distance").unwrap().as_f64().unwrap();

    assert_eq!(uid2, "101");
}
