# Issue 11: resolve_list HNSW Fix

**File**: `src/bridge/redb_resolver.rs`
**Effort**: 1-2 weeks
**Friction**: MEDIUM-HIGH

## Change
Replace O(n) brute-force vector search in resolve_list with HNSW pre-filtering.

## Current Problem

```rust
// Current code (lines 2867-2896) - O(n) brute force
if let Some(ref vec) = near_vector {
    let mut uid_dists = Vec::new();
    for uid in &uids {  // Loops through ALL related UIDs
        if let Some(embedding) = self.resolve_cached(*uid, "embedding", cache) {
            // Computes cosine similarity for each
            let sim = cosine_similarity(&embedding, vec);
            uid_dists.push((*uid, sim));
        }
    }
    uid_dists.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    uids = uid_dists.into_iter().map(|(u, _)| u).collect();
}
```

## Fix

```rust
if let Some(ref vec) = near_vector {
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

## Test

```rust
#[tokio::test]
async fn test_resolve_list_uses_hnsw_not_brute_force() {
    let parent = create_node("Folder", json!({"name": "test"})).await;
    
    // Create 1000 related items
    for i in 0..1000 {
        let child = create_node("Document", json!({"content": format!("doc {}", i)})).await;
        create_edge(parent.uid, "contains", child.uid).await;
    }
    
    let start = Instant::now();
    let results = query(r#"
        query {
            getFolder(id: "UID") {
                contains(filter: {near_vector: {vector: [0.1, 0.2]}}) {
                    content
                }
            }
        }
    "#).await;
    
    // Should complete in < 100ms (not O(n) scan)
    assert!(start.elapsed() < Duration::from_millis(100));
}
```
