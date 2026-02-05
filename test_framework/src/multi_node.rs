//! Multi-Node Testing Harness
//!
//! Spawns multiple VardaDB instances and tests synchronization between them.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use std::fs;
use std::io::Write;

use async_graphql::Value;
use tempfile::TempDir;
use tokio::time::sleep;

/// Handle to a running VardaDB node
#[allow(dead_code)]
pub struct NodeHandle {
    pub port: u16,
    pub node_id: u64,
    pub data_dir: TempDir,  // Kept alive to prevent temp dir cleanup
    pub process: Child,
    client: reqwest::Client,
}

impl NodeHandle {
    /// Execute a GraphQL query/mutation on this node
    pub async fn execute(&self, query: &str) -> Result<Value, String> {
        let url = format!("http://localhost:{}/graphql", self.port);
        
        // Build proper JSON request body using serde_json
        let request_body = serde_json::json!({
            "query": query
        });
        
        let response = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;
        
        let text = response.text().await.map_err(|e| e.to_string())?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("JSON parse error: {} - response: {}", e, text))?;
        
        if let Some(errors) = json.get("errors") {
            return Err(format!("GraphQL errors: {}", errors));
        }
        
        let data = json.get("data")
            .ok_or("No data in response")?;
        
        // Convert serde_json::Value to async_graphql::Value
        let value_str = serde_json::to_string(data).map_err(|e| e.to_string())?;
        let ag_value: Value = serde_json::from_str(&value_str)
            .map_err(|e| format!("async_graphql conversion error: {}", e))?;
        
        Ok(ag_value)
    }
    
    /// Check if the node is ready (can accept connections)
    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            let url = format!("http://localhost:{}/graphql", self.port);
            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() || resp.status().as_u16() == 400 {
                    // 400 is OK - means server is up but needs a POST
                    return Ok(());
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        Err(format!("Node {} not ready after {:?}", self.port, timeout))
    }
}

impl Drop for NodeHandle {
    fn drop(&mut self) {
        // Kill the process
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Multi-node test harness
pub struct MultiNodeHarness {
    pub nodes: Vec<NodeHandle>,
}

impl MultiNodeHarness {
    /// Create a new multi-node harness with the specified number of nodes
    pub async fn new(num_nodes: usize, base_port: u16, sdl: &str) -> Result<Self, String> {
        let mut nodes = Vec::new();
        
        // Find VardaDB binary
        let vardadb_bin = Self::find_vardadb_binary()?;
        
        for i in 0..num_nodes {
            let port = base_port + i as u16;
            let node_id = (i + 1) as u64;
            
            // Create temp directory for this node's data
            let data_dir = TempDir::new()
                .map_err(|e| format!("Failed to create temp dir: {}", e))?;
            
            // Write schema file to temp dir
            let schema_path = data_dir.path().join("schema.graphql");
            let mut schema_file = fs::File::create(&schema_path)
                .map_err(|e| format!("Failed to create schema file: {}", e))?;
            schema_file.write_all(sdl.as_bytes())
                .map_err(|e| format!("Failed to write schema: {}", e))?;
            
            // Create a minimal config file
            // All nodes in this harness share the same Zenoh prefix so they can sync
            let config_path = data_dir.path().join("config.toml");
            let config_content = format!(r#"
[server]
port = {}
storage_path = "{}"
node_id = {}

[zenoh]
mode = "peer"
prefix = "varda/test/{}"
"#, 
                port,
                data_dir.path().join("data").display(),
                node_id,
                base_port  // All nodes in this test share the same prefix for sync!
            );
            
            fs::write(&config_path, config_content)
                .map_err(|e| format!("Failed to write config: {}", e))?;
            
            // Create data directory
            fs::create_dir_all(data_dir.path().join("data"))
                .map_err(|e| format!("Failed to create data dir: {}", e))?;
            
            // Spawn VardaDB process
            let process = Command::new(&vardadb_bin)
                .arg("--config")
                .arg(config_path.to_str().unwrap())
                .arg("--port")
                .arg(port.to_string())
                .arg("--data-dir")
                .arg(data_dir.path().join("data").to_str().unwrap())
                .arg("--schema")
                .arg(schema_path.to_str().unwrap())
                .arg("--node-id")
                .arg(node_id.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("Failed to spawn VardaDB: {}", e))?;
            
            nodes.push(NodeHandle {
                port,
                node_id,
                data_dir,
                process,
                client: reqwest::Client::new(),
            });
        }
        
        // Wait for all nodes to be ready
        for node in &nodes {
            node.wait_ready(Duration::from_secs(10)).await?;
        }
        
        // Give Zenoh a moment to establish peer connections
        sleep(Duration::from_millis(500)).await;
        
        Ok(Self { nodes })
    }
    
    /// Find the VardaDB binary
    fn find_vardadb_binary() -> Result<PathBuf, String> {
        // Try common locations
        let candidates = [
            // Debug build
            PathBuf::from("../target/debug/vardadb"),
            PathBuf::from("../../target/debug/vardadb"),
            // Release build
            PathBuf::from("../target/release/vardadb"),
            PathBuf::from("../../target/release/vardadb"),
            // In PATH
            PathBuf::from("vardadb"),
        ];
        
        for path in &candidates {
            if path.exists() {
                return Ok(path.clone());
            }
        }
        
        // Try using `which`
        if let Ok(output) = Command::new("which").arg("vardadb").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Ok(PathBuf::from(path));
            }
        }
        
        Err("VardaDB binary not found. Run `cargo build` in VardaDB root first.".to_string())
    }
    
    /// Execute on a specific node
    pub async fn execute(&self, node_idx: usize, query: &str) -> Result<Value, String> {
        self.nodes.get(node_idx)
            .ok_or_else(|| format!("Node {} not found", node_idx))?
            .execute(query)
            .await
    }
    
    /// Wait for sync to propagate across nodes
    pub async fn wait_for_sync(&self, timeout: Duration) {
        // Zenoh sync should be near-instant, but give it some buffer
        sleep(timeout.min(Duration::from_secs(2))).await;
    }
}
