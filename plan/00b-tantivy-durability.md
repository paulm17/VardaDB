# Issue 00b: Tantivy Durability

**File**: `src/storage/tantivy_search.rs`
**Effort**: 1 day
**Friction**: LOW

## Change
Change Tantivy from batch commits to commit-on-every-write for safety.

## Current Problem

```rust
// Current code (Issue 8 batches deletes)
// Batching is WRONG for production - loses data on crash
```

## Fix

```rust
pub fn remove_document(&self, db_name: &str, uid: u64, field: &str) -> anyhow::Result<()> {
    let idx = self.get_or_create(db_name)?;
    let cid = composite_doc_id(uid, field);
    
    let mut writer = idx.writer.lock();
    writer.delete_term(Term::from_field_u64(idx.doc_id_field, cid));
    writer.commit()?; // Commit immediately - don't batch
    
    Ok(())
}

pub fn index_document(&self, db_name: &str, uid: u64, field: &str, text: &str) -> anyhow::Result<()> {
    let idx = self.get_or_create(db_name)?;
    
    let mut writer = idx.writer.lock();
    // ... add document ...
    writer.commit()?; // Commit immediately
    
    Ok(())
}
```

## Optional: Batch Mode for Bulk Ingest

Add explicit opt-in for batching (default is safe):

```rust
pub struct SearchConfig {
    pub batch_commits: bool, // default: false
    pub batch_size: usize,   // default: 100
}

pub fn remove_document(&self, ...) -> anyhow::Result<()> {
    if self.config.batch_commits {
        // Queue delete
        self.pending_deletes.push((uid, field));
        if self.pending_deletes.len() >= self.config.batch_size {
            self.flush_deletes()?;
        }
    } else {
        // Immediate commit (safe)
        writer.commit()?;
    }
}
```

## Test

```rust
#[tokio::test]
async fn test_tantivy_commits_immediately() {
    let search = create_test_search_engine();
    
    // Index document
    search.index_document("default", 1, "content", "test").unwrap();
    
    // Simulate crash (new searcher)
    let new_search = create_test_search_engine_same_path();
    
    // Document should be found (committed before crash)
    let results = new_search.search_bm25("default", "test", "content", "term", 10, false);
    assert_eq!(results.len(), 1);
}
```
