use rand::RngCore;
use std::path::Path;
use std::time::{Duration, Instant};
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

fn get_dir_size(path: impl AsRef<Path>) -> u64 {
    let mut size = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries {
            if let Ok(entry) = entry {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_dir() {
                        size += get_dir_size(entry.path());
                    } else {
                        size += metadata.len();
                    }
                }
            }
        }
    }
    size
}

#[test]
fn repro_flush_wal_truncation() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let db_path = tmp_dir.path().join("db");

    // 1. Open DB
    {
        let storage = Storage::new(&db_path, None).unwrap();
        storage.create_database("test").unwrap();

        println!("Starting Bulk Writes...");
        // 2. Write Bulk Data
        // Increase to 200MB (20,000 items * 10KB)
        let mut items = Vec::new();
        for i in 0..20000 {
            let key = format!("key_{}", i);
            // let value = vec![0u8; 1024 * 10]; // 10KB
            let mut value = vec![0u8; 1024 * 10];
            rand::thread_rng().fill_bytes(&mut value);
            items.push((i as u64, key.clone(), value.clone()));

            if items.len() >= 100 {
                let ts = Timestamp::physical_now();
                let storage_ts = Timestamp::new(ts, 0, 1);
                storage
                    .put_batch_lww("test", items.clone(), &storage_ts)
                    .unwrap();
                items.clear();
            }
        }

        println!(
            "Writes complete. DB Size (Pre-Flush): {} MB",
            get_dir_size(&db_path) / 1024 / 1024
        );

        // 3. Flush
        // This is what the user calls via flushDatabase mutation
        storage.flush().unwrap();

        // Wait a bit? unique internal queue?
        std::thread::sleep(Duration::from_millis(500));

        println!(
            "Flush complete. DB Size (Post-Flush): {} MB",
            get_dir_size(&db_path) / 1024 / 1024
        );

        println!("Listing files in DB dir:");
        for entry in std::fs::read_dir(&db_path).unwrap() {
            let entry = entry.unwrap();
            println!(
                "  {:?} ({} bytes)",
                entry.file_name(),
                entry.metadata().unwrap().len()
            );
        }

        // 4. Drop Storage (Close DB)
    }

    // 5. Re-open DB and measure time
    println!("Reopening DB...");
    let start = Instant::now();
    let _storage = Storage::new(&db_path, None).unwrap();
    let duration = start.elapsed();

    println!("Restart Duration: {:?}", duration);

    // Assert duration is fast (< 2s)
    assert!(
        duration < Duration::from_secs(2),
        "Restart took too long: {:?}",
        duration
    );
}
