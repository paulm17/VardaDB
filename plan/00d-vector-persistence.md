# Issue 00d: Vector Persistence (Crash Safety)

**File**: `src/storage/backend.rs`, `src/storage/vector_engine.rs`
**Effort**: 2 weeks
**Friction**: HIGH

## Problem

usearch indexes asynchronously. If process crashes between redb write and usearch index, vector is lost.

## Solution

Track pending vectors in redb, reconcile on startup.

## Code Change

```rust
// In Storage::put_vector
pub fn put_vector(&self, db_name: &str, uid: u64, vector: Vec<f64>) -> anyhow::Result<()> {
    // 1. Write to pending queue (redb transaction - survives crash)
    let pending_key = format!("vector_pending:{}:{}", db_name, uid);
    let pending_value = serialize_vector(&vector);
    self.sys_table.insert(pending_key.as_bytes(), &pending_value)?;
    
    // 2. Enqueue for async indexing (usearch)
    self.vector_tx.send((db_name.to_string(), uid, vector))?;
    
    Ok(())
}
```

```rust
// Background worker - removes from queue after successful index
fn vector_indexing_worker(&self) {
    while let Ok((db_name, uid, vector)) = self.vector_rx.recv() {
        if let Err(e) = self.vector_engine.add_vector(&db_name, uid, &vector) {
            log::error!("Failed to index vector: {}", e);
            continue; // Leave in queue, will retry on startup
        }
        
        // Success - remove from pending queue
        let pending_key = format!("vector_pending:{}:{}", db_name, uid);
        if let Err(e) = self.sys_table.remove(pending_key.as_bytes()) {
            log::error!("Failed to remove from queue: {}", e);
        }
    }
}
```

```rust
// Startup reconciliation
pub fn reconcile_vectors(&self) -> anyhow::Result<usize> {
    let mut reconciled = 0;
    
    for (key, value) in self.sys_table.prefix(b"vector_pending:") {
        let (db_name, uid) = parse_pending_key(&key)?;
        let vector = deserialize_vector(&value);
        
        // Check if already indexed (idempotent)
        if !self.vector_engine.contains(&db_name, uid) {
            self.vector_engine.add_vector(&db_name, uid, &vector)?;
            reconciled += 1;
        }
        
        // Remove from queue
        self.sys_table.remove(&key)?;
    }
    
    Ok(reconciled)
}
```

## Startup Order

1. Open redb database
2. Call `reconcile_vectors()`
3. Start vector indexing worker
4. Start accepting requests

## Test

```rust
#[tokio::test]
async fn test_vector_survives_crash() {
    let storage = create_test_storage();
    let vector = vec![1.0f64, 0.0, 0.0];
    
    // Index vector
    storage.put_vector("default", 1, vector.clone()).unwrap();
    
    // Simulate crash: new storage instance, same files
    drop(storage);
    let new_storage = create_test_storage_same_path();
    
    // Reconcile (would happen on startup)
    let reconciled = new_storage.reconcile_vectors().unwrap();
    assert!(reconciled >= 1); // Vector was pending
    
    // Search works
    let results = new_storage.search_vectors("default", &vector, 10).unwrap();
    assert_eq!(results[0].0, 1);
}
```
