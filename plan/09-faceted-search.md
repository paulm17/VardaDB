# Issue 09: Faceted Search

**File**: `src/storage/tantivy_search.rs`
**Effort**: 2-3 weeks
**Friction**: MEDIUM

## Change
Add faceted search using Tantivy's facet fields.

## Code Change

```rust
// In schema definition
let category_facet = sb.add_facet_field("category", FacetOptions::default());
```

```rust
// Indexing
pub fn index_facet(&self, db_name: &str, uid: u64, field: &str, value: &str) {
    let idx = self.get_or_create(db_name)?;
    let mut writer = idx.writer.lock();
    
    let facet = Facet::from(&format!("/{}/{}", field, value));
    writer.add_document(doc!(
        idx.doc_id_field => composite_doc_id(uid, field),
        idx.facet_field => facet,
    ))?;
}
```

```rust
// Facet counting
pub fn get_facet_counts(
    &self,
    db_name: &str,
    field: &str,
    prefix: Option<&str>,
) -> Vec<(String, u64)> {
    let idx = self.get_or_create(db_name)?;
    let searcher = idx.index.reader()?.searcher();
    
    let facet_collector = FacetCollector::for_field(idx.facet_field);
    let counts: FacetCounts = searcher.search(&AllQuery, &facet_collector)?;
    
    let facet_prefix = Facet::from(&format!("/{}/", field));
    counts.get(&facet_prefix)
        .map(|(facet, count)| (facet.to_string(), count))
        .collect()
}
```

## GraphQL Extension

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

## Test

```rust
#[tokio::test]
async fn test_facet_counts() {
    create_node("Product", json!({"name": "A", "category": "Electronics"})).await;
    create_node("Product", json!({"name": "B", "category": "Electronics"})).await;
    create_node("Product", json!({"name": "C", "category": "Books"})).await;
    
    let facets = get_facet_counts("default", "category", None).await;
    
    assert_eq!(facets["/category/Electronics"], 2);
    assert_eq!(facets["/category/Books"], 1);
}
```
