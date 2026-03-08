use vardadb::sync::{SyncConfig, SyncManager};

#[tokio::test(flavor = "multi_thread")]
async fn test_sync_disabled_by_default() {
    let config = SyncConfig::default();
    assert!(!config.enabled, "Sync should be disabled by default");

    let manager = SyncManager::new(config);
    // Should return immediately (no background task spawned)
    manager.start().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sync_enabled() {
    let config = SyncConfig {
        enabled: true,
        remote_url: Some("s3://bucket".to_string()),
    };

    let manager = SyncManager::new(config);
    // In a real test, we'd check if a task was spawned or side-effects occurred.
    // Here we just verify it doesn't panic.
    manager.start().await;
}
