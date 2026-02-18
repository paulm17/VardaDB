
use std::time::Duration;
use tokio::time::sleep;
use tempfile::tempdir;
use vardadb::config::VardaConfig;
use reqwest::Client;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_schema_application_flow() {
    // 1. Setup
    let data_dir = tempdir().unwrap();
    let port = 9123; // Hopefully free
    
    // Create config
    let mut config = VardaConfig::default();
    config.server.port = port;
    config.server.storage_path = data_dir.path().to_str().unwrap().to_string();
    config.server.is_mcp = false;
    
    // 2. Spawn Server
    let server_handle = tokio::spawn(async move {
        vardadb::run(config).await;
    });
    
    // Wait for server to start
    sleep(Duration::from_secs(2)).await;
    
    let client = Client::new();
    let base_url = format!("http://127.0.0.1:{}/_mgmt", port);
    
    // 3. Create Database 'test_db'
    let res = client.post(format!("{}/db", base_url))
        .json(&serde_json::json!({ "name": "test_db" }))
        .send()
        .await
        .expect("Failed to create db");
        
    assert!(res.status().is_success());
    
    // 4. Apply Schema
    let schema_sdl = "type TestNode { id: ID!, name: String }";
    let res = client.post(format!("{}/db/test_db/schema", base_url))
        .body(schema_sdl)
        .send()
        .await
        .expect("Failed to apply schema");
        
    assert!(res.status().is_success(), "Apply schema failed: {}", res.text().await.unwrap());
    
    // 5. Verify Persistence
    let schema_path = data_dir.path().join("test_db_schema.graphql");
    assert!(schema_path.exists(), "Schema file should persist at {:?}", schema_path);
    
    // 6. Cleanup
    // In `run()`: `let storage_path = config.server.storage_path.clone();`
    // In `apply_schema()`: `let storage_path = "varda_db_data";` <-- WAIT, THIS IS A BUG/ISSUE.
    // I noticed in my edit to `management.rs`:
    // `let storage_path = "varda_db_data";`
    // This ignores the configured storage path!
    
    // I should fix this bug before finishing the test.
    // But let's finish writing the test to reproducible fail if so.
    
    // 6. Cleanup
    server_handle.abort();
}
