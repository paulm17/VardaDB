use std::time::{Duration, Instant};
use tempfile::TempDir;

use vardadb::config::VardaConfig;
use vardadb::init_system;

use crate::{TestRunner, TestResult};

pub async fn run_blob_tests(runner: &mut TestRunner, config: crate::TestConfig, _seed: u64) {
    let start = Instant::now();
    let result = test_tus_upload_flow(config).await;
    runner.add_result(match result {
        Ok(_) => TestResult::pass("tus_upload_flow", "blob", start.elapsed()),
        Err(e) => TestResult::fail("tus_upload_flow", "blob", start.elapsed(), &e),
    });
}

async fn test_tus_upload_flow(test_config: crate::TestConfig) -> Result<(), String> {
    let temp_db = TempDir::new().map_err(|e| e.to_string())?;
    let temp_blobs = TempDir::new().map_err(|e| e.to_string())?;
    
    let mut config = VardaConfig::default();
    config.server.storage_path = temp_db.path().to_str().unwrap().to_string();
    config.server.blobs_path = Some(temp_blobs.path().to_str().unwrap().to_string());
    config.server.port = 9015; // use a fixed test port

    let (_state, app) = init_system(config).await;
    
    // Start server in background
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9015")
        .await
        .map_err(|e| e.to_string())?;
        
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    
    // Give server a moment to start
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    let client = reqwest::Client::new();
    let base_url = "http://127.0.0.1:9015";
    
    // Test directory
    let images_dir = std::path::PathBuf::from(test_config.blob_tests.images_dir);
    let mut entries = tokio::fs::read_dir(&images_dir).await.map_err(|e| format!("Failed to read dir {:?}: {}", images_dir, e))?;
    
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            println!("Testing upload for image: {}", filename);
            
            let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
            let len = bytes.len();
            
            // Encode filename in base64 for TUS standard
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            let b64_filename = STANDARD.encode(&filename);
            
            // 1. Initiate TUS Upload (POST /files)
            let res = client.post(&format!("{}/files/", base_url))
                .header("Tus-Resumable", "1.0.0")
                .header("Upload-Length", len.to_string())
                .header("Upload-Metadata", format!("filename {}", b64_filename))
                .send()
                .await
                .map_err(|e| e.to_string())?;
                
            if res.status() != 201 {
                return Err(format!("POST /files failed for {}: {}", filename, res.status()));
            }
            
            let location = res.headers().get("Location").unwrap().to_str().unwrap().to_string();
            
            // 2. Upload bytes (PATCH /files/:id)
            let patch_url = format!("{}{}", base_url, location);
            
            let res = client.patch(&patch_url)
                .header("Tus-Resumable", "1.0.0")
                .header("Upload-Offset", "0")
                .header("Content-Type", "application/offset+octet-stream")
                .body(bytes.clone())
                .send()
                .await
                .map_err(|e| e.to_string())?;
                
            if res.status() != 204 {
                return Err(format!("PATCH failed for {}: {}", filename, res.status()));
            }
            
            let file_url = res.headers().get("Varda-File-Url").unwrap().to_str().unwrap().to_string();
            
            // 3. GET /files/:id to verify content is served correctly
            let res = client.get(&format!("{}{}", base_url, file_url))
                .send()
                .await
                .map_err(|e| e.to_string())?;
                
            if res.status() != 200 {
                return Err(format!("GET file failed for {}: {}", filename, res.status()));
            }
            
            let downloaded_bytes = res.bytes().await.map_err(|e| e.to_string())?;
            if downloaded_bytes != bytes {
                return Err(format!("Content mismatch for {}", filename));
            }
            
            // 4. GraphQL Query to verify metadata was saved in the graph
            let tus_id = location.trim_start_matches("/files/");
            
            let graphql_query = serde_json::json!({
                "query": format!(r#"
                    query {{
                        getFileRef(id: "{}") {{
                            id
                            contentHash
                            status
                        }}
                    }}
                "#, tus_id)
            });

            let res = client.post(&format!("{}/graphql", base_url))
                .json(&graphql_query)
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if res.status() != 200 {
                 return Err(format!("GraphQL query failed for {}: {}", filename, res.status()));
            }
            
            let json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
            
            let get_file_ref = json.get("data")
                .and_then(|d| d.get("getFileRef"))
                .unwrap_or(&serde_json::Value::Null);
                
            if get_file_ref.is_null() {
                return Err(format!("FileRef not found in graph for id {}", tus_id));
            }
            
            let status = get_file_ref.get("status").and_then(|s| s.as_str()).unwrap_or("");
            if status != "STAGED" {
                return Err(format!("Expected status STAGED for {}, got {}", filename, status));
            }
        }
    }
    
    Ok(())
}
