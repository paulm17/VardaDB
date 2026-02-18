use fjall::{Database, KeyspaceCreateOptions, PersistMode};
use std::time::Instant;
use std::path::Path;

// This example mimics a "separate process" bulk inserting data.
// It uses the same underlying storage engine (Fjall) as VardaDB.

const INSERT_COUNT: u64 = 1_000_000;
const BATCH_SIZE: u64 = 10_000;

fn main() -> anyhow::Result<()> {
    // 1. Setup Storage
    // Use the same path as VardaDB default or a test path
    let path_str = "varda_db_data_bulk_test";
    let path = Path::new(path_str);
    
    // Clean start for the test
    let _ = std::fs::remove_dir_all(path);

    println!("Opening storage at {}...", path_str);
    
    // Config adjustments for bulk load:
    // In backend.rs: let db = Database::builder(path).open()?;
    // For tuning, we can use Config options if exposed via builder
    let db = Database::builder(path).open()?;
    
    // Mimic VardaDB structure: "default_main" keyspace
    let keyspace = db.keyspace("default_main", || KeyspaceCreateOptions::default())?;

    println!("Starting bulk insert of {} records...", INSERT_COUNT);
    let start = Instant::now();

    // 2. Bulk Insert Loop
    for i in 0..INSERT_COUNT {
        let key = format!("key:{:09}", i); // "key:000000123"
        let value = format!("value-for-{}", i);
        
        keyspace.insert(key, value)?;

        // 3. Periodic Flush / Sync
        if (i + 1) % BATCH_SIZE == 0 {
             if (i + 1) % (BATCH_SIZE * 10) == 0 {
                println!("Inserted {} records...", i + 1);
                
                // Manually persisting keyspace to ensure WAL is truncated/checkpointed
                // keyspace.persist? Not available directly as public API potentially?
                // We use db.persist(SyncAll) which rotates all memtables and syncs WAL.
                // This keeps the WAL small.
                db.persist(PersistMode::SyncAll)?;
            }
        }
    }
    
    let duration = start.elapsed();
    println!("\nDone! Inserted {} records in {:.2}s ({:.0} ops/s)", 
        INSERT_COUNT, duration.as_secs_f64(), INSERT_COUNT as f64 / duration.as_secs_f64());

    // 4. Force Final Persist
    // This ensures everything is on disk and WAL is clean for next startup.
    println!("Flushing to disk...");
    db.persist(PersistMode::SyncAll)?;
    println!("Flush complete.");

    // 5. Simulate Startup (Re-open)
    drop(keyspace);
    drop(db);
    
    println!("Re-opening database to measure startup time...");
    let start_open = Instant::now();
    let _db2 = Database::builder(path).open()?;
    println!("Startup took {:.2}ms", start_open.elapsed().as_secs_f64() * 1000.0);

    Ok(())
}
