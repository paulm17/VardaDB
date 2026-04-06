use tempfile::tempdir;
use vardadb::storage::backend::Storage;

#[test]
fn test_backend_persistence() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path(), None).unwrap();

    let key = "hello".as_bytes();
    let value = "world".as_bytes();

    // 1. Write
    storage.insert("default", key, value).unwrap();

    // 2. Read back
    let read = storage.get("default", key).unwrap();
    assert_eq!(read, Some(value.to_vec()));

    // 3. Flush to redb
    storage.flush().unwrap();

    // 4. Re-open the same redb file and verify durability
    drop(storage);
    let storage2 = Storage::new(dir.path(), None).unwrap();
    let read2 = storage2.get("default", key).unwrap();
    assert_eq!(read2, Some(value.to_vec()));
}
