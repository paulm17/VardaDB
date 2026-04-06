# Issue 03: Query-Time Field Boosting

**File**: `src/storage/tantivy_search.rs`
**Effort**: 1-2 weeks
**Friction**: LOW-MEDIUM

## Change
Extend search_bm25 to accept multiple fields with optional boost weights.

## Code Change

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
    let clauses: Vec<(Occur, Box<dyn Query>)> = fields
        .iter()
        .map(|fb| {
            let field = self.get_field(&fb.field);
            let terms = tokenize(query_text, strategy);
            let field_query = if require_all {
                BooleanQuery::new_multiterms_query(terms, field, Occur::Must)
            } else {
                BooleanQuery::new_multiterms_query(terms, field, Occur::Should)
            };
            (Occur::Should, Box::new(BoostQuery::new(Box::new(field_query), fb.boost)))
        })
        .collect();
    
    let multi_field_query = BooleanQuery::new(clauses);
    // Execute and return results
}
```

## GraphQL Extension

```graphql
query {
    searchDocuments(filter: {
        anyoftext: "rust programming"
        fields: [
            {field: "title", boost: 3.0}
            {field: "description", boost: 1.0}
        ]
    })
}
```

## Test

```rust
#[tokio::test]
async fn test_field_boost_affects_ranking() {
    create_node("Article", json!({
        "title": "Other topic",
        "description": "Contains the search term here"
    })).await;
    
    create_node("Article", json!({
        "title": "search term",
        "description": "Other content"
    })).await;
    
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
    
    // Title match should rank higher
    assert!(results[0].score > results[1].score);
}
```
