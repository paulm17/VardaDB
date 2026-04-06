# VardaDB Production Search/Retrieval Uplift Plan

**Date**: April 6, 2026  
**Status**: Draft - Comprehensive gap analysis and remediation plan  
**Scope**: ./src/engine, ./src/storage, ./src/bridge layers  

---

## Executive Summary

**Purpose**: Transform VardaDB from a functional prototype into a production-grade system with **zero data loss tolerance**.

This document identifies gaps versus production requirements, organized into:
1. **Production Resilience (Issue 0)** - Data durability, backup/restore, crash safety
2. **Core Search Features (Issues 1-13)** - Functional parity with production vector databases

Each issue includes:
- **Severity**: Critical/High/Medium/Low
- **Effort**: Estimated implementation complexity
- **Files Affected**: Specific code locations
- **Implementation Plan**: Concrete steps to resolve
- **Verification Tests**: Concrete test cases to verify completion

**Critical for Production**: Issues 0, 0a, 0b, 1, 2, 12 must be completed before any production deployment.

---

## Issue 0: Production Resilience Gaps [CRITICAL - META]

This issue captures systemic gaps in the plan related to production data durability and reliability that must be addressed alongside the functional issues.

### Gap 0.1: Embedding Failure is Silent Data Loss
**Current Plan**: Issue 1 specifies "log and skip" for embedding failures.
**Problem**: A node with a failed embedding exists but is invisible to semantic search. No detection, no recovery.
**Fix Required**:
- Retry with exponential backoff (3 attempts)
- Failed embeddings go to dead-letter queue with retry count
- Health endpoint: `GET /health/embeddings` returns `{pending: N, failed: M, success_rate: 0.97}`
- Alert when success_rate < 0.99 for 5 minutes
- Admin mutation: `retryFailedEmbeddings(limit: 100)`

### Gap 0.2: Redb Durability Audit
**Status**: Must be completed in Phase 0

**Task**: Audit all redb write paths and enforce Durability::Immediate

**Checklist**:
1. [ ] Audit `src/storage/redb_backend.rs` - confirm `begin_write()` uses default durability
2. [ ] Audit `src/storage/backend.rs` - check all `put_with_lww`, `delete_with_lww` paths
3. [ ] Audit `src/bridge/redb_resolver.rs` - check all batch write operations
4. [ ] If any path uses `Durability::Eventual`, change to `Durability::Immediate`
5. [ ] Document in `docs/DURABILITY.md`: "All redb writes use Immediate durability (fsync on every commit)"

**Completion Criterion**: 
```rust
// This test passes - all writes use Immediate durability
#[test]
fn test_redb_uses_immediate_durability() {
    let storage = create_test_storage();
    let txn = storage.backend.db.begin_write().unwrap();
    // Verify durability level
    assert_eq!(txn.durability(), redb::Durability::Immediate);
}
```

**Time Estimate**: 1 day to audit, 1 day to fix/document

### Gap 0.3: Tantivy Durability is P2 but Should be P0
**Issue 8** batches deletes for performance. **This is wrong for no-data-loss.**
**Fix Required**:
- Default: commit-on-every-write (slow but safe)
- Optional: `batch_commits: true` for bulk ingest (explicit opt-in)
- Never auto-commit on timer (risk of losing recent writes)

### Gap 0.4: No Backup/Restore
**Status**: Completely missing from plan

**redb backup is file copy.** The `.redb` file must be copied while writes are paused for consistency.

**Required for Production**:
- Backup: `POST /admin/backup` (copy `.redb`, Tantivy dir, usearch file; pause writes briefly)
- Restore: `POST /admin/restore` (stop writes, replace files, restart)
- Export: `POST /admin/export` (optional, logical export for migration)
- Import: `POST /admin/import` (optional, logical import)

**Recovery is to backup point only.** For near-real-time recovery, use frequent backups (e.g., every 15 minutes).

### Gap 0.5: Crash Recovery Tests Are Insufficient
**Current**: Test 12.4 simulates one crash scenario
**Required**:
- Test partial write (power loss mid-transaction)
- Test corrupted redb file (can we detect and truncate?)
- Test concurrent crashes during batch operations
- Test disk-full scenarios
- Test recovery after segfault (no clean shutdown)

---

## Issue 1: Embedding Generation is Dead [CRITICAL]

### Current State
**File**: `src/bridge/redb_resolver.rs` (line 3674)

```rust
// Automatic embedding generation was removed with the local model backend.
if let Some(config) = vector_config {
    // HNSW Update
    if let Some(val) = fields.get(&config.field) {
        if let Value::List(list) = val {
            let vec_data: Vec<f64> = list
                .iter()
                .filter_map(|v| match v {
                    Value::Number(n) => n.as_f64(),
                    _ => None,
                })
                .collect();
```

**File**: `src/engine/resolver.rs` (lines 15-20)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorConfig {
    pub field: String,
    pub source: String,
    // Future: pub model: String
}
```

The VectorConfig has a commented-out `model` field, and the resolver comment explicitly states that automatic embedding generation was removed. Vectors must be supplied as raw float arrays by the caller.

### Gap Analysis
- **No @embedding(model: "...") directive**: Cannot specify an embedding model in the schema
- **No on-write text-to-vector pipeline**: Text fields are not automatically converted to vectors
- **No automatic re-embedding on field update**: Updates to source text fields don't trigger re-indexing
- **Raw vector store only**: The system is a vector store, not a vector database in the Weaviate/Qdrant sense

### Impact
Applications must handle all embedding generation externally. This creates a sharp edge for consumers of the schema API and limits adoption for use cases requiring seamless semantic search.

### Design Contract: One Node = One Chunk

**VardaDB generates exactly one embedding per node.** This is intentional and correct. The expected ingestion pattern is:

```
Source Document → Chunking (ingestion pipeline) → Multiple Nodes → One Embedding Each
```

**Example**:
```graphql
# Ingestion pipeline chunks a 10-page document into 3 parts
type DocumentChunk {
    chunk_id: ID! @unique
    parent_doc_id: String! @search(by: [exact])
    content: String @embedding(model: "all-MiniLM-L6-v2", target: "embedding")
    embedding: [Float!] @search(by: [hnsw])
    chunk_index: Int!
}
```

**If you store a whole document as a single node**, retrieval quality is your problem:
- A single 384-dimensional vector cannot meaningfully represent a 10-page document
- Semantic search will return the document for broad queries but miss specific sections
- The fix is in your ingestion pipeline (chunk before storing), not in VardaDB

**VardaDB is not responsible for chunking.** This is a deliberate architectural boundary:
- Chunking strategies vary (fixed tokens, semantic boundaries, paragraphs, sentences)
- Chunk size depends on use case (RAG vs classification vs search)
- Overlap/chunk boundaries are application-specific

**Implementation consequence**: The `@embedding` directive generates one vector for the field content. If that content is too large to embed meaningfully, chunk it before storing.

### Implementation Plan

**Phase 1: Reintroduce Local Embedding Backend (2-3 weeks)**

New File: `src/embedding/mod.rs`

Create traits and model registry for managing embedding backends. Implement OnnxEmbeddingModel using the `ort` crate for ONNX runtime support. Implement RemoteEmbeddingModel for OpenAI-compatible APIs. Add model caching layer for embeddings and create a background embedding worker thread similar to the existing vector worker.

**Phase 2: Schema Integration (1 week)**

Update GraphQL schema parsing in `src/engine/schema.rs` to support @embedding directive:

```graphql
type Document {
    content: String @embedding(
        model: "sentence-transformers/all-MiniLM-L6-v2", 
        target: "embedding"
    )
    embedding: [Float!] @search(by: [hnsw])
}
```

Changes:
1. Uncomment and activate model field in VectorConfig
2. Add target_field to specify where embeddings are stored
3. Parse @embedding directive during schema compilation
4. Validate that source and target fields exist with correct types

**Phase 3: Write-Time Pipeline (2 weeks)**

Modify create_node_internal and update_node_internal in src/bridge/redb_resolver.rs to trigger embedding generation when @embedding fields are modified.

**Failure Handling (Production-Grade):**
1. **Retry with exponential backoff** (3 attempts: immediate, 100ms, 1s)
2. **Dead Letter Queue (DLQ)**: Failed embeddings stored in `embedding_dlq` table
3. **Node write succeeds** regardless of embedding status (don't block on embedding)
4. **Background retry worker** processes DLQ every 30 seconds
5. **Health endpoint** exposes: pending count, failed count, success rate
6. **Alert threshold**: <99% success rate triggers warning

**Caching:**
- Cache embeddings by (model, text_hash) to avoid recomputation
- TTL: 1 hour for hot embeddings

**Total Effort**: 5-6 weeks

### Verification Tests

**Test 1.1: Schema Parsing**
```rust
#[test]
fn test_embedding_directive_parsed() {
    let schema = r#"
        type Document {
            content: String @embedding(model: "test-model", target: "embedding")
            embedding: [Float!] @search(by: [hnsw])
        }
    "#;
    let meta = parse_schema(schema).unwrap();
    assert_eq!(meta.vector_config.unwrap().model, "test-model");
    assert_eq!(meta.vector_config.unwrap().target_field, "embedding");
}
```

**Test 1.2: Embedding Generation on Create**
```rust
#[tokio::test]
async fn test_embedding_generated_on_create() {
    // Given schema with @embedding directive
    // When creating a node with text content
    let result = create_node("Document", json!({"content": "hello world"})).await;
    
    // Then the target field should have a vector automatically
    let node = get_node(result.uid).await;
    let embedding = node["embedding"].as_array().unwrap();
    assert!(embedding.len() > 0);
    assert_eq!(embedding.len(), 384); // MiniLM dimensions
}
```

**Test 1.3: Re-embedding on Update**
```rust
#[tokio::test]
async fn test_embedding_regenerated_on_update() {
    let uid = create_node("Document", json!({"content": "original text"})).await.uid;
    let old_embedding = get_node(uid)["embedding"].clone();
    
    // When updating the source text field
    update_node(uid, json!({"content": "completely different text"})).await;
    let new_embedding = get_node(uid)["embedding"].clone();
    
    // Then the embedding should be regenerated
    assert_ne!(old_embedding, new_embedding);
}
```

**Test 1.4: Batch Embedding Performance**
```rust
#[tokio::test]
async fn test_batch_embedding_performance() {
    let start = Instant::now();
    for i in 0..100 {
        create_node("Document", json!({"content": format!("text {}", i)})).await;
    }
    // Should complete in < 10 seconds for local model
    assert!(start.elapsed() < Duration::from_secs(10));
}
```

**Test 1.5: Embedding Failure Handling**
```rust
#[tokio::test]
async fn test_embedding_failure_does_not_fail_write() {
    // When embedding model fails (e.g., model file missing)
    // The write should still succeed (embedding skipped)
    let result = create_node("Document", json!({"content": "test"})).await;
    assert!(result.is_ok());
    // Error should be logged
}
```

**Test 1.6: Semantic Search Works**
```rust
#[tokio::test]
async fn test_semantic_search_returns_similar_results() {
    create_node("Document", json!({"content": "The quick brown fox"})).await;
    create_node("Document", json!({"content": "A fast brown dog"})).await;
    create_node("Document", json!({"content": "Stock market analysis"})).await;
    
    let query_vector = embed_text("fast animal");
    let results = search_vectors("Document", &query_vector, 3).await;
    
    assert_eq!(results[0].content, "A fast brown dog");
    assert_eq!(results[1].content, "The quick brown fox");
}
```

**Test 1.7: Different Models Have Different Dimensions**
```rust
#[test]
fn test_model_dimensions() {
    let mini_lm = ModelRegistry::get("sentence-transformers/all-MiniLM-L6-v2");
    assert_eq!(mini_lm.dimensions(), 384);
    
    let mpnet = ModelRegistry::get("sentence-transformers/all-mpnet-base-v2");
    assert_eq!(mpnet.dimensions(), 768);
}
```

**Test 1.8: Embedding Retry on Failure**
```rust
#[tokio::test]
async fn test_embedding_retries_on_failure() {
    // Mock model that fails twice then succeeds
    let mock_model = MockModel::with_failure_sequence(vec![true, true, false]);
    
    let result = generate_with_retry("test text", &mock_model, 3).await;
    
    assert!(result.is_ok());
    assert_eq!(mock_model.call_count(), 3); // Retried twice
}
```

**Test 1.9: Dead Letter Queue for Persistent Failures**
```rust
#[tokio::test]
async fn test_failed_embeddings_go_to_dlq() {
    // Mock model that always fails
    let mock_model = MockModel::always_fails();
    
    let uid = create_node_with_mock("Document", json!({"content": "test"}), &mock_model).await.uid;
    
    // Node should exist
    assert!(node_exists(uid));
    
    // But embedding should be in DLQ
    let dlq_entry = storage.get(b"embedding_dlq:Document:UID").unwrap();
    assert!(dlq_entry.is_some());
    assert_eq!(dlq_entry.retry_count, 3);
}
```

**Test 1.10: DLQ Retry Worker**
```rust
#[tokio::test]
async fn test_dlq_retry_worker() {
    // Add failed embedding to DLQ
    storage.put_dlq("Document", uid, "test text", retry_count: 3).unwrap();
    
    // Start retry worker
    let worker = EmbeddingRetryWorker::new(Duration::from_secs(1));
    worker.start().await;
    
    // Wait for retry
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Should be processed and removed from DLQ
    let dlq_entry = storage.get(b"embedding_dlq:Document:UID").unwrap();
    assert!(dlq_entry.is_none());
}
```

**Test 1.11: Health Endpoint Shows Failed Embeddings**
```rust
#[tokio::test]
async fn test_health_endpoint_shows_embedding_status() {
    // Create some failed embeddings
    for _ in 0..5 {
        add_to_dlq("Document").await;
    }
    
    let health = get("/health/embeddings").await;
    
    assert_eq!(health["failed"], 5);
    assert!(health["success_rate"] < 1.0);
}
```

---

## Issue 2: resolve_list Uses Brute-Force Vector Search [CRITICAL]

### Current State
**File**: `src/bridge/redb_resolver.rs` (lines 2867-2896)

```rust
if let Some(ref vec) = near_vector {
    let mut uid_dists = Vec::new();
    for uid in &uids {
        if let Some(Value::List(floats)) = self.resolve_cached(*uid, "embedding", cache) {
            let embed: Vec<f64> = floats
                .iter()
                .filter_map(|v| match v {
                    Value::Number(n) => n.as_f64(),
                    _ => None,
                })
                .collect();

            if embed.len() == vec.len() {
                let dot: f64 = embed.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
                let norm_a: f64 = embed.iter().map(|a| a * a).sum::<f64>().sqrt();
                let norm_b: f64 = vec.iter().map(|b| b * b).sum::<f64>().sqrt();

                if norm_a > 0.0 && norm_b > 0.0 {
                    let sim = dot / (norm_a * norm_b);
                    uid_dists.push((*uid, 1.0 - sim));
                }
            }
        }
    }
    uid_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    uids = uid_dists.into_iter().map(|(u, _)| u).collect();
}
```

### Gap Analysis
- **O(n) complexity**: For relationship-traversal queries with near_vector, it loads ALL related UIDs
- **No HNSW usage**: Unlike scan_nodes which correctly delegates to usearch, resolve_list computes cosine similarity in a loop
- **Scalability issue**: For small relationship sets it is fine, but does not scale beyond hundreds of edges

### Impact
Relationship queries with vector search become unusable at scale. A node with 10,000 related items will trigger 10,000 embedding lookups and cosine computations.

### Implementation Plan (1-2 weeks)

File: `src/bridge/redb_resolver.rs`

Replace brute-force loop with HNSW pre-filtering:

```rust
if let Some(ref vec) = near_vector {
    // Get candidate UIDs from relationship
    let related_set: HashSet<u64> = uids.iter().copied().collect();
    
    // Use HNSW to get nearest neighbors globally
    let vec_f32: Vec<f32> = vec.iter().map(|&x| x as f32).collect();
    let hnsw_results = self
        .storage
        .vector_engine
        .search(&self.db_name, &vec_f32, related_set.len() * 2);
    
    // Filter to relationship set while preserving HNSW order
    uids = hnsw_results
        .into_iter()
        .filter(|(uid, _)| related_set.contains(uid))
        .map(|(uid, _)| uid)
        .collect();
}
```

Alternative approach: Add vector index filtering support to usearch for constrained searches.

### Verification Tests

**Test 2.1: HNSW Used for Relationship Queries (Performance)**
```rust
#[tokio::test]
async fn test_resolve_list_uses_hnsw_not_brute_force() {
    // Given a node with 1000 related items
    let parent = create_node("Folder", json!({"name": "test"})).await;
    for i in 0..1000 {
        let child = create_node("Document", json!({"content": format!("doc {}", i)})).await;
        create_edge(parent.uid, "contains", child.uid).await;
    }
    
    // When querying with near_vector filter
    let start = Instant::now();
    let results = query(r#"
        query {
            getFolder(id: "UID") {
                contains(filter: {near_vector: {vector: [0.1, 0.2, ...]}}) {
                    content
                }
            }
        }
    "#).await;
    
    // Should complete in < 100ms (not O(n) scan)
    assert!(start.elapsed() < Duration::from_millis(100));
    assert_eq!(results.len(), 10); // Default limit
}
```

**Test 2.2: Correct Results from HNSW Pre-filter**
```rust
#[tokio::test]
async fn test_hnsw_prefilter_correctness() {
    // Create nodes with known vectors
    let uids: Vec<u64> = (0..100)
        .map(|i| create_node_with_vector("Item", i as f32).uid)
        .collect();
    
    let query_vector = vec![50.0f32];
    
    // Get results from brute force
    let manual_results = brute_force_search(&uids, &query_vector).await;
    
    // Get results from HNSW
    let hnsw_results = resolve_list_with_hnsw(&uids, &query_vector).await;
    
    // Top 10 should match (ordering may vary slightly due to approximate nature)
    assert_eq!(manual_results[..10].sort(), hnsw_results[..10].sort());
}
```

**Test 2.3: Empty Relationship Set**
```rust
#[tokio::test]
async fn test_resolve_list_empty_set() {
    let results = resolve_list_with_hnsw(&[], &query_vector).await;
    assert!(results.is_empty());
}
```

**Test 2.4: Single Item Relationship**
```rust
#[tokio::test]
async fn test_resolve_list_single_item() {
    let uid = create_node_with_vector("Item", 1.0).uid;
    let results = resolve_list_with_hnsw(&[uid], &query_vector).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], uid);
}
```

---

## Issue 3: No Query-Time Field Boosting or Multi-Field Search [HIGH]

### Current State
**File**: `src/storage/tantivy_search.rs` (lines 263-273)

```rust
pub fn search_bm25(
    &self,
    db_name: &str,
    query_text: &str,
    _field: &str,  // Single field only
    strategy: &str,
    k: usize,
    require_all: bool,
) -> Vec<(u64, f64)> {
```

The _field parameter only accepts a single field name. There is no mechanism to:
- Search across multiple fields simultaneously
- Apply different boost weights to different fields
- Use cross_fields or most_fields style queries

### Gap Analysis
- Single-field limitation: @search(by: [term]) on title and description cannot be scored differently
- No boost parameter: Cannot tune lexical relevance per query
- Elasticsearch-style multi_match is not available

### Implementation Plan (1-2 weeks)

File: `src/storage/tantivy_search.rs`

Extend search_bm25 to accept multiple fields with optional boosts:

```rust
pub struct FieldBoost {
    pub field: String,
    pub boost: f32,
}

pub fn search_bm25_multi(
    &self,
    db_name: &str,
    query_text: &str,
    fields: &[FieldBoost],
    strategy: &str,
    k: usize,
    require_all: bool,
) -> Vec<(u64, f64)> {
    // Build multi-field query with Tantivy's DisjunctionMaxQuery
    // or BooleanQuery with SHOULD clauses and field-specific boosts
}
```

Update GraphQL schema to support field weights:

```graphql
query {
    searchDocuments(
        filter: {
            anyoftext: "rust programming"
            fields: [
                {field: "title", boost: 3.0}
                {field: "description", boost: 1.0}
            ]
        }
    )
}
```

---

## Issue 4: No Phrase Queries or Proximity Search [HIGH]

### Current State
**File**: `src/storage/tantivy_search.rs` (lines 325-350)

```rust
let content_query: Box<dyn Query> = if require_all {
    // AND - every term is individually required
    let clauses: Vec<(Occur, Box<dyn Query>)> = terms
        .into_iter()
        .map(|t| {
            (
                Occur::Must,
                Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs)) as Box<dyn Query>,
            )
        })
        .collect();
    Box::new(BooleanQuery::new(clauses))
} else {
    // OR - at least one term must match
    let clauses: Vec<(Occur, Box<dyn Query>)> = terms
        .into_iter()
        .map(|t| {
            (
                Occur::Should,
                Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs)) as Box<dyn Query>,
            )
        })
        .collect();
    Box::new(BooleanQuery::new(clauses))
};
```

The current implementation only supports bag-of-words AND/OR semantics. TermQuery treats each term independently.

### Gap Analysis
- Only bag-of-words AND/OR is supported
- No PhraseQuery for exact phrase matching
- No slop-based proximity queries
- Searching for "graph database" matches documents containing both words anywhere, not just the exact phrase

### Implementation Plan (1-2 weeks)

Add phrase query support to Tantivy integration:

```rust
// New parameter in search_bm25
pub fn search_bm25(
    &self,
    db_name: &str,
    query_text: &str,
    _field: &str,
    strategy: &str,
    k: usize,
    require_all: bool,
    phrase_slop: Option<u32>, // NEW: slop for phrase queries
) -> Vec<(u64, f64)> {
```

Use Tantivy's PhraseQuery when the query is wrapped in quotes:

```rust
if query_text.starts_with('"') && query_text.ends_with('"') {
    // Use PhraseQuery for exact phrase matching
    let phrase_terms = parse_phrase_terms(query_text);
    let phrase_query = PhraseQuery::new(phrase_terms);
    // Optionally set slop for proximity matching
} else {
    // Use existing BooleanQuery with TermQueries
}
```

Update schema to expose phrase search:

```graphql
query {
    searchDocuments(filter: {phrase: "graph database"})  # Exact phrase
    searchDocuments(filter: {near: {terms: "graph database", slop: 2}})  # Proximity
}
```

---

## Issue 5: No Fuzzy / Edit-Distance Matching [HIGH]

### Current State
The Tantivy integration does not use FuzzyTermQuery. All term matches are exact after tokenization.

**File**: `src/storage/tantivy_search.rs` (lines 325-350)

Only TermQuery is used - no fuzzy matching exists.

### Gap Analysis
- Zero tolerance for typos
- No Levenshtein distance support
- Qdrant, Meilisearch, and Elasticsearch all offer fuzzy matching by default

### Implementation Plan (1 week)

Add fuzzy term query support:

```rust
use tantivy::query::FuzzyTermQuery;

pub fn search_bm25(
    &self,
    db_name: &str,
    query_text: &str,
    _field: &str,
    strategy: &str,
    k: usize,
    require_all: bool,
    fuzzy_distance: Option<u8>, // NEW: Levenshtein distance (0-2 typical)
) -> Vec<(u64, f64)> {
```

Build fuzzy queries when distance is specified:

```rust
let term_query: Box<dyn Query> = if let Some(distance) = fuzzy_distance {
    Box::new(FuzzyTermQuery::new(term, distance, true)) // true = prefix match
} else {
    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs))
};
```

GraphQL schema extension:

```graphql
query {
    searchDocuments(filter: {fuzzy: {terms: "database", distance: 1}})
    # Matches: database, databse, databaze, etc.
}
```

---



## Issue 3: No Query-Time Field Boosting or Multi-Field Search [HIGH]

### Current State
**File**: `src/storage/tantivy_search.rs` (lines 263-273)

The _field parameter only accepts a single field name. There is no mechanism to:
- Search across multiple fields simultaneously
- Apply different boost weights to different fields
- Use cross_fields or most_fields style queries

### Gap Analysis
- Single-field limitation: @search(by: [term]) on title and description cannot be scored differently
- No boost parameter: Cannot tune lexical relevance per query
- Elasticsearch-style multi_match is not available

### Implementation Plan (1-2 weeks)

File: `src/storage/tantivy_search.rs`

Extend search_bm25 to accept multiple fields with optional boosts:

```rust
pub struct FieldBoost {
    pub field: String,
    pub boost: f32,
}

pub fn search_bm25_multi(
    &self,
    db_name: &str,
    query_text: &str,
    fields: &[FieldBoost],
    strategy: &str,
    k: usize,
    require_all: bool,
) -> Vec<(u64, f64)>
```

Update GraphQL schema to support field weights with multiple fields and boost parameters.

### Verification Tests

**Test 3.1: Multi-Field Search**
```rust
#[tokio::test]
async fn test_multi_field_search() {
    create_node("Article", json!({
        "title": "Rust programming",
        "description": "A guide to Python"
    })).await;
    create_node("Article", json!({
        "title": "Python tutorial",
        "description": "Rust for beginners"
    })).await;
    
    // Search across both fields
    let results = search_bm25_multi(
        "default",
        "rust programming",
        &[
            FieldBoost { field: "title", boost: 1.0 },
            FieldBoost { field: "description", boost: 1.0 }
        ],
        "term",
        10,
        false
    ).await;
    
    assert!(results.len() >= 2);
}
```

**Test 3.2: Field Boosting Affects Ranking**
```rust
#[tokio::test]
async fn test_field_boost_affects_ranking() {
    // Doc A: matches in description only
    create_node("Article", json!({
        "title": "Other topic",
        "description": "Contains the search term here"
    })).await;
    
    // Doc B: matches in title only
    create_node("Article", json!({
        "title": "search term",
        "description": "Other content"
    })).await;
    
    // With title boost, Doc B should rank higher
    let results = search_bm25_multi(
        "default",
        "search term",
        &[
            FieldBoost { field: "title", boost: 3.0 },
            FieldBoost { field: "description", boost: 1.0 }
        ],
        "term",
        10,
        false
    ).await;
    
    assert!(results[0].score > results[1].score);
}
```

**Test 3.3: Single Field Still Works**
```rust
#[tokio::test]
async fn test_single_field_backward_compatible() {
    let results = search_bm25("default", "query", "title", "term", 10, false).await;
    assert!(!results.is_empty());
}
```

**Test 3.4: Empty Field List Returns Error**
```rust
#[tokio::test]
async fn test_empty_field_list_error() {
    let result = search_bm25_multi("default", "query", &[], "term", 10, false).await;
    assert!(result.is_err());
}
```

---

## Issue 4: No Phrase Queries or Proximity Search [HIGH]

### Current State
**File**: `src/storage/tantivy_search.rs` (lines 325-350)

The current implementation only supports bag-of-words AND/OR semantics using TermQuery.

### Gap Analysis
- Only bag-of-words AND/OR is supported
- No PhraseQuery for exact phrase matching
- No slop-based proximity queries
- Searching for "graph database" matches documents containing both words anywhere

### Implementation Plan (1-2 weeks)

Add phrase query support using Tantivy\'s PhraseQuery when queries are wrapped in quotes.

### Verification Tests

**Test 4.1: Exact Phrase Matching**
```rust
#[tokio::test]
async fn test_exact_phrase_matching() {
    create_node("Document", json!({"content": "graph database is great"})).await;
    create_node("Document", json!({"content": "database graph relationships"})).await;
    
    // Phrase query should only match first document
    let results = search_bm25("default", "\"graph database\"", "content", "fulltext", 10, false, None).await;
    
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("graph database"));
}
```

**Test 4.2: Phrase Not Matching Reversed Words**
```rust
#[tokio::test]
async fn test_phrase_order_matters() {
    create_node("Document", json!({"content": "quick brown fox"})).await;
    create_node("Document", json!({"content": "brown quick fox"})).await;
    
    let results = search_bm25("default", "\"quick brown\"", "content", "fulltext", 10, false, None).await;
    
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("quick brown"));
}
```

**Test 4.3: Proximity Search with Slop**
```rust
#[tokio::test]
async fn test_proximity_search() {
    create_node("Document", json!({"content": "the quick brown fox"})).await;      // "quick" and "fox" are 2 apart
    create_node("Document", json!({"content": "the quick lazy brown fox"})).await; // "quick" and "fox" are 3 apart
    
    // With slop=2, only first document should match
    let results = search_bm25("default", "quick fox", "content", "fulltext", 10, false, Some(2)).await;
    
    assert_eq!(results.len(), 1);
}
```

**Test 4.4: Regular Bag-of-Words Still Works**
```rust
#[tokio::test]
async fn test_regular_query_unaffected() {
    create_node("Document", json!({"content": "graph database"})).await;
    create_node("Document", json!({"content": "database graph"})).await;
    
    // Without quotes, both should match (bag-of-words)
    let results = search_bm25("default", "graph database", "content", "fulltext", 10, true, None).await;
    
    assert_eq!(results.len(), 2);
}
```

---

## Issue 5: No Fuzzy / Edit-Distance Matching [HIGH]

### Current State
The Tantivy integration does not use FuzzyTermQuery. All term matches are exact after tokenization.

### Gap Analysis
- Zero tolerance for typos
- No Levenshtein distance support
- Qdrant, Meilisearch, and Elasticsearch all offer fuzzy matching by default

### Implementation Plan (1 week)

Add fuzzy term query support using Tantivy\'s FuzzyTermQuery with configurable Levenshtein distance.

### Verification Tests

**Test 5.1: Typo Tolerance**
```rust
#[tokio::test]
async fn test_fuzzy_typo_tolerance() {
    create_node("Product", json!({"name": "database"})).await;
    
    // With fuzzy distance 1, "databse" (typo) should match "database"
    let results = search_bm25(
        "default",
        "databse",
        "name",
        "term",
        10,
        false,
        Some(1) // fuzzy_distance
    ).await;
    
    assert_eq!(results.len(), 1);
}
```

**Test 5.2: No Match Without Fuzzy**
```rust
#[tokio::test]
async fn test_no_fuzzy_no_typo_match() {
    create_node("Product", json!({"name": "database"})).await;
    
    // Without fuzzy, exact match required
    let results = search_bm25("default", "databse", "name", "term", 10, false, None).await;
    
    assert!(results.is_empty());
}
```

**Test 5.3: Distance 2 Matches More Variations**
```rust
#[tokio::test]
async fn test_fuzzy_distance_2() {
    create_node("Product", json!({"name": "database"})).await;
    
    // Distance 2 should match "databaze" (2 edits: s->z, add e)
    let results = search_bm25("default", "databaze", "name", "term", 10, false, Some(2)).await;
    
    assert_eq!(results.len(), 1);
}
```

**Test 5.4: Prefix Matching**
```rust
#[tokio::test]
async fn test_fuzzy_prefix_matching() {
    create_node("Product", json!({"name": "programming"})).await;
    
    // With prefix=true (default), "prog" should match
    let results = search_bm25("default", "prog", "name", "term", 10, false, Some(1)).await;
    
    assert_eq!(results.len(), 1);
}
```

---

## Issue 6: Geo Search Has No Spatial Index [HIGH]

### Current State
**File**: `src/bridge/redb_resolver.rs` (lines 1074-1134)

The near, within, and intersects filters work by loading candidate UIDs from the type index then iterating and computing haversine / point-in-polygon in Rust.

### Gap Analysis
- **Full scan**: No geohash prefix index, no R-tree, no H3/S2 cell index
- O(n) complexity for geo queries
- At any non-trivial dataset size this becomes a full table scan

### Implementation Plan (3-4 weeks)

**Phase 1: Geohash-based Spatial Index (2 weeks)**

Add geohash prefix keys to codec with new 0x0A prefix:

```rust
pub fn encode_geohash_index_key(geohash: &str, uid: u64) -> Vec<u8>
pub fn encode_geohash_prefix(geohash_prefix: &str) -> Vec<u8>
```

**Phase 2: Index Maintenance (1 week)**

Modify create_node_internal and update_node_internal to write geohash index entries when writing geo fields.

**Phase 3: Query Optimization (1 week)**

Modify geo filter processing to use geohash tiles for bounding box queries before applying precise haversine checks.

Alternative: Consider H3 or S2 cell indexing for better polygon support.

### Verification Tests

**Test 6.1: Geohash Index Created on Write**
```rust
#[tokio::test]
async fn test_geohash_index_created() {
    let uid = create_node("Place", json!({
        "location": {"lat": 51.5074, "lon": -0.1278}
    })).await.uid;
    
    // Check that geohash index keys exist
    let geohash = compute_geohash(51.5074, -0.1278, 8);
    for i in 1..=8 {
        let prefix = &geohash[..i];
        let key = Codec::encode_geohash_index_key(prefix, uid);
        assert!(storage.contains_key(&key).unwrap());
    }
}
```

**Test 6.2: Near Query Uses Spatial Index**
```rust
#[tokio::test]
async fn test_near_query_uses_spatial_index() {
    // Create 10000 random places
    for _ in 0..10000 {
        create_random_place().await;
    }
    
    // Query near London
    let start = Instant::now();
    let results = query(r#"
        query {
            searchPlaces(filter: {
                near: {
                    distance: 10000,
                    coordinate: {lat: 51.5074, lon: -0.1278}
                }
            }) { name }
        }
    "#).await;
    
    // Should complete quickly using spatial index (not full scan)
    assert!(start.elapsed() < Duration::from_millis(50));
    assert!(!results.is_empty());
}
```

**Test 6.3: Distance Calculation Accuracy**
```rust
#[test]
fn test_haversine_distance_accuracy() {
    let london = (51.5074, -0.1278);
    let paris = (48.8566, 2.3522);
    
    let distance = haversine_distance(london, paris);
    
    // London to Paris is approximately 344 km
    assert!((distance - 344000.0).abs() < 1000.0);
}
```

**Test 6.4: Within Polygon**
```rust
#[tokio::test]
async fn test_within_polygon() {
    create_node("Place", json!({
        "location": {"lat": 51.5, "lon": -0.1},
        "name": "Inside"
    })).await;
    create_node("Place", json!({
        "location": {"lat": 60.0, "lon": -10.0},
        "name": "Outside"
    })).await;
    
    let polygon = create_london_polygon();
    let results = query(r#"
        query {
            searchPlaces(filter: {within: {polygon: POLYGON}}) { name }
        }
    "#).await;
    
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "Inside");
}
```

---

## Issue 7: No Trigram Index Wired to Search [MEDIUM]

### Current State
**File**: `src/engine/tokenizer.rs` (lines 18-27)

The trigram tokenizer strategy exists but is NOT integrated with Tantivy or the KV index layer.

### Gap Analysis
- Tokenizer.rs has trigram strategy defined
- No codec prefix for trigram index keys
- No integration with search pipeline
- Cannot do efficient substring/LIKE queries
- Dead code that was never fully integrated

### Implementation Plan (1 week)

**Step 1**: Add Trigram Index Keys to Codec with 0x0B prefix

**Step 2**: Index Trigrams on Write in create_node_internal and update_node_internal

**Step 3**: Add contains() query support that computes trigrams and does index intersection

### Verification Tests

**Test 7.1: Trigram Index Created**
```rust
#[tokio::test]
async fn test_trigram_index_created() {
    let uid = create_node("Document", json!({
        "content": "hello",
        "_tokenizers": ["trigram"]
    })).await.uid;
    
    // Trigrams for "hello": "hel", "ell", "llo"
    for trigram in ["hel", "ell", "llo"] {
        let key = Codec::encode_trigram_index_key("content", trigram, uid);
        assert!(storage.contains_key(&key).unwrap());
    }
}
```

**Test 7.2: Contains Query Works**
```rust
#[tokio::test]
async fn test_contains_query() {
    create_node("Document", json!({"content": "graph database"})).await;
    create_node("Document", json!({"content": "graph theory"})).await;
    create_node("Document", json!({"content": "sql database"})).await;
    
    // Contains "graph" should match first two
    let results = search_contains("Document", "content", "graph").await;
    
    assert_eq!(results.len(), 2);
}
```

**Test 7.3: Substring Matching**
```rust
#[tokio::test]
async fn test_substring_matching() {
    create_node("Document", json!({"content": "programming in rust"})).await;
    
    // Should match "gram" substring
    let results = search_contains("Document", "content", "gram").await;
    
    assert_eq!(results.len(), 1);
}
```

**Test 7.4: Short Query (<3 chars) Returns Error**
```rust
#[tokio::test]
async fn test_short_contains_query_error() {
    let result = search_contains("Document", "content", "ab").await;
    assert!(result.is_err());
}
```

---

## Issue 8: Tantivy Commit-on-Every-Delete [MEDIUM]

### Current State
**File**: `src/storage/tantivy_search.rs` (lines 239-251)

Every field deletion triggers a Tantivy segment commit via writer.commit().

### Gap Analysis
- Write amplification problem on bulk ingestion or high-update workloads
- Correct for consistency but expensive
- No batch delete support

### Implementation Plan (1-2 weeks)

Solution: Batch deletes with controlled commit boundaries:

1. Add pending_deletes DashMap to SearchEngine
2. Queue deletes instead of immediate commit
3. Auto-flush when batch size threshold reached (e.g., 100 deletes)
4. Expose explicit flush_deletes() API for controlled commit boundaries
5. Call flush_deletes() in Storage::commit_all()

### Verification Tests

**Test 8.1: Deletes Are Batched**
```rust
#[tokio::test]
async fn test_deletes_are_batched() {
    let search_engine = create_test_search_engine();
    
    // Queue 50 deletes
    for i in 0..50 {
        search_engine.remove_document("default", i, "content").unwrap();
    }
    
    // Should not have committed yet
    assert_eq!(search_engine.pending_deletes.len(), 50);
}
```

**Test 8.2: Auto-Flush on Threshold**
```rust
#[tokio::test]
async fn test_auto_flush_on_threshold() {
    let search_engine = create_test_search_engine();
    
    // Queue 150 deletes (threshold is 100)
    for i in 0..150 {
        search_engine.remove_document("default", i, "content").unwrap();
    }
    
    // Should have auto-flushed
    assert!(search_engine.pending_deletes.len() < 100);
}
```

**Test 8.3: Explicit Flush Works**
```rust
#[tokio::test]
async fn test_explicit_flush() {
    let search_engine = create_test_search_engine();
    
    search_engine.remove_document("default", 1, "content").unwrap();
    assert_eq!(search_engine.pending_deletes.len(), 1);
    
    search_engine.flush_deletes("default").unwrap();
    assert_eq!(search_engine.pending_deletes.len(), 0);
}
```

**Test 8.4: Documents Actually Deleted After Flush**
```rust
#[tokio::test]
async fn test_documents_deleted_after_flush() {
    let search_engine = create_test_search_engine();
    
    // Index a document
    search_engine.index_document("default", 1, "content", "test data", "term").unwrap();
    search_engine.flush_all().unwrap();
    
    // Delete it
    search_engine.remove_document("default", 1, "content").unwrap();
    search_engine.flush_deletes("default").unwrap();
    
    // Search should not find it
    let results = search_engine.search_bm25("default", "test", "content", "term", 10, false);
    assert!(results.is_empty());
}
```

---

## Issue 9: No Search Result Highlighting or Snippet Extraction [MEDIUM]

### Current State
No way to return the matching context around a hit. Tantivy has a SnippetGenerator that is not being used.

### Gap Analysis
- No SnippetGenerator usage
- UIDs and scores only returned, nothing else
- Useful for any UI built on top
- Elasticsearch and Meilisearch both provide highlighting

### Implementation Plan (1 week)

**File**: `src/storage/tantivy_search.rs`

Add snippet generation to search_bm25:

```rust
use tantivy::snippet::SnippetGenerator;

pub struct SearchResult {
    pub uid: u64,
    pub score: f64,
    pub snippet: Option<String>,
    pub highlighted_terms: Vec<String>,
}

pub fn search_bm25_with_snippets(
    &self,
    db_name: &str,
    query_text: &str,
    _field: &str,
    strategy: &str,
    k: usize,
    require_all: bool,
) -> Vec<SearchResult>
```

Steps:
1. Store original text content in Tantivy (already done with set_stored())
2. Use SnippetGenerator to extract matching context
3. Return snippets alongside UIDs and scores
4. Update GraphQL schema to expose snippet field

### Verification Tests

**Test 9.1: Snippet Generated**
```rust
#[tokio::test]
async fn test_snippet_generated() {
    create_node("Document", json!({
        "content": "The quick brown fox jumps over the lazy dog"
    })).await;
    
    let results = search_bm25_with_snippets(
        "default",
        "quick fox",
        "content",
        "fulltext",
        10,
        false
    ).await;
    
    assert!(results[0].snippet.is_some());
    assert!(results[0].snippet.unwrap().contains("quick"));
}
```

**Test 9.2: Highlighted Terms Returned**
```rust
#[tokio::test]
async fn test_highlighted_terms() {
    create_node("Document", json!({"content": "rust programming language"})).await;
    
    let results = search_bm25_with_snippets(
        "default",
        "rust programming",
        "content",
        "fulltext",
        10,
        false
    ).await;
    
    assert!(results[0].highlighted_terms.contains(&"rust".to_string()));
    assert!(results[0].highlighted_terms.contains(&"programming".to_string()));
}
```

**Test 9.3: Snippet Context is Relevant**
```rust
#[tokio::test]
async fn test_snippet_context() {
    let long_text = "A".repeat(1000) + " important keyword here " + &"B".repeat(1000);
    create_node("Document", json!({"content": long_text})).await;
    
    let results = search_bm25_with_snippets(
        "default",
        "important keyword",
        "content",
        "fulltext",
        10,
        false
    ).await;
    
    // Snippet should contain the keyword and surrounding context
    let snippet = results[0].snippet.as_ref().unwrap();
    assert!(snippet.contains("important keyword"));
    assert!(snippet.len() < 300); // Reasonable snippet length
}
```

**Test 9.4: No Snippet When No Match**
```rust
#[tokio::test]
async fn test_no_snippet_when_no_match() {
    // Document doesn't contain search terms
    create_node("Document", json!({"content": "completely unrelated text"})).await;
    
    let results = search_bm25_with_snippets(
        "default",
        "missing term",
        "content",
        "fulltext",
        10,
        false
    ).await;
    
    // May still return document due to stemming/tokenization
    // but snippet should be None or empty
    if let Some(ref snippet) = results[0].snippet {
        assert!(snippet.is_empty() || snippet.contains("..."));
    }
}
```

---

## Issue 10: No Faceted Search / Aggregations [MEDIUM]

### Current State
No way to say "give me counts of results by field X" without issuing N queries.

### Gap Analysis
- No facet field support in Tantivy schema
- Cannot do faceted navigation (e.g., filter by category and see counts)
- No aggregation pipeline
- Elasticsearch\'s aggregation pipeline is the reference

### Implementation Plan (2-3 weeks)

**Phase 1: Tantivy Facet Support (1 week)**

Add facet fields to schema:

```rust
// In get_or_create()
let category_facet = sb.add_facet_field("category", FacetOptions::default());
```

**Phase 2: Facet Indexing (1 week)**

Modify index_document to handle facet fields:

```rust
pub fn index_facet(&self, db_name: &str, uid: u64, field: &str, value: &str)
```

**Phase 3: Facet Queries (1 week)**

Add facet counting API:

```rust
pub fn get_facet_counts(
    &self,
    db_name: &str,
    field: &str,
    prefix: Option<&str>,
) -> Vec<(String, u64)>
```

GraphQL extension:

```graphql
query {
    searchProducts(filter: {price: {lt: 100}}) {
        items { name price }
        facets {
            category { value count }
            brand { value count }
        }
    }
}
```

### Verification Tests

**Test 10.1: Facet Counts Returned**
```rust
#[tokio::test]
async fn test_facet_counts() {
    create_node("Product", json!({"name": "A", "category": "Electronics"})).await;
    create_node("Product", json!({"name": "B", "category": "Electronics"})).await;
    create_node("Product", json?({"name": "C", "category": "Books"})).await;
    
    let facets = get_facet_counts("default", "category", None).await;
    
    assert_eq!(facets["Electronics"], 2);
    assert_eq!(facets["Books"], 1);
}
```

**Test 10.2: Facets with Filter**
```rust
#[tokio::test]
async fn test_facets_with_filter() {
    create_node("Product", json!({"name": "A", "category": "Electronics", "price": 100})).await;
    create_node("Product", json!({"name": "B", "category": "Electronics", "price": 200})).await;
    create_node("Product", json!({"name": "C", "category": "Books", "price": 50})).await;
    
    // Get facets for products under $150
    let facets = query(r#"
        query {
            searchProducts(filter: {price: {lt: 150}}) {
                facets { category { value count } }
            }
        }
    "#).await;
    
    assert_eq!(facets["category"]["Electronics"], 1);
    assert_eq!(facets["category"]["Books"], 1);
}
```

**Test 10.3: Hierarchical Facets**
```rust
#[tokio::test]
async fn test_hierarchical_facets() {
    create_node("Product", json!({"category": "/Electronics/Phones"})).await;
    create_node("Product", json?({"category": "/Electronics/Laptops"})).await;
    create_node("Product", json?({"category": "/Books/Fiction"})).await;
    
    let facets = get_facet_counts("default", "category", Some("/Electronics")).await;
    
    assert_eq!(facets.len(), 2);
    assert!(facets.contains_key("/Electronics/Phones"));
    assert!(facets.contains_key("/Electronics/Laptops"));
}
```

**Test 10.4: Empty Facets for Non-Existent Field**
```rust
#[tokio::test]
async fn test_empty_facets_nonexistent_field() {
    let facets = get_facet_counts("default", "nonexistent_field", None).await;
    assert!(facets.is_empty());
}
```

---

## Issue 11: RRF Weights Both Legs Equally [MEDIUM]

### Current State
**File**: `src/bridge/redb_resolver.rs` (lines 909-942)

```rust
pub fn search_hybrid(
    &self,
    text_query: &str,
    field: &str,
    vector: &[f64],
    k: usize,
    require_all: bool,
) -> Vec<(u64, f64)> {
    // BM25 results (over-fetch then fuse)
    let text_results = self.search_text_bm25(text_query, field, "fulltext", k * 2, require_all);

    // ANN results (over-fetch then fuse)
    let vec_f32: Vec<f32> = vector.iter().map(|&x| x as f32).collect();
    let vec_results = self
        .storage
        .vector_engine
        .search(&self.db_name, &vec_f32, k * 2);

    // Reciprocal Rank Fusion
    let mut scores: std::collections::HashMap<u64, f64> = std::collections::HashMap::new();
    for (rank, (uid, _)) in text_results.iter().enumerate() {
        *scores.entry(*uid).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
    }
    for (rank, (uid, _)) in vec_results.iter().enumerate() {
        *scores.entry(*uid).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
    }
    // ...
}
```

Both BM25 rank and ANN rank use identical 1/(60 + rank) weight.

### Gap Analysis
- Both legs weighted equally in RRF
- No way to tune lexical vs semantic weight per query or field
- Weaviate and Qdrant expose an alpha parameter for this
- search_hybrid in resolver.rs has require_all=false hardcoded

### Implementation Plan (1 week)

Add alpha parameter to hybrid search:

```rust
pub fn search_hybrid(
    &self,
    text_query: &str,
    field: &str,
    vector: &[f64],
    k: usize,
    require_all: bool,
    alpha: Option<f32>, // NEW: 0.0 = all BM25, 1.0 = all vector, default 0.5
) -> Vec<(u64, f64)> {
    let alpha = alpha.unwrap_or(0.5);
    let text_weight = 1.0 - alpha;
    let vector_weight = alpha;
    
    // Apply weighted RRF
    for (rank, (uid, _)) in text_results.iter().enumerate() {
        *scores.entry(*uid).or_default() += text_weight / (60.0 + rank as f64 + 1.0);
    }
    for (rank, (uid, _)) in vec_results.iter().enumerate() {
        *scores.entry(*uid).or_default() += vector_weight / (60.0 + rank as f64 + 1.0);
    }
}
```

GraphQL extension:

```graphql
query {
    searchDocuments(
        filter: {
            near_vector: {vector: [0.1, 0.2, ...], alpha: 0.7}
            anyoftext: "rust programming"
        }
    )
}
```

Also thread require_all parameter through the trait method.

### Verification Tests

**Test 11.1: Alpha 0.0 All BM25**
```rust
#[tokio::test]
async fn test_alpha_zero_all_bm25() {
    // Create documents that match BM25 but not vector
    create_test_documents().await;
    
    let text_query = "rust programming";
    let vector_query = vec![0.0; 384]; // Unrelated vector
    
    // With alpha=0.0, should only use BM25
    let results = search_hybrid(
        text_query,
        "content",
        &vector_query,
        10,
        false,
        Some(0.0)
    ).await;
    
    // Should get BM25 results even with random vector
    assert!(!results.is_empty());
}
```

**Test 11.2: Alpha 1.0 All Vector**
```rust
#[tokio::test]
async fn test_alpha_one_all_vector() {
    create_test_documents().await;
    
    let text_query = "gibberish nonsense"; // No BM25 match
    let vector_query = get_embedding("rust programming");
    
    // With alpha=1.0, should only use vector
    let results = search_hybrid(
        text_query,
        "content",
        &vector_query,
        10,
        false,
        Some(1.0)
    ).await;
    
    // Should get vector results even with nonsense text
    assert!(!results.is_empty());
}
```

**Test 11.3: Alpha 0.5 Balanced**
```rust
#[tokio::test]
async fn test_alpha_half_balanced() {
    create_test_documents().await;
    
    let text_query = "rust";
    let vector_query = get_embedding("rust programming");
    
    // With alpha=0.5, both contribute equally
    let results = search_hybrid(
        text_query,
        "content",
        &vector_query,
        10,
        false,
        Some(0.5)
    ).await;
    
    // Should get different ranking than pure BM25 or pure vector
    let bm25_only = search_bm25("default", "rust", "content", "fulltext", 10, false).await;
    let vec_only = search_vectors("default", &vector_query, 10).await;
    
    // Hybrid ranking should differ from both
    assert_ne!(results.iter().map(|r| r.uid).collect::<Vec<_>>(),
               bm25_only.iter().map(|r| r.uid).collect::<Vec<_>>());
}
```

**Test 11.4: Require All Threaded Through**
```rust
#[tokio::test]
async fn test_require_all_threaded() {
    create_test_documents().await;
    
    // With require_all=true, should only return documents matching both text AND vector
    let results_and = search_hybrid("rust", "content", &vector, 10, true, Some(0.5)).await;
    let results_or = search_hybrid("rust", "content", &vector, 10, false, Some(0.5)).await;
    
    // AND should return fewer results than OR
    assert!(results_and.len() <= results_or.len());
}
```

---

## Issue 12: Vector Index Is Not Persisted Transactionally [CRITICAL]

### Current State
**File**: `src/storage/vector_engine.rs` (lines 212-235)

usearch saves to disk only on flush() / save_all(). Between flushes, if the process crashes:
- Vectors written to usearch but not yet saved are lost
- Corresponding redb records survive (redb is ACID)
- The two stores can diverge

### Gap Analysis
- No reconciliation path on startup
- No "on startup, re-index any UIDs in redb whose vector is missing from usearch"
- usearch persists separately from redb - can lose vectors on crash
- Need reconciliation mechanism between redb and usearch

### Implementation Plan (2-3 weeks)

**Phase 1: Vector Pending Queue (1 week)**

Since usearch indexes asynchronously, vectors can be lost if the process crashes between the redb write and the usearch index update. Solution: track pending vectors in redb.

```rust
// In Storage::put_vector
pub fn put_vector(&self, db_name: &str, uid: u64, vector: Vec<f64>) -> anyhow::Result<()> {
    // Write to pending queue first (guaranteed by redb transaction)
    let pending_key = format!("vector_pending:{}:{}", db_name, uid);
    let pending_value = serialize_vector(&vector);
    self.sys_table.insert(pending_key.as_bytes(), &pending_value)?;
    
    // Then enqueue for async indexing
    self.vector_tx.send((db_name.to_string(), uid, vector))?;
    Ok(())
}
```

**Phase 2: Vector Index Reconciliation on Startup (1 week)**

On startup, check for vectors that were written to redb but never made it to usearch:

```rust
pub fn reconcile_vectors(&self) -> anyhow::Result<usize> {
    let mut reconciled = 0;
    
    // Scan pending queue for unindexed vectors
    for (key, value) in self.sys_table.prefix(b"vector_pending:") {
        if let Some((db_name, uid)) = parse_pending_key(&key) {
            // Check if vector exists in usearch
            if !self.vector_engine.contains(&db_name, uid) {
                // Re-index
                let vector = deserialize_vector(&value);
                self.vector_engine.add_vector(&db_name, uid, &vector)?;
                reconciled += 1;
            }
            // Remove from pending queue after successful indexing
            self.sys_table.remove(&key)?;
        }
    }
    
    Ok(reconciled)
}
```

**Phase 3: Periodic Cleanup (1 week)**

1. Batch vector writes
2. Flush usearch index  
3. Clear pending queue entries for successfully indexed vectors
4. On startup: reconcile any remaining pending vectors

Alternative approach: Store vectors directly in redb alongside other data, with usearch as a query-only accelerator that rebuilds from redb on startup.

### Verification Tests

**Test 12.1: Pending Queue Entry Created**
```rust
#[tokio::test]
async fn test_vector_pending_entry_created() {
    let storage = create_test_storage().await;
    let vector = vec![0.1f64; 384];
    
    storage.put_vector("default", 1, vector.clone()).unwrap();
    
    // Check pending entry exists
    let pending_key = b"vector_pending:default:1";
    let pending_value = storage.sys_table.get(pending_key).unwrap();
    assert!(pending_value.is_some());
}
```

**Test 12.2: Reconciliation on Startup**
```rust
#[tokio::test]
async fn test_reconciliation_on_startup() {
    let storage = create_test_storage().await;
    let vector = vec![0.1f64; 384];
    
    // Write vector to pending queue but don't index
    storage.put_vector_pending_only("default", 1, vector.clone()).unwrap();
    
    // Verify not in usearch
    assert!(!storage.vector_engine.contains("default", 1));
    
    // Run reconciliation
    let reconciled = storage.reconcile_vectors().unwrap();
    
    // Should have indexed the missing vector
    assert_eq!(reconciled, 1);
    assert!(storage.vector_engine.contains("default", 1));
}
```

**Test 12.3: Pending Queue Cleared After Successful Index**
```rust
#[tokio::test]
async fn test_pending_cleared_after_index() {
    let storage = create_test_storage().await;
    let vector = vec![0.1f64; 384];
    
    storage.put_vector("default", 1, vector.clone()).unwrap();
    storage.reconcile_vectors().unwrap();
    
    // Pending entry should be removed
    let pending_key = b"vector_pending:default:1";
    let pending_value = storage.sys_table.get(pending_key).unwrap();
    assert!(pending_value.is_none());
}
```

**Test 12.4: Vector Search Works After Recovery**
```rust
#[tokio::test]
async fn test_vector_search_after_recovery() {
    let storage = create_test_storage().await;
    let vector = vec![1.0f64, 0.0, 0.0];
    
    // Index a vector
    storage.put_vector("default", 1, vector.clone()).unwrap();
    storage.vector_engine.save_all().unwrap();
    
    // Simulate crash: clear usearch but keep pending queue
    storage.vector_engine.clear().unwrap();
    
    // Recover
    storage.reconcile_vectors().unwrap();
    
    // Search should still work
    let results = storage.search_vectors("default", &vector, 10).unwrap();
    assert_eq!(results[0].0, 1);
}
```

---

## Issue 13: No BM25 Stat Exposure / Index Health [LOW]

### Current State
**File**: `src/storage/codec.rs` (lines 200-228)

```rust
// --- BM25 Stats ---
// Prefix: 0x05
// Key: [0x05][Pred][0x00][StatType]
// StatType: 0=DocCount, 1=TotalLen, 2=DF(Term)

pub fn encode_stat_key(predicate: &str, stat_type: u8, term: Option<&str>) -> Vec<u8> { ... }

// --- Doc Meta (Length) ---
// Prefix: 0x06
// Key: [0x06][Pred][0x00][UID]
pub fn encode_doc_meta_key(predicate: &str, uid: u64) -> Vec<u8> { ... }
```

These codec keys (0x05, 0x06) are defined but appear unused now that Tantivy handles search.

### Gap Analysis
- Old manual BM25 stat keys are dead code
- No way to query index statistics (document count, term frequency distribution, index size)
- Useful for debugging relevance issues in production

### Implementation Plan (1 week)

**Step 1: Clean Up Dead Code**

Remove or deprecate encode_stat_key and encode_doc_meta_key from codec.rs.

**Step 2: Expose Tantivy Index Stats**

Add API to SearchEngine:

```rust
pub struct IndexStats {
    pub doc_count: u64,
    pub term_count: u64,
    pub index_size_bytes: u64,
    pub segment_count: usize,
}

pub fn get_stats(&self, db_name: &str) -> anyhow::Result<IndexStats> {
    let idx = self.get_or_create(db_name)?;
    let searcher = idx.index.reader()?.searcher();
    let doc_count = searcher.num_docs();
    let segments = searcher.segment_readers();
    // ... gather stats
}
```

**Step 3: Admin API**

Add endpoint:

```graphql
query {
    indexStats(type: "Document") {
        docCount
        termCount
        indexSizeBytes
        segmentCount
    }
}
```

### Verification Tests

**Test 13.1: Index Stats Returned**
```rust
#[tokio::test]
async fn test_index_stats() {
    // Create some documents
    for i in 0..100 {
        create_node("Document", json!({"title": format!("doc {}", i)})).await;
    }
    
    let stats = search_engine.get_stats("default").unwrap();
    
    assert_eq!(stats.doc_count, 100);
    assert!(stats.index_size_bytes > 0);
    assert!(stats.segment_count >= 1);
}
```

**Test 13.2: Stats Updated After Delete**
```rust
#[tokio::test]
async fn test_stats_after_delete() {
    create_node("Document", json!({"title": "test"})).await;
    let stats_before = search_engine.get_stats("default").unwrap();
    
    // Delete the document
    search_engine.remove_document("default", uid, "title").unwrap();
    search_engine.flush_deletes("default").unwrap();
    
    let stats_after = search_engine.get_stats("default").unwrap();
    assert!(stats_after.doc_count < stats_before.doc_count);
}
```

**Test 13.3: Empty Index Stats**
```rust
#[tokio::test]
async fn test_empty_index_stats() {
    let stats = search_engine.get_stats("empty_db").unwrap();
    
    assert_eq!(stats.doc_count, 0);
    assert_eq!(stats.index_size_bytes, 0);
    assert_eq!(stats.segment_count, 0);
}
```

**Test 13.4: Dead Codec Keys Removed**
```rust
#[test]
fn test_dead_codec_keys_removed() {
    // These functions should no longer exist
    // Uncommenting should cause compile error:
    // let _ = Codec::encode_stat_key("field", 0, None);
    // let _ = Codec::encode_doc_meta_key("field", 1);
}
```

---

## Issue 14: Backup and Restore [CRITICAL - was 0b]

### Current State
No backup or restore mechanisms exist. A disk failure or corrupted store means total data loss.

### redb Backup Strategy

**redb does not have a built-in backup API.** Backup is implemented by copying the `.redb` file directly:

1. **File-copy backup**: Copy `default.redb` to backup location
   - redb is crash-safe and uses MVCC, so reads can continue during copy
   - For consistent snapshot, pause writes during copy (or accept slight inconsistency)
   - Optionally run `db.compact()` before copy to reduce file size

2. **Tantivy indexes**: Copy the Tantivy directory separately

3. **usearch indexes**: Copy the usearch `.usearch` file separately

4. **Restore**: Stop VardaDB, replace files from backup, restart

**Note**: Savepoints exist in redb but are for in-process rollback only, not file-level backup.

### Implementation

**Backup**
```rust
pub fn create_backup(&self, backup_path: &Path) -> anyhow::Result<BackupId>;
```
- Pause writes briefly (acquire write lock)
- Copy redb file (`.redb`)
- Copy Tantivy index directory
- Copy usearch index file (`.usearch`)
- Resume writes
- Return backup ID

**Restore**
```rust
pub fn restore_from_backup(backup_path: &Path) -> anyhow::Result<()>;
```
- Stop all writes
- Validate backup integrity
- Replace current files with backup files
- Restart (reconciliation will handle any vector index drift)

**Export/Import** (optional, for migration)
```rust
pub fn export(&self, format: ExportFormat) -> anyhow::Result<Vec<u8>>;
pub fn import(&self, data: &[u8], format: ExportFormat) -> anyhow::Result<()>;
```
- Portable format (JSON, Parquet)
- Logical export for cross-version compatibility

### Implementation Tasks
- [ ] Implement backup endpoint: pause writes, copy `.redb`, Tantivy dir, usearch file
- [ ] Implement restore endpoint: stop writes, validate backup, replace files
- [ ] Add `compact()` before backup option (reduces file size)
- [ ] Add backup integrity verification (checksum files)
- [ ] Export/Import to JSON (optional, for migration)
- [ ] Automated backup scheduling (every 15 min recommended)

### Verification Tests

**Test 14.1: Backup Creates Consistent Snapshot**
```rust
#[tokio::test]
async fn test_backup_creates_consistent_snapshot() {
    // Create documents
    for i in 0..100 {
        create_node("Doc", json!({"id": i})).await;
    }
    
    let backup_id = admin("POST /admin/backup").await;
    
    // Verify backup files exist and are consistent
    let backup_path = format!("backups/{}/", backup_id);
    assert!(Path::new(&format!("{}default.redb", backup_path)).exists());
    assert!(Path::new(&format!("{}tantivy/", backup_path)).exists());
    assert!(Path::new(&format!("{}vectors/", backup_path)).exists());
}
```

**Test 14.2: Restore Recovers Data**
```rust
#[tokio::test]
async fn test_restore_recover_data() {
    // Create and backup
    let uid = create_node("Doc", json!({"content": "important"})).await.uid;
    let backup_id = admin("POST /admin/backup").await;
    
    // Delete node
    delete_node(uid).await;
    assert!(!node_exists(uid));
    
    // Restore
    admin("POST /admin/restore", json!({"backup_id": backup_id})).await;
    
    // Node should be back
    assert!(node_exists(uid));
    assert_eq!(get_node(uid)["content"], "important");
}
```

**Test 14.3: Export Produces Valid Portable Format**
```rust
#[tokio::test]
async fn test_export_import() {
    create_node("Doc", json!({"content": "test"})).await;
    
    let export_data = admin("POST /admin/export").await;
    
    // Clear database
    clear_all_data().await;
    
    // Import
    admin("POST /admin/import", export_data).await;
    
    // Data should be restored
    let results = query("{ searchDocs { content } }").await;
    assert_eq!(results[0]["content"], "test");
}
```

**Test 14.4: Backup While Running Does Not Block**
```rust
#[tokio::test]
async fn test_hot_backup_non_blocking() {
    // Start background writes
    let write_handle = tokio::spawn(async {
        for i in 0..1000 {
            create_node("Doc", json!({"id": i})).await;
        }
    });
    
    // Backup should not block
    let start = Instant::now();
    let backup_id = admin("POST /admin/backup").await;
    let backup_time = start.elapsed();
    
    // Backup should complete quickly even while writing
    assert!(backup_time < Duration::from_secs(5));
    
    // Wait for writes to complete
    write_handle.await.unwrap();
    
    // Verify backup is consistent (no corruption)
    admin("POST /admin/restore", json!({"backup_id": backup_id})).await;
    let count = count_nodes("Doc").await;
    assert!(count > 0); // Should have some data
}
```

---

## Summary and Prioritization

| Issue | Severity | Effort | Priority |
|-------|----------|--------|----------|
| 0. Production Resilience (Meta) | CRITICAL | 2-3 weeks | P0 |
| 0a. Tantivy Durability (was #8) | CRITICAL | 1 week | P0 |
| 14. Backup/Restore | CRITICAL | 3 days | P0 |
| 1. Embedding Generation | CRITICAL | 5-6 weeks | P0 |
| 2. resolve_list Brute-Force | CRITICAL | 1-2 weeks | P0 |
| 12. Vector Persistence | CRITICAL | 2-3 weeks | P0 |
| 4. Phrase/Proximity | HIGH | 1-2 weeks | P1 |
| 5. Fuzzy Matching | HIGH | 1 week | P1 |
| 6. Geo Spatial Index | HIGH | 3-4 weeks | P1 |
| 3. Field Boosting | HIGH | 1-2 weeks | P2 |
| 9. Highlighting | MEDIUM | 1 week | P2 |
| 10. Faceted Search | MEDIUM | 2-3 weeks | P2 |
| 11. RRF Weighting | MEDIUM | 1 week | P3 |
| 7. Trigram Index | MEDIUM | 1 week | P3 |
| 13. BM25 Stats | LOW | 1 week | P4 |

**Recommended Roadmap**:

**Phase 0 (Week 1)**: Production Resilience Foundation

**Days 1-2: Durability Audit**
- Issue #0: Redb durability audit (confirm all writes use `Durability::Immediate`)
- Issue #0a: Tantivy durability (commit-on-write, no batching by default)

**Days 3-5: Backup/Restore**
- Issue #14: Hot Backup and Restore (filesystem-level snapshots)
- Add comprehensive crash recovery tests

Recovery is to last backup point only. Use frequent backups (e.g., every 15 minutes) for near-real-time recovery.

**Phase 1 (Weeks 3-6)**: Critical Functional Issues
- Issue #2: resolve_list HNSW fix
- Issue #12: Vector persistence reconciliation
- Issue #1: Embedding generation Phase 1-2 (backend + schema)

**Phase 2 (Weeks 7-10)**: Search Quality
- Issue #4: Phrase queries
- Issue #5: Fuzzy matching
- Issue #3: Field boosting
- Issue #1: Embedding generation Phase 3 (write-time pipeline with DLQ)

**Phase 3 (Weeks 11-14)**: Scale and Features
- Issue #6: Geo spatial index
- Issue #9: Highlighting
- Issue #10: Faceted search

**Phase 4 (Ongoing)**: Polish
- Issue #11: RRF weighting
- Issue #7: Trigram index
- Issue #13: BM25 stats

---

**Document Version**: 1.1  
**Last Updated**: April 6, 2026  
**Author**: AI Assistant (updated with production resilience focus)

