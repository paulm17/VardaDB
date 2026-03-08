use async_graphql::Request;
use management::DatabaseManager;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::RwLock;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::realtime::bus::EventBus;
use vardadb::server::management::ManagementState;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_full_server_flow_repro() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().to_path_buf();

    let db_name = "repro_db";
    let schema_sdl = "
        type User { 
            id: ID! 
            name: String 
            email: String 
        } 
    ";

    // 1. START SERVER (Simulation) & 2. CREATE DATABASE & 3. APPLY SCHEMA
    {
        println!("--- STEP 1-3: Start, Create DB, Apply Schema ---");
        let storage = Arc::new(Storage::new(&db_path, None).unwrap());
        let schemas = Arc::new(dashmap::DashMap::<String, Arc<RwLock<Arc<Schema>>>>::new());
        let event_bus = EventBus::new();

        let state = ManagementState {
            storage: storage.clone(),
            schemas: schemas.clone(),
            event_bus: event_bus.clone(),
            storage_path: db_path.clone(),
        };

        // Create DB
        state.create_db(db_name).await.unwrap();
        // Apply Schema
        state.apply_schema(db_name, schema_sdl).await.unwrap();

        // 4. ADD DATA (via GraphQL)
        println!("--- STEP 4: Add Data ---");

        // We need to simulate the GraphQL Handler logic here manually
        // because we don't want to spin up the actual HTTP server in a unit test if we can avoid it.
        // We can get the schema from the map (populated by apply_schema)

        let schema_wrapper = state
            .schemas
            .get(db_name)
            .expect("Schema should be loaded")
            .clone();
        let schema = schema_wrapper.read().await.clone();

        let mutation = "
            mutation {
                createUser(input: {name: \"Alice\", email: \"alice@example.com\"}) {
                    id
                    name
                }
            }
        ";

        let resp = schema.execute(Request::new(mutation)).await;
        assert!(resp.errors.is_empty(), "Mutation failed: {:?}", resp.errors);

        let data = resp.data.into_json().unwrap();
        println!("Mutation Result: {}", data);

        // Ensure Flush
        storage.flush().unwrap();
    } // STOP SERVER (Drop)

    // 5. RESTART SERVER & 6. QUERY DATA
    {
        println!("--- STEP 5-6: Restart & Query ---");
        let storage = Arc::new(Storage::new(&db_path, None).unwrap());
        let _schemas = Arc::new(dashmap::DashMap::<String, Arc<RwLock<Arc<Schema>>>>::new());
        let _event_bus = EventBus::new();

        // Simulate Server Startup Wiring (from lib.rs)
        // Note: ManagementState doesn't auto-load schemas on init in strict sense,
        // but the `graphql_handler` lazy loads them.

        // We need to mimic `graphql_handler`'s lazy load logic to get the schema

        // Check if DB exists
        assert!(
            storage.get_database(db_name).is_some(),
            "Database should persist in storage"
        );

        // Lazy Load Logic from lib.rs (simplified)
        // 1. Resolve Resolver
        let resolver = SqliteResolver::new(storage.clone(), db_name);

        // 2. Load Schema File (using fixed path logic)
        let schema_file_path = db_path.join(format!("{}_schema.graphql", db_name));
        assert!(schema_file_path.exists(), "Schema file must exist");

        let loaded_sdl = tokio::fs::read_to_string(schema_file_path).await.unwrap();
        assert_eq!(loaded_sdl, schema_sdl, "Schema SDL must match");

        // 3. Build Schema
        let new_schema = Schema::load_with_resolver(&loaded_sdl, resolver).unwrap();

        // 4. Query
        let query = "{ queryUser { name email } }";
        let resp = new_schema.execute(Request::new(query)).await;

        assert!(resp.errors.is_empty(), "Query failed: {:?}", resp.errors);

        let data = resp.data.into_json().unwrap();
        println!("Query Result: {}", data);

        let users = data.get("queryUser").unwrap().as_array().unwrap();
        assert_eq!(users.len(), 1, "Should have 1 user");
        assert_eq!(users[0].get("name").unwrap().as_str().unwrap(), "Alice");
    }
}
