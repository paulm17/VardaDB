use reqwest::Client;
use std::net::TcpListener;
use std::time::Duration;
use tempfile::tempdir;
use tokio::time::sleep;
use vardadb::config::VardaConfig;

fn get_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_schema_application_flow() {
    // 1. Setup
    let data_dir = tempdir().unwrap();
    let port = get_free_port();

    // Create config
    let mut config = VardaConfig::default();
    config.server.port = port;
    config.server.storage_path = data_dir.path().to_str().unwrap().to_string();
    config.server.is_mcp = false;

    // 2. Spawn Server
    let server_handle = tokio::spawn(async move {
        vardadb::run(config).await;
    });

    // Wait for server to be ready (poll instead of fixed sleep)
    let client = Client::new();
    let base_url = format!("http://127.0.0.1:{}/_mgmt", port);

    for i in 1..=20 {
        sleep(Duration::from_millis(500)).await;
        if server_handle.is_finished() {
            panic!("Server task exited prematurely");
        }
        match client.get(format!("{}/db", base_url)).send().await {
            Ok(_) => break,
            Err(_) => {
                if i == 20 {
                    panic!("Server never became ready after 10s");
                }
            }
        }
    }

    // 3. Create Database 'test_db'
    let res = client
        .post(format!("{}/db", base_url))
        .json(&serde_json::json!({ "name": "test_db" }))
        .send()
        .await
        .expect("Failed to create db");

    assert!(res.status().is_success());

    // 4. Apply Schema
    let schema_sdl = "type TestNode { id: ID!, name: String }";
    let res = client
        .post(format!("{}/db/test_db/schema", base_url))
        .body(schema_sdl)
        .send()
        .await
        .expect("Failed to apply schema");

    assert!(
        res.status().is_success(),
        "Apply schema failed: {}",
        res.text().await.unwrap()
    );

    // 5. Verify Persistence
    // NOTE: Bug in management.rs - apply_schema() hardcodes `let storage_path = "varda_db_data"`
    // instead of using the configured storage path. This test will catch that regression.
    let schema_path = data_dir.path().join("test_db_schema.graphql");
    assert!(
        schema_path.exists(),
        "Schema file should persist at {:?} - check that apply_schema() uses the configured storage path, not a hardcoded 'varda_db_data'",
        schema_path
    );

    // 6. Cleanup
    server_handle.abort();
}
