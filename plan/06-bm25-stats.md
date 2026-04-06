# Issue 06: BM25 Index Statistics

**File**: `src/storage/tantivy_search.rs`
**Effort**: 1 week
**Friction**: LOW

## Change
Expose Tantivy index statistics for observability.

## Code Change

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
    
    // Calculate total index size
    let index_size = std::fs::metadata(&self.index_path(db_name))?.len();
    
    // Estimate term count (sum over segments)
    let term_count: u64 = segments.iter()
        .map(|seg| seg.num_docs() as u64 * 100) // Rough estimate
        .sum();
    
    Ok(IndexStats {
        doc_count,
        term_count,
        index_size_bytes: index_size,
        segment_count: segments.len(),
    })
}
```

## GraphQL Extension

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

## Test

```rust
#[tokio::test]
async fn test_index_stats() {
    for i in 0..100 {
        create_node("Document", json!({"title": format!("doc {}", i)})).await;
    }
    
    let stats = search_engine.get_stats("default").unwrap();
    
    assert_eq!(stats.doc_count, 100);
    assert!(stats.index_size_bytes > 0);
    assert!(stats.segment_count >= 1);
}
```
