# Issue 04: RRF Weighting (Alpha Parameter)

**File**: `src/bridge/redb_resolver.rs`
**Effort**: 1 week
**Friction**: LOW

## Change
Add alpha parameter to hybrid search for tuning lexical vs semantic weight.

## Code Change

```rust
pub fn search_hybrid(
    &self,
    text_query: &str,
    field: &str,
    vector: &[f64],
    k: usize,
    require_all: bool,
    alpha: Option<f32>, // NEW: 0.0 = all BM25, 1.0 = all vector
) -> Vec<(u64, f64)> {
    let alpha = alpha.unwrap_or(0.5);
    let text_weight = 1.0 - alpha;
    let vector_weight = alpha;
    
    let text_results = self.search_text_bm25(text_query, field, "fulltext", k * 2, require_all);
    let vec_results = self.storage.vector_engine.search(&self.db_name, &vector_f32, k * 2);
    
    let mut scores: HashMap<u64, f64> = HashMap::new();
    
    for (rank, (uid, _)) in text_results.iter().enumerate() {
        *scores.entry(*uid).or_default() += text_weight / (60.0 + rank as f64 + 1.0);
    }
    
    for (rank, (uid, _)) in vec_results.iter().enumerate() {
        *scores.entry(*uid).or_default() += vector_weight / (60.0 + rank as f64 + 1.0);
    }
    
    // Sort and return
}
```

## GraphQL Extension

```graphql
query {
    searchDocuments(
        filter: {
            near_vector: {vector: [0.1, 0.2], alpha: 0.7}
            anyoftext: "rust programming"
        }
    )
}
```

## Test

```rust
#[tokio::test]
async fn test_alpha_zero_all_bm25() {
    create_test_documents().await;
    
    let text_query = "rust programming";
    let vector_query = vec![0.0; 384]; // Unrelated vector
    
    // With alpha=0.0, should only use BM25
    let results = search_hybrid(text_query, "content", &vector_query, 10, false, Some(0.0)).await;
    
    assert!(!results.is_empty()); // Should get BM25 results
}
```
