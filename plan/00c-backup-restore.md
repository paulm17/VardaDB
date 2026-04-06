# Issue 00c: Backup and Restore

**File**: Admin endpoints
**Effort**: 3 days
**Friction**: LOW

## Change
Add file-copy backup since redb has no built-in backup API.

## Implementation

```rust
pub fn create_backup(&self, backup_dir: &Path) -> anyhow::Result<String> {
    let backup_id = generate_backup_id();
    let backup_path = backup_dir.join(&backup_id);
    fs::create_dir_all(&backup_path)?;
    
    // Pause writes briefly
    let _write_lock = self.acquire_write_lock()?;
    
    // Copy files
    fs::copy(&self.redb_path, backup_path.join("default.redb"))?;
    copy_dir(&self.tantivy_path, backup_path.join("tantivy"))?;
    copy_dir(&self.vector_path, backup_path.join("vectors"))?;
    
    // Write metadata
    fs::write(
        backup_path.join("metadata.json"),
        json!({
            "timestamp": Utc::now().to_rfc3339(),
            "version": env!("CARGO_PKG_VERSION"),
        }).to_string()
    )?;
    
    Ok(backup_id)
}

pub fn restore_from_backup(&self, backup_path: &Path) -> anyhow::Result<()> {
    // Validate backup
    if !backup_path.join("default.redb").exists() {
        bail!("Invalid backup: missing redb file");
    }
    
    // Stop all writes
    let _write_lock = self.acquire_exclusive_write_lock()?;
    
    // Replace files
    fs::copy(backup_path.join("default.redb"), &self.redb_path)?;
    replace_dir(backup_path.join("tantivy"), &self.tantivy_path)?;
    replace_dir(backup_path.join("vectors"), &self.vector_path)?;
    
    // Reopen databases
    self.reopen()?;
    
    // Run reconciliation (vectors may be out of sync)
    self.reconcile_vectors()?;
    
    Ok(())
}
```

## Admin Endpoints

```
POST /admin/backup
Response: { "backup_id": "2024-01-15T10-30-00-abc123" }

POST /admin/restore
Body: { "backup_id": "2024-01-15T10-30-00-abc123" }

GET /admin/backups
Response: [{ "id": "...", "timestamp": "...", "size_bytes": 12345 }]
```

## Test

```rust
#[tokio::test]
async fn test_backup_and_restore() {
    // Create data
    let uid = create_node("Doc", json!({"content": "important"})).await.uid;
    
    // Backup
    let backup_id = admin("POST /admin/backup").await;
    
    // Delete
    delete_node(uid).await;
    assert!(!node_exists(uid));
    
    // Restore
    admin("POST /admin/restore", json!({"backup_id": backup_id})).await;
    
    // Verify
    assert!(node_exists(uid));
    assert_eq!(get_node(uid)["content"], "important");
}
```

## Notes

- Backup copies files while holding write lock (brief pause)
- No PITR - recovery is to backup point only
- Use frequent backups (15 min) for near-real-time recovery
