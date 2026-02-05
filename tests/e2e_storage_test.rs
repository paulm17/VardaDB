use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::storage::codec::Codec;
use vardadb::bridge::fjall_resolver::FjallResolver;
use std::sync::Arc;
use tempfile::tempdir;
use serde_json::Value;

#[tokio::test]
async fn test_e2e_storage_query() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());
    
    // Seed Data: User 1 -> name: "Alice"
    let uid = 1u64;
    // Note: Dgraph/Codec key is [Prefix][UID][Pred]
    // We must manually insert the data to simulate "existing data"
    // Ideally we would use Mutations, but they aren't implemented yet.
    
    // Key encoding for "name" field of UID 1
    // Assuming prefix "0x01" for Data as per Codec
    let data_key = Codec::encode_data_key(uid, "name");
    let val_bytes = serde_json::to_vec(&Value::String("Alice".to_string())).unwrap();
    storage.insert(&data_key, &val_bytes).unwrap();
    
    // Manual Type Index (Required for node_exists check)
    let type_key = Codec::encode_type_index_key("User", uid);
    storage.insert(&type_key, &[]).unwrap();

    // Setup Schema (Dynamic!)
    // We define 'User', and the engine should auto-generate 'getUser'
    let sdl = "
        type User {
            name: String
        }
    ";
    
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    
    // Create Resolver
    let resolver = Box::new(FjallResolver::new(storage.clone()));
    
    // 3. Execute Query
    // Query: { getUser(uid: "1") { name } }
    let query = "{ getUser(uid: \"1\") { name } }"; 
         
    let response_json = schema.execute_with_resolver(query, resolver).await;
    
    // 4. Verify Result
    let response: Value = serde_json::from_str(&response_json).expect("Response should be valid JSON");
    let data = response.get("data").expect("Response should have data field");
    // data should be { "getUser": { "name": "Alice" } }
    
    let user = data.get("getUser").expect("Data should have getUser field");
    let name = user.get("name").expect("User should have name field");
    
    assert_eq!(name.as_str(), Some("Alice"), "Query should return 'Alice' from Fjall");
}
