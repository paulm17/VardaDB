use std::sync::Arc;
use tempfile::tempdir;
use vardadb::caching::CacheManager;
use vardadb::storage::backend::Storage;

#[test]
fn test_cache_view_creation() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let manager = CacheManager::new(storage.clone());

    // Create View "active_users"
    let view = manager.create_view("active_users", "SELECT * FROM users WHERE active = true");

    // Simulate Dataflow Update
    view.process_update("user:1", "Alice").unwrap();

    // Verify Consistency
    let val = view.get("user:1").unwrap();
    assert_eq!(val, Some("Alice".to_string()));

    // Verify key namespacing in storage
    // Key should be "view:active_users:user:1"
    let raw_val = storage.get("default", b"view:active_users:user:1").unwrap();
    assert!(raw_val.is_some());
}
