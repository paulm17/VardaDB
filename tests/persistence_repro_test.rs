use tempfile::tempdir;
use vardadb::storage::backend::Storage;

#[test]
fn test_database_persistence() {
    let dir = tempdir().unwrap();
    let db_path = dir.path();

    // 1. Start Storage and Create DB
    {
        let storage = Storage::new(db_path, None).unwrap();
        storage.create_database("archondb").unwrap();
        assert!(
            storage.get_database("archondb").is_some(),
            "archondb should exist"
        );
        storage.flush().unwrap(); // Ensure persistence
    } // Drop storage (simulate stop)

    // 2. Restart Storage
    {
        let storage = Storage::new(db_path, None).unwrap();
        let dbs = storage.list_databases();
        println!("Databases found: {:?}", dbs);
        assert!(
            dbs.iter().any(|(name, _)| name == "archondb"),
            "archondb should persist after restart"
        );
        assert!(
            storage.get_database("archondb").is_some(),
            "archondb should be accessible"
        );
    }
}
