use tempfile::tempdir;
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

#[test]
fn test_data_persistence_after_restart() {
    let dir = tempdir().unwrap();
    let db_path = dir.path();
    let db_name = "test_db";

    // 1. Start Storage, Create DB, Insert Data
    {
        let storage = Storage::new(db_path, None).unwrap();
        storage.create_database(db_name).unwrap();

        // Insert 100 items
        for i in 0..100 {
            let key = format!("key_{}", i);
            let value = format!("value_{}", i).into_bytes();
            let ts = Timestamp::physical_now();
            let timestamp = Timestamp::new(ts, 0, 1);

            // Use put_with_lww
            storage
                .put_with_lww(db_name, i as u64, &key, &value, &timestamp)
                .unwrap();
        }

        // Flush
        storage.flush().unwrap();
    } // Drop storage

    // 2. Restart Storage
    {
        let storage = Storage::new(db_path, None).unwrap();
        let databases = storage.list_databases();
        assert!(
            databases.iter().any(|(name, _)| name == db_name),
            "Database should persist"
        );

        // Check Data
        for i in 0..100 {
            let key = format!("key_{}", i);
            // Reconstruct the key used in put_with_lww (it encodes as [uid, predicate])
            // Wait, PutWithLWW encodes the key internally:
            // let key = crate::storage::codec::Codec::encode_data_key(uid, predicate);

            // We need to use storage.get to check, but storage.get takes raw key?
            // storage.get(db_name, key) -> wraps main.get(key)

            // Wait, storage.get in backend.rs takes &key.
            // But put_with_lww constructs the key internally.
            // So we can't easily use storage.get(db_name, "raw_key") if put_with_lww keys are encoded.

            // Let's look at backend.rs again to see how to read back LWW data.
            // storage.get(db_name, key) -> main.get(key).

            // I need to use the Codec to encode the key for lookup?
            // Or does Storage expose a read_lww method?

            // backend.rs:
            // pub fn get(&self, db_name: &str, key: &[u8]) ...

            // It seems `get` expects the *encoded* key if we use `put_with_lww`.
            // Let's use `storage.scan` or similar if available, or just manually encode.

            let encoded_key = vardadb::storage::codec::Codec::encode_data_key(i as u64, &key);
            let result = storage.get(db_name, &encoded_key).unwrap();

            assert!(
                result.is_some(),
                "Data for item {} should exist after restart",
                i
            );

            let val = result.unwrap();
            let expected_value = format!("value_{}", i).into_bytes();
            assert_eq!(val, expected_value, "Value should match");
        }
    }
}
