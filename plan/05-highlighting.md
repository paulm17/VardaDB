# Issue 05: Search Result Highlighting

**File**: `src/storage/tantivy_search.rs`
**Effort**: 1 week
**Friction**: LOW

## Change
Add snippet generation using Tantivy's `SnippetGenerator`.

## Code Change

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
    field: &str,
    strategy: &str,
    k: usize,
    require_all: bool,
) -> Vec<SearchResult> {
    let index = self.get_or_create(db_name)?;
    let searcher = index.index.reader()?.searcher();
    let query = self.build_query(query_text, field, strategy, require_all)?;
    
    let snippet_generator = SnippetGenerator::new(&searcher, &query, index.schema.get_field(field)?)?;
    
    let top_docs = searcher.search(&query, &TopDocs::with_limit(k))?;
    
    top_docs.iter().map(|(score, doc_addr)| {
        let doc = searcher.doc(*doc_addr)?;
        let snippet = snippet_generator.snippet_from_doc(&doc);
        
        SearchResult {
            uid: extract_uid(&doc),
            score: *score,
            snippet: Some(snippet.to_html()),
            highlighted_terms: snippet.highlighted().iter().map(|s| s.to_string()).collect(),
        }
    }).collect()
}
```

## Test

```rust
#[tokio::test]
async fn test_snippet_generated() {
    create_node("Document", json!({
        "content": "The quick brown fox jumps over the lazy dog"
    })).await;
    
    let results = search_bm25_with_snippets(
        "default", "quick fox", "content", "fulltext", 10, false
    ).await;
    
    assert!(results[0].snippet.is_some());
    assert!(results[0].snippet.unwrap().contains("quick"));
}
```
