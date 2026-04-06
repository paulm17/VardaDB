# Issue 01: Fuzzy / Edit-Distance Matching

**File**: `src/storage/tantivy_search.rs`
**Effort**: 1 week
**Friction**: LOW

## Change
Add fuzzy term matching using Tantivy's `FuzzyTermQuery`.

## Code Change

```rust
use tantivy::query::FuzzyTermQuery;

pub fn search_bm25(
    &self,
    db_name: &str,
    query_text: &str,
    field: &str,
    strategy: &str,
    k: usize,
    require_all: bool,
    fuzzy_distance: Option<u8>, // NEW
) -> Vec<(u64, f64)> {
    // ... existing code ...
    
    let term_query: Box<dyn Query> = if let Some(distance) = fuzzy_distance {
        Box::new(FuzzyTermQuery::new(term, distance, true))
    } else {
        Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs))
    };
    
    // ... rest of existing code ...
}
```

## GraphQL Extension

```graphql
query {
    searchDocuments(filter: {fuzzy: {terms: "database", distance: 1}})
}
```

## Test

```rust
#[tokio::test]
async fn test_fuzzy_typo_tolerance() {
    create_node("Product", json!({"name": "database"})).await;
    
    // "databse" (typo) should match "database" with distance=1
    let results = search_bm25(
        "default", "databse", "name", "term", 10, false, Some(1)
    ).await;
    
    assert_eq!(results.len(), 1);
}
```
