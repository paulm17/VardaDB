# Issue 02: Phrase Queries and Proximity Search

**File**: `src/storage/tantivy_search.rs`
**Effort**: 1-2 weeks
**Friction**: LOW

## Change
Add phrase query support using Tantivy's `PhraseQuery` when query is wrapped in quotes.

## Code Change

```rust
pub fn search_bm25(
    &self,
    db_name: &str,
    query_text: &str,
    field: &str,
    strategy: &str,
    k: usize,
    require_all: bool,
    phrase_slop: Option<u32>, // NEW
) -> Vec<(u64, f64)> {
    if query_text.starts_with('"') && query_text.ends_with('"') {
        let phrase_terms = parse_phrase_terms(query_text);
        let mut phrase_query = PhraseQuery::new(phrase_terms);
        if let Some(slop) = phrase_slop {
            phrase_query.set_slop(slop);
        }
        Box::new(phrase_query)
    } else {
        // Existing boolean query
    }
}
```

## GraphQL Extension

```graphql
query {
    searchDocuments(filter: {phrase: "graph database"})
}
```

## Test

```rust
#[tokio::test]
async fn test_exact_phrase_matching() {
    create_node("Document", json!({"content": "graph database is great"})).await;
    create_node("Document", json!({"content": "database graph relationships"})).await;
    
    let results = search_bm25(
        "default", "\"graph database\"", "content", "fulltext", 10, false, None
    ).await;
    
    assert_eq!(results.len(), 1);
}
```
