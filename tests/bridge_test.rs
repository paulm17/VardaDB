use vardadb::bridge::fjall_resolver::FjallResolver;
use vardadb::storage::backend::Storage;
use vardadb::storage::codec::Codec;
use vardadb::engine::resolver::Resolver;
use tempfile::tempdir;
use std::sync::Arc;
use async_graphql::Value;

#[test]
fn test_bridge_resolution() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());
    
    // 1. Setup Data manually (Stubbing the "Mutation" part)
    // User: Alice (UID 100)
    let uid = 100u64;
    let name = "Alice";
    
    // Insert Data: <100> <name> "Alice"
    let data_key = Codec::encode_data_key(uid, "name");
    let val_bytes = serde_json::to_vec(&Value::String(name.to_string())).unwrap();
    storage.insert(&data_key, &val_bytes).unwrap();
    
    // Insert Index: <name> "Alice" -> <100>
    let index_key = Codec::encode_unique_index_key("name", name);
    use byteorder::{BigEndian, ByteOrder};
    let mut uid_bytes = [0u8; 8];
    BigEndian::write_u64(&mut uid_bytes, uid);
    storage.insert(&index_key, &uid_bytes).unwrap();
    
    // 2. Initialize Resolver
    let resolver = FjallResolver::new(storage.clone());
    
    // 3. Test find_uid (Index Scan)
    let found_uid = resolver.find_uid("name", "Alice");
    assert_eq!(found_uid, Some(uid), "Should find UID 100 for Alice");
    
    // 4. Test resolve (Data Fetch)
    let val = resolver.resolve(found_uid.unwrap(), "name");
    
    match val {
        Some(Value::String(s)) => assert_eq!(s, "Alice"),
        _ => panic!("Expected String value 'Alice'"),
    }
}
