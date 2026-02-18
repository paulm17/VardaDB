
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread")]
async fn test_vector_api_search() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    
    // 1. Inject Data: 3 Vectors
    storage.put_vector(100, vec![1.0, 0.0]).unwrap();
    storage.put_vector(101, vec![0.9, 0.1]).unwrap();
    storage.put_vector(102, vec![0.0, 1.0]).unwrap();

    // 2. Initialize Schema
    let resolver = FjallResolver::new(storage.clone(), "default");
    let sdl = "type User { name: String }"; // Minimal SDL
    let schema = Schema::load_with_resolver(sdl, resolver).unwrap();

    // 3. Search for neighbors of [1.0, 0.0]
    let query = r#"
        query {
            search(vector: [1.0, 0.0], k: 2) {
                uid
                distance
            }
        }
    "#;

    // We use execute_with_resolver but pass a NEW resolver (sharing the storage)
    // because execute_with_resolver requires passing a resolver instance.
    // In a real app, the server does this for every request.
    let req_resolver = FjallResolver::new(storage.clone(), "default");
    let resp_json = schema.execute_with_resolver(query, Box::new(req_resolver)).await;
    
    println!("Response: {}", resp_json);

    // 4. Verify Results
    let v: serde_json::Value = serde_json::from_str(&resp_json).unwrap();
    let data = v.get("data").expect("No data in response");
    let search = data.get("search").expect("No search field").as_array().expect("Search is not list");

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
