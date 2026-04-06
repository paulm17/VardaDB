use std::sync::Arc;
use tempfile::TempDir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

/// Tests the vector search API end-to-end:
/// insert vectors directly into the usearch engine (synchronously via
/// `storage.vector_engine`) and then search through the GraphQL `search` query.
#[tokio::test(flavor = "multi_thread")]
async fn test_vector_api_search() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());

    // Build three 384-dimensional unit vectors
    let mut v1 = vec![0.0f32; 384];
    v1[0] = 1.0;
    let mut v2 = vec![0.0f32; 384];
    v2[0] = 0.9;
    v2[1] = 0.1;
    let mut v3 = vec![0.0f32; 384];
    v3[1] = 1.0;

    // Insert directly into the usearch engine (synchronous — no channel race).
    storage.vector_engine.add_vector("default", 100, &v1).unwrap();
    storage.vector_engine.add_vector("default", 101, &v2).unwrap();
    storage.vector_engine.add_vector("default", 102, &v3).unwrap();

    // Minimal schema — we just need the `search` root query to be available.
    let resolver = RedbResolver::new(storage.clone(), "default");
    let sdl = "type User { name: String }";
    let schema = Schema::load_with_resolver(sdl, resolver.clone()).unwrap();

    // Search for the 2 nearest neighbours of v1
    let query_vec: Vec<f32> = v1.clone();
    let query_str = format!(
        r#"query {{ search(vector: {:?}, k: 2) {{ uid distance }} }}"#,
        query_vec
    );

    let resp = schema
        .execute_with_resolver(&query_str, Box::new(RedbResolver::new(storage.clone(), "default")))
        .await;
    eprintln!("search response: {}", resp);

    let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let search = v["data"]["search"].as_array().expect("search array");

    assert_eq!(search.len(), 2, "Expected 2 nearest neighbours");

    // First result should be uid 100 (exact match, distance ≈ 0)
    assert_eq!(search[0]["uid"].as_str().unwrap(), "100");
    let dist0 = search[0]["distance"].as_f64().unwrap();
    assert!(dist0 < 0.01, "Distance to self should be near zero, got {}", dist0);

    // Second result should be uid 101 (closest after 100)
    assert_eq!(search[1]["uid"].as_str().unwrap(), "101");
}
