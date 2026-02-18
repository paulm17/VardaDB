use vardadb::storage::backend::Storage;
use vardadb::server::management::ManagementState;
use management::DatabaseManager;
use vardadb::realtime::bus::EventBus;
use std::sync::Arc;

use tempfile::tempdir;

#[tokio::test(flavor = "multi_thread")]
async fn test_schema_persistence() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().to_path_buf();
    let db_name = "test_schema_db";
    let schema_content = "type User { id: ID! name: String }";

    // 1. Initialize Management State
    {
        let storage = Arc::new(Storage::new(&db_path, None).unwrap());
        let schemas = Arc::new(dashmap::DashMap::new());
        let event_bus = EventBus::new();
        
        let state = ManagementState {
            storage: storage.clone(),
            schemas: schemas.clone(),
            event_bus: event_bus,
            storage_path: db_path.clone(),
        };

        // Create DB
        state.create_db(db_name).await.unwrap();
        
        // Apply Schema
        state.apply_schema(db_name, schema_content).await.unwrap();
        
        // Check if schema file exists on disk
        let schema_file = db_path.join(format!("{}_schema.graphql", db_name));
        assert!(schema_file.exists(), "Schema file should exist at {:?}", schema_file);
        
        let saved_content = tokio::fs::read_to_string(&schema_file).await.unwrap();
        assert_eq!(saved_content, schema_content, "Schema content should match");
        
        storage.flush().unwrap();
    }

    // 2. Restart and Verify Load (Simulation)
    // In actual server run, `lazy_load_schema` in `src/lib.rs` does this.
    // We can simulate the logic here to ensure the PATH is correct.
    {
        let _storage = Arc::new(Storage::new(&db_path, None).unwrap());
        
        // Simulate what `graphql_handler` does
        // It uses `state.storage_path.join(...)` which we fixed.
        
        let schema_path = db_path.join(format!("{}_schema.graphql", db_name));
        assert!(schema_path.exists(), "Schema file should still exist");
        
        // Manually try to load it using the same logic as the fix
        let sdl = std::fs::read_to_string(&schema_path).expect("Failed to read schema");
        assert_eq!(sdl, schema_content);
    }
}
