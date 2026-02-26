use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;

use anyhow::anyhow;

/// Default maximum concurrent HTTP requests
const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 32;

#[derive(Clone)]
pub struct VardaClient {
    client: Client,
    url: String,
    database: String,
    /// Semaphore to limit concurrent HTTP requests
    request_semaphore: Arc<Semaphore>,
}

#[derive(Serialize)]
struct GraphqlRequest {
    query: String,
    variables: Value,
}

#[derive(Deserialize)]
struct GraphqlResponse {
    data: Option<Value>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

impl VardaClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            url: url.into(),
            database: "default".to_string(),
            request_semaphore: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_REQUESTS)),
        }
    }

    /// Set the target database for requests
    pub fn with_database(mut self, database: impl Into<String>) -> Self {
        self.database = database.into();
        self
    }

    /// Set the maximum number of concurrent HTTP requests (default: 32)
    pub fn with_max_concurrent_requests(mut self, max: usize) -> Self {
        self.request_semaphore = Arc::new(Semaphore::new(max));
        self
    }

    pub async fn post_dynamic(
        &self,
        query: &str,
        variables: Value,
    ) -> anyhow::Result<Value> {
        use std::time::Instant;
        let call_start = Instant::now();
        
        // Acquire semaphore permit to limit concurrency
        let _permit = self.request_semaphore.acquire().await
            .map_err(|e| anyhow!("Semaphore error: {}", e))?;
        let sem_time = call_start.elapsed();

        let body = GraphqlRequest {
            query: query.to_string(),
            variables,
        };

        let http_start = Instant::now();
        let res = self.client.post(&self.url)
            .header("x-varda-db", &self.database)
            .json(&body)
            .send()
            .await?;
        let http_time = http_start.elapsed();
            
        let parse_start = Instant::now();
        let response_body: GraphqlResponse = res.json().await?;
        let parse_time = parse_start.elapsed();
        
        if let Some(errors) = response_body.errors {
            if let Some(first_error) = errors.first() {
                return Err(anyhow!("GraphQL Error: {}", first_error.message));
            }
        }

        let total = call_start.elapsed();
        let debug = std::env::var("VARDADB_DEBUG").map(|v| v == "1").unwrap_or(false);
        if debug && total.as_millis() > 20 {
            // Extract mutation/query name for readable logs
            let op_name = query.split('{').nth(1)
                .and_then(|s| s.split('(').next())
                .unwrap_or("unknown")
                .trim();
            eprintln!("[CLIENT] {} | sem={:?} http={:?} parse={:?} total={:?}",
                     op_name, sem_time, http_time, parse_time, total);
        }

        response_body.data.ok_or_else(|| anyhow!("No data returned"))
    }

    /// Triggers a database flush (memtable to SST) to reduce WAL size and recovery time.
    /// Should be called after bulk ingestion.
    pub async fn flush_database(&self) -> anyhow::Result<()> {
        let (query, _) = crate::GraphqlBuilder::new_mutation("flushDatabase")
            .build();
        
        self.post_dynamic(&query, serde_json::Value::Null).await?;
        Ok(())
    }

    /// Triggers explicit database compaction (blocking on server side).
    /// Returns the duration in milliseconds.
    /// Should be called periodically during bulk ingestion to prevent automatic compaction slowdowns.
    pub async fn compact(&self) -> anyhow::Result<i64> {
        let query = "mutation { compactDatabase }";
        let result = self.post_dynamic(query, serde_json::Value::Null).await?;
        result["compactDatabase"]
            .as_i64()
            .ok_or_else(|| anyhow!("No duration returned from compaction"))
    }

    /// Starts a bulk write session.
    /// The returned `BulkWriter` must be explicitly finished to ensure the database is flushed.
    pub fn start_bulk_write(&self) -> BulkWriter {
        BulkWriter::new(self.clone())
    }
}

pub struct BulkWriter {
    client: VardaClient,
    committed: bool,
}

impl BulkWriter {
    fn new(client: VardaClient) -> Self {
        Self {
            client,
            committed: false,
        }
    }

    /// Access the underlying client to perform operations.
    pub fn client(&self) -> &VardaClient {
        &self.client
    }

    /// Flushes the database and marks the bulk operation as complete.
    /// This must be called at the end of a bulk ingestion to ensure data is persisted to SSTables
    /// and the WAL is truncated.
    pub async fn finish(self) -> anyhow::Result<()> {
        let mut this = self;
        this.client.flush_database().await?;
        this.committed = true;
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BulkRecord {
    pub type_name: String,
    pub uid: Option<u64>,
    pub fields: std::collections::HashMap<String, serde_json::Value>,
}

pub struct TcpBulkWriter {
    stream: tokio::net::TcpStream,
}

impl TcpBulkWriter {
    pub async fn connect(host: &str, port: u16) -> anyhow::Result<Self> {
        let addr = format!("{}:{}", host, port);
        let stream = tokio::net::TcpStream::connect(&addr).await
            .map_err(|e| anyhow::anyhow!("Failed to connect to VardaDB Bulk Ingest port {}: {}", port, e))?;
            
        Ok(Self { stream })
    }
    
    /// Connect and send a database name handshake so the server routes
    /// all records into the correct keyspace.
    pub async fn connect_with_db(host: &str, port: u16, database: &str) -> anyhow::Result<Self> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        
        let addr = format!("{}:{}", host, port);
        let mut stream = tokio::net::TcpStream::connect(&addr).await
            .map_err(|e| anyhow::anyhow!("Failed to connect to VardaDB Bulk Ingest port {}: {}", port, e))?;
        
        // Send database name as first frame (length-prefixed UTF-8 string)
        let db_bytes = database.as_bytes();
        stream.write_u32_le(db_bytes.len() as u32).await?;
        stream.write_all(db_bytes).await?;
        
        // Wait for server ACK
        let mut ack = [0u8; 1];
        stream.read_exact(&mut ack).await
            .map_err(|e| anyhow::anyhow!("Failed reading db selection ACK: {}", e))?;
            
        Ok(Self { stream })
    }
    
    pub async fn ingest_batch(&mut self, batch: &[BulkRecord]) -> anyhow::Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        
        let bytes = serde_json::to_vec(batch)?;
        self.stream.write_u32_le(bytes.len() as u32).await?;
        self.stream.write_all(&bytes).await?;
        
        // Wait for VardaDB ACK backpressure (1 byte)
        let mut ack = [0u8; 1];
        self.stream.read_exact(&mut ack).await
            .map_err(|e| anyhow::anyhow!("Failed reading backpressure ACK from VardaDB: {}", e))?;
            
        Ok(())
    }
}

impl Drop for BulkWriter {
    fn drop(&mut self) {
        if !self.committed {
            eprintln!("WARNING: BulkWriter dropped without calling finish(). Database WAL may not be flushed. Data recovery time will be impacted.");
        }
    }
}
