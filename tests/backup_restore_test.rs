use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_backup_and_restore() {
    let dir = tempdir().unwrap();
    let storage_path = dir.path().to_path_buf();
    let backup_dir = dir.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();

    // === Phase 1: Create data with first storage instance ===
    let storage = Arc::new(Storage::new(&storage_path, None).unwrap());
    storage.register_exit_hook();

    let sdl = "
        type Doc {
            id: ID!
            content: String
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    println!("=== Phase 1: Create 10 documents ===");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));
    let mut created_uids: Vec<String> = Vec::new();

    for i in 0..10 {
        let mutation = format!(
            "mutation {{ createDoc(input: {{ content: \"document {}\" }}) {{ uid }} }}",
            i
        );
        let response = schema.execute_with_resolver(&mutation, resolver.clone()).await;
        let v: Value = serde_json::from_str(&response).unwrap();
        let uid = v["data"]["createDoc"]["uid"]
            .as_str()
            .unwrap()
            .to_string();
        created_uids.push(uid);
    }
    println!("Created {} documents", created_uids.len());

    // Verify all 10 exist
    for (i, uid) in created_uids.iter().enumerate() {
        let query = format!("query {{ getDoc(uid: \"{}\") {{ content }} }}", uid);
        let response = schema.execute_with_resolver(&query, resolver.clone()).await;
        let v: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(v["data"]["getDoc"]["content"], format!("document {}", i));
    }
    println!("Verified all 10 documents exist");

    // === Phase 2: Create backup (before any issues) ===
    println!("=== Phase 2: Create backup ===");
    let backup_id = storage.create_backup(&backup_dir).expect("Backup failed");
    println!("Created backup: {}", backup_id);

    // Verify backup exists with correct structure
    let backup_path = backup_dir.join(&backup_id);
    assert!(backup_path.exists(), "Backup directory should exist");
    assert!(backup_path.join("default.redb").exists(), "default.redb should exist");
    assert!(backup_path.join("default.meta").exists(), "default.meta should exist");
    println!("Backup verified on disk");

    // === Phase 3: Drop storage WITHOUT flush (simulating clean shutdown) ===
    println!("=== Phase 3: Shutting down storage ===");
    drop(resolver);
    drop(storage);
    // Allow background worker threads to finish cleanup
    std::thread::sleep(std::time::Duration::from_millis(100));
    println!("Storage dropped");

    // === Phase 4: Restore files to storage path ===
    println!("=== Phase 4: Restore from backup ===");
    
    // Copy redb file
    let src_redb = backup_path.join("default.redb");
    let dest_redb = storage_path.join("default.redb");
    std::fs::copy(&src_redb, &dest_redb).expect("Failed to copy redb file");
    println!("Restored default.redb from {:?}", src_redb);
    
    // Copy tantivy index per-database directory
    let src_tantivy = backup_path.join("default_tantivy");
    if src_tantivy.exists() {
        let dest_tantivy = storage_path.join("default_tantivy");
        if dest_tantivy.exists() {
            std::fs::remove_dir_all(&dest_tantivy).ok();
        }
        copy_dir_recursive(&src_tantivy, &dest_tantivy);
        println!("Restored default_tantivy");
    }
    
    // Copy vector index files per-database
    let src_vectors = backup_path.join("default_vectors.usearch");
    if src_vectors.exists() {
        std::fs::copy(&src_vectors, storage_path.join("default_vectors.usearch")).unwrap();
        println!("Restored default_vectors.usearch");
    }
    let src_dims = backup_path.join("default_vectors.dims");
    if src_dims.exists() {
        std::fs::copy(&src_dims, storage_path.join("default_vectors.dims")).unwrap();
        println!("Restored default_vectors.dims");
    }
    
    println!("Restore completed");

    // === Phase 5: Open fresh storage and verify ===
    println!("=== Phase 5: Verify restored data ===");
    let storage2 = Arc::new(Storage::new(&storage_path, None).unwrap());
    let resolver2 = Box::new(RedbResolver::new(storage2.clone(), "default"));

    // Verify all 10 documents exist after restore
    for (i, uid) in created_uids.iter().enumerate() {
        let query = format!("query {{ getDoc(uid: \"{}\") {{ content }} }}", uid);
        let response = schema.execute_with_resolver(&query, resolver2.clone()).await;
        let v: Value = serde_json::from_str(&response).unwrap();
        
        if v["data"]["getDoc"].is_null() {
            println!("ERROR: Document {} (uid={}) not found after restore", i, uid);
            println!("Response: {:?}", v);
        }
        
        assert_eq!(
            v["data"]["getDoc"]["content"],
            format!("document {}", i),
            "Document {} should be restored (uid={})",
            i, uid
        );
    }
    println!("All 10 documents verified after restore");

    println!("=== Test passed: Backup and restore working correctly ===");
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_backups() {
    let dir = tempdir().unwrap();
    let backup_dir = dir.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();

    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    // Initially no backups
    let backups = storage.list_backups(&backup_dir).unwrap();
    assert!(backups.is_empty(), "Should have no backups initially");

    // Create a backup
    let backup_id = storage.create_backup(&backup_dir).unwrap();

    // List backups
    let backups = storage.list_backups(&backup_dir).unwrap();
    assert_eq!(backups.len(), 1, "Should have one backup");
    assert_eq!(backups[0].id, backup_id);
    assert!(!backups[0].timestamp.is_empty());
    println!("Listed backup: {:?}", backups[0]);

    // Create another backup
    std::thread::sleep(std::time::Duration::from_millis(10)); // Ensure different timestamp
    let backup_id2 = storage.create_backup(&backup_dir).unwrap();

    let backups = storage.list_backups(&backup_dir).unwrap();
    assert_eq!(backups.len(), 2, "Should have two backups");
    
    // Should be sorted newest first
    assert_eq!(backups[0].id, backup_id2);
    assert_eq!(backups[1].id, backup_id);
    println!("Backups sorted correctly: {:?}, {:?}", backups[0].id, backups[1].id);
}