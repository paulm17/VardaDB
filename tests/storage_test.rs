use vardadb::storage::backend::Storage;
use tempfile::tempdir;

#[test]
fn test_backend_persistence() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path()).unwrap();

    let key = "hello".as_bytes();
    let value = "world".as_bytes();

    // 1. Write
    storage.insert(key, value).unwrap();

    // 2. Read
    let read = storage.get(key).unwrap();
    assert_eq!(read, Some(value.to_vec()));

    // 3. Persist (Flush)
    storage.flush().unwrap();

    // 4. Re-open and verify
    drop(storage);
    let storage2 = Storage::new(dir.path()).unwrap();
    let read2 = storage2.get(key).unwrap();
    assert_eq!(read2, Some(value.to_vec()));
}
