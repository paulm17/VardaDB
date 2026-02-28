use vardadb::storage::sqlite_backend::{SqliteBackend, SqliteTable};
use std::sync::Arc;
use std::time::Instant;
use std::path::Path;

// This example mimics a "separate process" bulk inserting data.
// It uses the same underlying storage engine (SQLite) as VardaDB.

const INSERT_COUNT: u64 = 1_000_000;
const BATCH_SIZE: u64 = 10_000;

fn main() -> anyhow::Result<()> {
    // 1. Setup Storage
    let path_str = "varda_db_data_bulk_test";
    let path = Path::new(path_str);
    
    // Clean start for the test
    let _ = std::fs::remove_dir_all(path);

    println!("Opening storage at {}...", path_str);
    
    let backend = Arc::new(SqliteBackend::new(path)?);
    backend.create_main_table("default_main")?;
    let table = SqliteTable::new("default_main".to_string(), backend.clone());

    println!("Starting bulk insert of {} records...", INSERT_COUNT);
    let start = Instant::now();

    // 2. Bulk Insert Loop (using batched transactions for performance)
    let mut batch_count = 0u64;
    for chunk_start in (0..INSERT_COUNT).step_by(BATCH_SIZE as usize) {
        let chunk_end = (chunk_start + BATCH_SIZE).min(INSERT_COUNT);
        
        backend.write_batch(|conn| {
            for i in chunk_start..chunk_end {
                let key = format!("key:{:09}", i);
                let value = format!("value-for-{}", i);
                table.batch_insert_on_conn(conn, key.as_bytes(), value.as_bytes())?;
            }
            Ok(())
        })?;
        
        batch_count += 1;
        if batch_count % 10 == 0 {
            println!("Inserted {} records...", chunk_end);
        }
    }
    
    let duration = start.elapsed();
    println!("\nDone! Inserted {} records in {:.2}s ({:.0} ops/s)", 
        INSERT_COUNT, duration.as_secs_f64(), INSERT_COUNT as f64 / duration.as_secs_f64());

    // 3. WAL Checkpoint (equivalent to Fjall's PersistMode::SyncAll)
    println!("Checkpointing WAL...");
    backend.shutdown()?;
    println!("Checkpoint complete.");

    // 4. Simulate Startup (Re-open)
    drop(table);
    drop(backend);
    
    println!("Re-opening database to measure startup time...");
    let start_open = Instant::now();
    let _backend2 = SqliteBackend::new(path)?;
    println!("Startup took {:.2}ms", start_open.elapsed().as_secs_f64() * 1000.0);

    Ok(())
}
