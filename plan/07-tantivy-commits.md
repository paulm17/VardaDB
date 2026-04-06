# Issue 07: Tantivy Commit Batching

**File**: `src/storage/tantivy_search.rs`
**Effort**: 1-2 weeks
**Friction**: MEDIUM

## Change
Batch Tantivy deletes to reduce write amplification.

## Code Change

```rust
pub struct SearchEngine {
    // ... existing fields ...
    pending_deletes: DashMap<String, Vec<(u64, String)>>, // db_name -> [(uid, field)]
}

pub fn remove_document(&self, db_name: &str, uid: u64, field: &str) -> anyhow::Result<()> {
    let batch_key = (db_name.to_string(), uid, field.to_string());
    self.indexed_this_batch.remove(&batch_key);
    
    // Queue delete instead of immediate commit
    let mut deletes = self.pending_deletes.entry(db_name.to_string()).or_default();
    deletes.push((uid, field.to_string()));
    
    // Auto-flush if batch size threshold reached
    if deletes.len() >= 100 {
        self.flush_deletes(db_name)?;
    }
    Ok(())
}

pub fn flush_deletes(&self, db_name: &str) -> anyhow::Result<()> {
    if let Some((_, deletes)) = self.pending_deletes.remove(db_name) {
        let idx = self.get_or_create(db_name)?;
        let mut writer = idx.writer.lock();
        
        for (uid, field) in deletes {
            let cid = composite_doc_id(uid, &field);
            writer.delete_term(Term::from_field_u64(idx.doc_id_field, cid));
        }
        
        writer.commit()?;
    }
    Ok(())
}
```

## Test

```rust
#[tokio::test]
async fn test_deletes_are_batched() {
    let search_engine = create_test_search_engine();
    
    // Queue 50 deletes
    for i in 0..50 {
        search_engine.remove_document("default", i, "content").unwrap();
    }
    
    // Should not have committed yet
    assert_eq!(search_engine.pending_deletes.len(), 50);
    
    // Queue 50 more (triggers auto-flush at 100)
    for i in 50..100 {
        search_engine.remove_document("default", i, "content").unwrap();
    }
    
    // Should have auto-flushed
    assert!(search_engine.pending_deletes.len() < 100);
}
```
