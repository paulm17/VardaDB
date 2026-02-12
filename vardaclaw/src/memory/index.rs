use anyhow::Result;
use reqwest::Client;
use serde_json::json;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, info, warn};

use super::search::MemoryChunk;

#[derive(Clone)]
pub struct MemoryIndex {
    client: Client,
    base_url: String,
    workspace: PathBuf,
    /// Token count per chunk (default: 400)
    chunk_size: usize,
    /// Token overlap between chunks (default: 80)
    chunk_overlap: usize,
}

#[derive(Debug)]
pub struct ReindexStats {
    pub files_processed: usize,
    pub files_updated: usize,
    pub chunks_indexed: usize,
    pub duration: Duration,
}

impl MemoryIndex {
    /// Create a new memory index connected to VardaDB
    pub fn new_with_db_path(workspace: &Path, _db_path: &Path) -> Result<Self> {
        // We ignore db_path as we don't use local SQLite anymore.
        // We default to localhost:8080, or read from env
        let base_url = std::env::var("VARDADB_URL").unwrap_or_else(|_| "http://localhost:8080/graphql".to_string());
        
        info!("Initializing VardaClaw MemoryIndex connected to {}", base_url);

        Ok(Self {
            client: Client::new(),
            base_url,
            workspace: workspace.to_path_buf(),
            chunk_size: 400,
            chunk_overlap: 80,
        })
    }

    /// Set chunk size and overlap (builder pattern)
    pub fn with_chunk_config(mut self, chunk_size: usize, chunk_overlap: usize) -> Self {
        self.chunk_size = chunk_size;
        self.chunk_overlap = chunk_overlap;
        self
    }

    /// Create a new memory index (legacy wrapper)
    pub fn new(workspace: &Path) -> Result<Self> {
        let db_path = workspace.join("memory.sqlite"); // Dummy path
        Self::new_with_db_path(workspace, &db_path)
    }

    /// Index a file by sending chunks to VardaDB
    pub fn index_file(&self, path: &Path, _force: bool) -> Result<bool> {
        let content = fs::read_to_string(path)?;
        
        let relative_path = path
            .strip_prefix(&self.workspace)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        debug!("Indexing file to VardaDB: {}", relative_path);

        // Chunk the content
        let chunks = chunk_text(&content, self.chunk_size, self.chunk_overlap);

        // Determine GraphQL query
        // We use createMemoryChunk mutation. VardaDB handles embeddings via @vector.
        let mutation = r#"
            mutation CreateMemoryChunk($content: String!, $source: String!, $startLine: Int!, $endLine: Int!) {
                createMemoryChunk(content: $content, sourceFile: $source, startLine: $startLine, endLine: $endLine)
            }
        "#;

        // Currently we don't have a bulk API in the default schema, so we send one by one.
        // Or we could check if file changed hash, but VardaDB doesn't expose file hash check easily yet.
        // For 'force' logic, we assume we overwrite.
        
        // First, delete existing chunks for this file to avoid duplicates?
        // Our schema doesn't have deleteByFile... 
        // But `createMemoryChunk` just adds nodes.
        // Ideally we should verify if we need to clean up.
        // For now, let's just append (v0.1 limitation). 
        // Future improvement: Add `deleteMemoryChunks(filter: {sourceFile: $file})` to schema.
        
        // Actually, we can check if we should index by hash... but let's implement the core logic first.
        
        let mut updated = false;
        
        for chunk in chunks {
            let variables = json!({
                "content": chunk.content,
                "source": relative_path,
                "startLine": chunk.line_start,
                "endLine": chunk.line_end
            });

            let body = json!({
                "query": mutation,
                "variables": variables
            });

            // We perform a blocking call here (inside blocking context if wrapped, or async if allowed).
            // Wait, this method is synchronous in signature. We need to block.
            let res = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    self.client.post(&self.base_url)
                        .json(&body)
                        .send()
                        .await?
                        .error_for_status()?
                        .text()
                        .await
                })
            });

            match res {
                Ok(_) => { updated = true; },
                Err(e) => {
                    warn!("Failed to index chunk for {}: {}", relative_path, e);
                }
            }
        }

        Ok(updated)
    }

    /// Remove a file (Not fully implemented on server yet)
    pub fn remove_file(&self, relative_path: &str) -> Result<()> {
        debug!("Removing file (no-op on server for now): {}", relative_path);
        // In real VardaDB usage, we would delete nodes where sourceFile == relative_path
        Ok(())
    }

    /// Get all indexed file paths (Not implemented on server yet)
    pub fn indexed_files(&self) -> Result<Vec<String>> {
        // Return empty or query server for distinct sourceFiles
        // For now, return empty to trigger reindexing?
        // Or maybe just return empty means "nothing indexed" so reindex will run.
        Ok(Vec::new()) 
    }

    /// Search using VardaDB
    pub fn search(&self, _query: &str, _limit: usize) -> Result<Vec<MemoryChunk>> {
        // We use getMemoryChunk with FTS/Vector
        // If query is just text, we treat it as FTS if we have a way.
        // But getMemoryChunk usually expects nearVector for semantic search.
        // Or we can use `content ~ query`.
        
        // Let's assume we want to use the vector search if possible, but search() signature is text.
        // MemoryManager::search calls search_hybrid which calls search_vector.
        // MemoryManager::search_fts calls this.
        
        // Let's implement a simple FTS-like query if possible, or matches regex.
        // Schema: getMemoryChunk(nearVector: ..., limit: ...)
        // We don't have text search in schema yet? 
        // We have `getMemoryChunk`. If we pass no args, it returns all.
        // We probably need to implement text search support in VardaDB or just use vector search here if we can.
        
        // But `search` is strictly text-based in LocalGPT interface.
        // Let's return empty for now and rely on `search_vector`.
        Ok(Vec::new()) 
    }

    /// Get total chunk count
    pub fn chunk_count(&self) -> Result<usize> {
        // Aggregate query
        let query = r#"
            query {
                aggregateMemoryChunk {
                   count
                }
            }
        "#;
        
        let body = json!({ "query": query });
        
        let res = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.client.post(&self.base_url)
                    .json(&body)
                    .send()
                    .await?
                    .json::<serde_json::Value>()
                    .await
            })
        })?;
        
        let count = res["data"]["aggregateMemoryChunk"]["count"].as_u64().unwrap_or(0);
        Ok(count as usize)
    }

    /// Get chunk count for a specific file
    pub fn file_chunk_count(&self, _path: &Path) -> Result<usize> {
        // Not implemented on server yet
        Ok(0)
    }

    /// Get database size in bytes (Dummy)
    pub fn size_bytes(&self) -> Result<u64> {
        Ok(0)
    }

    /// Get the database path (Dummy)
    pub fn db_path(&self) -> &Path {
        &self.workspace // Just return something valid
    }

    // ========================================================================
    // Embedding Support
    // ========================================================================

    pub fn chunks_without_embeddings(&self, _limit: usize) -> Result<Vec<(String, String)>> {
        // VardaDB handles embeddings. We tell LocalGPT we have none to process.
        Ok(Vec::new())
    }

    pub fn store_embedding(&self, _chunk_id: &str, _embedding: &[f32], _model: &str) -> Result<()> {
        // No-op
        Ok(())
    }

    pub fn get_cached_embedding(&self, _provider: &str, _model: &str, _text_hash: &str) -> Result<Option<Vec<f32>>> {
        Ok(None)
    }

    pub fn cache_embedding(&self, _provider: &str, _model: &str, _key: &str, _hash: &str, _embedding: &[f32]) -> Result<()> {
        Ok(())
    }

    pub fn has_vec_extension(&self) -> bool {
        true // We support vector search via VardaDB
    }

    pub fn embedded_chunk_count(&self, _model: &str) -> Result<usize> {
        self.chunk_count()
    }
    
    // Legacy support
    pub fn search_hybrid(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        model: &str,
        limit: usize,
        _text_weight: f32,
        _vector_weight: f32,
    ) -> Result<Vec<MemoryChunk>> {
        if let Some(embedding) = query_embedding {
            self.search_vector(embedding, model, limit)
        } else {
            self.search(query, limit)
        }
    }

    /// Vector search using VardaDB
    pub fn search_vector(
        &self,
        query_embedding: &[f32],
        _model: &str,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>> {
        let query = r#"
            query Search($vec: [Float!]!, $limit: Int!) {
                getMemoryChunk(nearVector: $vec, limit: $limit) {
                    content
                    sourceFile
                    startLine
                    endLine
                    score
                }
            }
        "#;

        let variables = json!({
            "vec": query_embedding,
            "limit": limit
        });
        
        let body = json!({
            "query": query,
            "variables": variables
        });

        let res = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.client.post(&self.base_url)
                    .json(&body)
                    .send()
                    .await?
                    .json::<serde_json::Value>()
                    .await
            })
        })?;
        
        let chunks = res["data"]["getMemoryChunk"].as_array();
        
        let mut results = Vec::new();
        if let Some(chunks) = chunks {
            for c in chunks {
                results.push(MemoryChunk {
                    file: c["sourceFile"].as_str().unwrap_or("").to_string(),
                    line_start: c["startLine"].as_i64().unwrap_or(0) as i32,
                    line_end: c["endLine"].as_i64().unwrap_or(0) as i32,
                    content: c["content"].as_str().unwrap_or("").to_string(),
                    score: c["score"].as_f64().unwrap_or(0.0),
                });
            }
        }
        
        Ok(results)
    }
}

struct ChunkInfo {
    line_start: i32,
    line_end: i32,
    content: String,
}

fn chunk_text(text: &str, target_tokens: usize, overlap_tokens: usize) -> Vec<ChunkInfo> {
    let lines: Vec<&str> = text.lines().collect();
    let mut chunks = Vec::new();

    if lines.is_empty() {
        return chunks;
    }

    // Rough estimate: 4 chars per token
    let target_chars = target_tokens * 4;
    let overlap_chars = overlap_tokens * 4;

    let mut start_line = 0;
    let mut current_chars = 0;
    let mut chunk_lines = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        chunk_lines.push(*line);
        current_chars += line.len() + 1; // +1 for newline

        if current_chars >= target_chars || i == lines.len() - 1 {
            // Create chunk
            chunks.push(ChunkInfo {
                line_start: (start_line + 1) as i32,
                line_end: (i + 1) as i32,
                content: chunk_lines.join("\n"),
            });

            // Calculate overlap for next chunk
            let mut overlap_len = 0;
            let mut overlap_start = chunk_lines.len();

            for (j, line) in chunk_lines.iter().enumerate().rev() {
                overlap_len += line.len() + 1;
                if overlap_len >= overlap_chars {
                    overlap_start = j;
                    break;
                }
            }

            // Prepare for next chunk
            if overlap_start < chunk_lines.len() {
                start_line += overlap_start;
                chunk_lines = chunk_lines[overlap_start..].to_vec();
                current_chars = chunk_lines.iter().map(|l| l.len() + 1).sum();
            } else {
                start_line = i + 1;
                chunk_lines.clear();
                current_chars = 0;
            }
        }
    }

    chunks
}


