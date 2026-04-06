# Issue 08: Trigram Index

**File**: `src/storage/codec.rs`, `src/bridge/redb_resolver.rs`
**Effort**: 1 week
**Friction**: MEDIUM

## Change
Add trigram index for efficient substring/LIKE queries.

## Code Change

```rust
// In src/storage/codec.rs

/// Prefix: 0x0B
/// Key: [0x0B][Predicate][0x00][Trigram][UID]
pub fn encode_trigram_index_key(predicate: &str, trigram: &str, uid: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(0x0B);
    buf.extend_from_slice(predicate.as_bytes());
    buf.push(0x00);
    buf.extend_from_slice(trigram.as_bytes());
    buf.write_u64::<BigEndian>(uid).unwrap();
    buf
}
```

```rust
// In write path (create_node_internal/update_node_internal)

if tokenizers.contains(&"trigram".to_string()) {
    let trigrams = Tokenizer::tokenize(text, "trigram");
    for trigram in trigrams {
        let key = Codec::encode_trigram_index_key(field, &trigram, uid);
        main_table.insert(&key, &[])?;
    }
}
```

```rust
// In search path

pub fn search_contains(&self, db_name: &str, field: &str, substring: &str) -> Vec<u64> {
    let trigrams = Tokenizer::tokenize(substring, "trigram");
    
    // Find UIDs that have all trigrams (intersection)
    let mut candidates: Option<HashSet<u64>> = None;
    
    for trigram in trigrams {
        let prefix = Codec::encode_trigram_prefix(field, &trigram);
        let matches: HashSet<u64> = self.prefix_scan(&prefix).collect();
        
        candidates = match candidates {
            None => Some(matches),
            Some(c) => Some(c.intersection(&matches).copied().collect()),
        };
    }
    
    candidates.unwrap_or_default().into_iter().collect()
}
```

## GraphQL Extension

```graphql
query {
    searchDocuments(filter: {description: {contains: "graph"}})
}
```

## Test

```rust
#[tokio::test]
async fn test_contains_query() {
    create_node("Document", json!({"content": "graph database"})).await;
    create_node("Document", json!({"content": "graph theory"})).await;
    create_node("Document", json!({"content": "sql database"})).await;
    
    let results = search_contains("Document", "content", "graph").await;
    
    assert_eq!(results.len(), 2);
}
```
