use crate::storage::sqlite_backend::{SqliteBackend, SqliteTable};
use byteorder::{BigEndian, ByteOrder};
use jobs::{JobStore, Queue};
use permissions::storage::auth_store::AuthStore;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use tracing::{error, info};
use uuid::Uuid;

// Global registry for flushing
static ACTIVE_STORAGES: std::sync::OnceLock<Mutex<Vec<Weak<Storage>>>> = std::sync::OnceLock::new();

extern "C" fn crash_handler(_signum: libc::c_int) {
    println!("\n[VardaDB] Process exiting. Global shutdown hook triggered...");
    if let Some(mutex) = ACTIVE_STORAGES.get() {
        if let Ok(mut list) = mutex.lock() {
            let count = list.len();
            if count > 0 {
                println!("[VardaDB] Flushing {} active storage instances...", count);
                for weak in list.drain(..) {
                    if let Some(storage) = weak.upgrade() {
                        let _ = storage.flush();
                    }
                }
                println!("[VardaDB] Flush complete. Exiting.");
            }
        }
    }
    std::process::exit(0);
}

pub struct Storage {
    pub backend: Arc<SqliteBackend>,
    // Database Management
    // Map: DatabaseName -> (Main Table, History Table)
    pub keyspaces: std::sync::RwLock<std::collections::HashMap<String, (SqliteTable, SqliteTable)>>,

    // System Tables (Global)
    pub sys_table: SqliteTable,        // SYSTEM: Config (NodeID, etc)
    pub quarantine_table: SqliteTable, // QUARANTINE: Global
    pub metrics_table: SqliteTable,    // METRICS: Time-series metrics
    pub traces_table: SqliteTable,     // TRACES: Trace spans
    pub auth_store: AuthStore,         // AUTH: Authorization tuples and attributes

    pub jobs_store: Arc<JobStore<SqliteTable>>, // JOB STORE (Global)
    pub system_queue: Arc<Queue<SqliteTable>>,  // DEFAULT QUEUE (Global)
    pub node_id: u64,
    pub clock: std::sync::Mutex<crate::storage::timestamp::Timestamp>,
    pub vector_tx: std::sync::mpsc::SyncSender<(u64, Vec<f64>)>,

    // Incremental Fingerprints: DbName -> (Hash, Count)
    pub fingerprints: std::sync::Arc<
        dashmap::DashMap<String, (std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU64)>,
    >,

    // Flag indicating fingerprints are ready for sync
    pub fingerprints_ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Storage {
    pub fn new(path: impl AsRef<Path>, node_id_override: Option<u64>) -> anyhow::Result<Self> {
        let backend = Arc::new(SqliteBackend::new(path.as_ref())?);

        // Create System Tables
        backend.create_table("sys")?;
        backend.create_table("quarantine")?;
        backend.create_table("sys_metrics")?;
        backend.create_table("sys_traces")?;
        backend.create_table("vectors")?;
        backend.create_native_search_tables()?;
        backend.create_table("auth_tuples")?;
        backend.create_table("auth_attributes")?;
        backend.create_table("jobs")?;
        // Auth login tables (was previously created by AuthStore::init from Database)
        backend.create_table("auth_users")?;
        backend.create_table("auth_tokens")?;
        backend.create_table("auth_confirmations")?;
        backend.create_table("auth_identities")?;
        backend.create_table("auth_social_state")?;
        backend.create_table("auth_keys")?;

        let sys_table = SqliteTable::new("sys".to_string(), backend.clone());
        let quarantine_table = SqliteTable::new("quarantine".to_string(), backend.clone());
        let metrics_table = SqliteTable::new("sys_metrics".to_string(), backend.clone());
        let traces_table = SqliteTable::new("sys_traces".to_string(), backend.clone());

        // AuthZ Store
        let auth_tuples_table = SqliteTable::new("auth_tuples".to_string(), backend.clone());
        let auth_attributes_table =
            SqliteTable::new("auth_attributes".to_string(), backend.clone());
        let auth_store = AuthStore::new(
            std::sync::Arc::new(auth_tuples_table)
                as std::sync::Arc<dyn permissions::storage::auth_store::KvStore>,
            std::sync::Arc::new(auth_attributes_table)
                as std::sync::Arc<dyn permissions::storage::auth_store::KvStore>,
        );

        // Vector Worker (Bounded Channel)
        let (tx, rx) = std::sync::mpsc::sync_channel::<(u64, Vec<f64>)>(5000);
        let worker_backend = backend.clone();

        std::thread::spawn(move || {
            println!("Storage: Vector Background Worker Started");
            while let Ok((uid, vec)) = rx.recv() {
                let vec_f32: Vec<f32> = vec.iter().map(|v| *v as f32).collect();
                let vec_bytes = unsafe {
                    std::slice::from_raw_parts(vec_f32.as_ptr() as *const u8, vec_f32.len() * 4)
                };

                let _ = worker_backend.with_writer(|conn| {
                    // Upsert vector into vec_data table
                    conn.execute(
                        "INSERT OR REPLACE INTO vec_data(uid, embedding) VALUES (?1, ?2)",
                        rusqlite::params![uid as i64, vec_bytes],
                    )?;
                    Ok(())
                });
            }
            println!("Storage: Vector Background Worker Stopped");
        });

        // Jobs Store
        let jobs_table = SqliteTable::new("jobs".to_string(), backend.clone());
        let jobs_store = Arc::new(JobStore::new(Arc::new(jobs_table)));
        let system_queue = Arc::new(Queue::new("system_queue".to_string(), jobs_store.clone()));

        // Auto-discover databases from existing tables
        let mut initial_keyspaces = std::collections::HashMap::new();

        // Always ensure "default" database exists
        backend.create_main_table("default_main")?;
        backend.create_table("default_history")?;
        let default_main = SqliteTable::new_main("default_main".to_string(), backend.clone());
        let default_history = SqliteTable::new("default_history".to_string(), backend.clone());
        initial_keyspaces.insert("default".to_string(), (default_main, default_history));

        // Auto-discover other databases from sqlite_master
        let all_tables = backend.list_tables();
        println!("Storage: All tables in database: {:?}", all_tables);

        for table_name in &all_tables {
            if table_name.ends_with("_main") && table_name != "default_main" {
                let db_name = table_name.trim_end_matches("_main");
                let history_name = format!("{}_history", db_name);

                if all_tables.contains(&history_name) {
                    println!("Storage: Discovered database '{}'", db_name);
                    let main_table = SqliteTable::new_main(table_name.clone(), backend.clone());
                    let hist_table = SqliteTable::new(history_name, backend.clone());
                    initial_keyspaces.insert(db_name.to_string(), (main_table, hist_table));
                }
            }
        }

        // Load or Generate Node ID
        let node_id = if let Some(id) = node_id_override {
            sys_table.insert("node_id", &id.to_be_bytes())?;
            id
        } else if let Some(val) = sys_table.get(b"node_id")? {
            if val.len() == 8 {
                BigEndian::read_u64(&val)
            } else {
                let new_id = Uuid::new_v4().as_u128() as u64;
                sys_table.insert("node_id", &new_id.to_be_bytes())?;
                new_id
            }
        } else {
            let new_id = Uuid::new_v4().as_u128() as u64;
            sys_table.insert("node_id", &new_id.to_be_bytes())?;
            new_id
        };

        info!("Storage: Initialized with Node ID: {}", node_id);

        let clock = std::sync::Mutex::new(if let Some(val) = sys_table.get(b"clock")? {
            if val.len() >= 16 {
                let bytes: [u8; 16] = val[0..16].try_into().unwrap();
                let stored = crate::storage::timestamp::Timestamp::from_bytes(&bytes);
                let now = crate::storage::timestamp::Timestamp::physical_now();
                if stored.millis >= now {
                    stored
                } else {
                    crate::storage::timestamp::Timestamp::new(now, 0, node_id)
                }
            } else {
                crate::storage::timestamp::Timestamp::new(
                    crate::storage::timestamp::Timestamp::physical_now(),
                    0,
                    node_id,
                )
            }
        } else {
            crate::storage::timestamp::Timestamp::new(
                crate::storage::timestamp::Timestamp::physical_now(),
                0,
                node_id,
            )
        });

        let storage = Self {
            backend,
            keyspaces: std::sync::RwLock::new(initial_keyspaces),
            sys_table,
            quarantine_table,
            metrics_table,
            traces_table,
            auth_store,
            jobs_store,
            system_queue,
            node_id,
            clock,
            vector_tx: tx,
            fingerprints: std::sync::Arc::new(dashmap::DashMap::new()),
            fingerprints_ready: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        // Restore Fingerprints (Fast load / Fallback to scan)
        if let Err(e) = storage.restore_fingerprints() {
            error!("Storage: Failed to restore/rebuild fingerprints: {}", e);
        }

        Ok(storage)
    }

    pub fn register_exit_hook(self: &Arc<Self>) {
        let mutex = ACTIVE_STORAGES.get_or_init(|| {
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(5));
                unsafe {
                    libc::signal(libc::SIGINT, crash_handler as libc::sighandler_t);
                    libc::signal(libc::SIGTERM, crash_handler as libc::sighandler_t);
                }
            });
            Mutex::new(Vec::new())
        });

        if let Ok(mut list) = mutex.lock() {
            list.push(Arc::downgrade(self));
        }
    }

    // --- Database Management ---

    pub fn create_database(&self, name: &str) -> anyhow::Result<()> {
        let main_name = format!("{}_main", name);
        let history_name = format!("{}_history", name);

        self.backend.create_main_table(&main_name)?;
        self.backend.create_table(&history_name)?;

        let main_table = SqliteTable::new_main(main_name, self.backend.clone());
        let history_table = SqliteTable::new(history_name, self.backend.clone());

        let mut lock = self.keyspaces.write().unwrap();
        lock.insert(name.to_string(), (main_table, history_table));

        Ok(())
    }

    pub fn list_databases(&self) -> Vec<String> {
        let lock = self.keyspaces.read().unwrap();
        lock.keys().cloned().collect()
    }

    pub fn get_database(&self, name: &str) -> Option<(SqliteTable, SqliteTable)> {
        let lock = self.keyspaces.read().unwrap();
        lock.get(name).cloned()
    }

    pub fn delete_database(&self, name: &str) -> anyhow::Result<()> {
        {
            let mut lock = self.keyspaces.write().unwrap();
            if lock.remove(name).is_none() {
                return Err(anyhow::anyhow!("Database not found"));
            }
        };

        let main_name = format!("{}_main", name);
        let history_name = format!("{}_history", name);
        self.backend.drop_table(&main_name)?;
        self.backend.drop_table(&history_name)?;
        Ok(())
    }

    // --- Data Access ---

    pub fn get(&self, db_name: &str, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;

        // Main table stores value in the value column and ts separately
        // We need to return just the value (no timestamp prefix stripping needed
        // because SQLite stores ts in its own column)
        main.get(key)
    }

    /// Last-Write-Wins Put — Atomic via SQLite ON CONFLICT
    pub fn put_with_lww(
        &self,
        db_name: &str,
        uid: u64,
        predicate: &str,
        value: &[u8],
        timestamp: &crate::storage::timestamp::Timestamp,
    ) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, history) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;

        let key = crate::storage::codec::Codec::encode_data_key(uid, predicate);
        let ts_bytes = timestamp.to_bytes();

        // Atomic LWW upsert — ON CONFLICT only updates if new ts > existing ts
        main.upsert_lww(&key, value, &ts_bytes)?;

        // Write HISTORY
        let hist_key = crate::storage::codec::Codec::encode_history_key(&ts_bytes, uid, predicate);
        history.insert(&hist_key, value)?;
        self.update_history_hash(db_name, &hist_key, value);

        Ok(())
    }

    /// Batch Last-Write-Wins Put — Uses SQLite transaction for atomicity
    pub fn put_batch_lww(
        &self,
        db_name: &str,
        items: Vec<(u64, String, Vec<u8>)>,
        timestamp: &crate::storage::timestamp::Timestamp,
    ) -> anyhow::Result<()> {
        use std::time::Instant;
        let op_start = Instant::now();
        let item_count = items.len();

        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _history) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;
        let lock_time = op_start.elapsed();

        let ts_bytes = timestamp.to_bytes();
        let main_clone = main.clone();

        let batch_start = Instant::now();
        self.backend.write_batch(|conn| {
            for (uid, predicate, value) in &items {
                let key = crate::storage::codec::Codec::encode_data_key(*uid, predicate);
                main_clone.batch_upsert_lww_on_conn(conn, &key, value, &ts_bytes)?;
            }
            Ok(())
        })?;
        let commit_time = batch_start.elapsed();

        let total_time = op_start.elapsed();

        // Log if any phase took > 50ms
        if crate::debug_logging() && total_time.as_millis() > 50 {
            println!(
                "⏱️ put_batch_lww SLOW: {} items | lock={:?}, commit={:?}, total={:?}",
                item_count, lock_time, commit_time, total_time
            );
        }

        Ok(())
    }

    /// Last-Write-Wins Delete
    pub fn delete_with_lww(
        &self,
        db_name: &str,
        uid: u64,
        predicate: &str,
        timestamp: &crate::storage::timestamp::Timestamp,
    ) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, history) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;

        let key = crate::storage::codec::Codec::encode_data_key(uid, predicate);
        let ts_bytes = timestamp.to_bytes();

        // Delete only if not stale
        let deleted = main.delete_lww(&key, &ts_bytes)?;
        if !deleted {
            return Ok(()); // Stale delete
        }

        // Write Tombstone to HISTORY
        let hist_key = crate::storage::codec::Codec::encode_history_key(&ts_bytes, uid, predicate);
        history.insert(&hist_key, &[])?;
        self.update_history_hash(db_name, &hist_key, &[]);

        Ok(())
    }

    // Direct Insert (Legacy/Raw)
    pub fn insert(&self, db_name: &str, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;
        main.insert(key, value)?;
        Ok(())
    }

    /// Delete a raw key from the main table
    pub fn delete_key(&self, db_name: &str, key: &[u8]) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;
        main.remove(key)?;
        Ok(())
    }

    pub fn remove(&self, db_name: &str, key: &[u8]) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;
        main.remove(key)?;
        Ok(())
    }

    pub fn contains_key(&self, db_name: &str, key: &[u8]) -> anyhow::Result<bool> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;
        main.contains_key(key)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        println!("Storage: Flush starting...");

        // Persist clock state
        {
            let clock = self.clock.lock().unwrap();
            let _ = self.sys_table.insert("clock", &clock.to_bytes());
        }

        // Persist fingerprints
        if let Err(e) = self.persist_fingerprints() {
            eprintln!("Storage: Failed to persist fingerprints: {}", e);
        }

        // WAL checkpoint — merges WAL into main database file
        self.backend.shutdown()?;

        println!("Storage: Flush complete (WAL checkpoint done)");
        Ok(())
    }

    /// SQLite B-trees don't need compaction — this is a no-op.
    pub fn needs_compaction(&self) -> bool {
        false
    }

    /// No-op for SQLite. Returns 0ms.
    pub fn compact(&self) -> anyhow::Result<u64> {
        Ok(0)
    }

    // --- Sync & Quarantine ---

    pub fn get_history_range(
        &self,
        db_name: &str,
        start_ts: Option<&crate::storage::timestamp::Timestamp>,
        end_ts: Option<&crate::storage::timestamp::Timestamp>,
    ) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (_, history) = keyspaces
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?;

        let results = if let (Some(start), Some(end)) = (start_ts, end_ts) {
            let lower = start.to_bytes().to_vec();
            let mut upper = end.to_bytes().to_vec();
            upper.push(0xFF);
            history.range(&lower, &upper)
        } else if let Some(start) = start_ts {
            let lower = start.to_bytes().to_vec();
            // Scan from start to end of table
            history.prefix(&lower)
        } else {
            // Full scan
            history.iter()
        };

        Ok(results)
    }

    pub fn put_quarantine(
        &self,
        uid: u64,
        predicate: &str,
        value: &[u8],
        timestamp: &crate::storage::timestamp::Timestamp,
    ) -> anyhow::Result<()> {
        let key = crate::storage::codec::Codec::encode_quarantine_key(uid, predicate);
        let mut new_val = Vec::with_capacity(16 + value.len());
        new_val.extend_from_slice(&timestamp.to_bytes());
        new_val.extend_from_slice(value);

        self.quarantine_table.insert(&key, &new_val)?;
        Ok(())
    }

    pub fn scan_quarantine(&self) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self.quarantine_table.iter())
    }

    pub fn delete_quarantine(&self, key: &[u8]) -> anyhow::Result<()> {
        self.quarantine_table.remove(key)?;
        Ok(())
    }

    pub fn next_timestamp(&self) -> crate::storage::timestamp::Timestamp {
        let mut clock = self.clock.lock().unwrap();
        let now = crate::storage::timestamp::Timestamp::physical_now();
        let next = clock.send(now);
        *clock = next;
        next
    }

    pub fn update_clock(&self, remote_ts: &crate::storage::timestamp::Timestamp) {
        let mut clock = self.clock.lock().unwrap();
        let now = crate::storage::timestamp::Timestamp::physical_now();
        let next = clock.receive(remote_ts, now);
        *clock = next;
    }

    // --- Vector Operations ---

    pub fn put_vector(&self, uid: u64, vector: Vec<f64>) -> anyhow::Result<()> {
        self.vector_tx
            .send((uid, vector))
            .map_err(|e| anyhow::anyhow!("Failed to send vector to worker: {}", e))?;
        Ok(())
    }

    pub fn delete_vector(&self, uid: u64) -> anyhow::Result<()> {
        let uid_i64 = uid as i64;
        self.backend.with_writer(|conn| {
            conn.execute(
                "DELETE FROM vec_data WHERE uid = ?1",
                rusqlite::params![uid_i64],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn search_vectors(&self, query: &[f64], k: usize) -> anyhow::Result<Vec<(u64, f64)>> {
        let vec_f32: Vec<f32> = query.iter().map(|v| *v as f32).collect();
        let vec_bytes =
            unsafe { std::slice::from_raw_parts(vec_f32.as_ptr() as *const u8, vec_f32.len() * 4) };

        let conn = self.backend.get_reader()?;
        let res = (|| -> anyhow::Result<Vec<(u64, f64)>> {
            let mut stmt = conn.prepare("SELECT uid, distance FROM vec_data WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance")?;
            let rows = stmt.query_map(rusqlite::params![vec_bytes, k as i64], |row| {
                let uid: i64 = row.get(0)?;
                let distance: f64 = row.get(1)?;
                Ok((uid as u64, distance))
            })?;

            let mut results = Vec::new();
            for r in rows {
                if let Ok(val) = r {
                    results.push(val);
                }
            }
            Ok(results)
        })();

        self.backend.return_reader(conn);
        res
    }

    // --- Incremental Fingerprint Logic ---

    pub fn hash_item(key: &[u8], value: &[u8]) -> u64 {
        const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
        const PRIME: u64 = 0x100000001b3;

        let mut hash = OFFSET_BASIS;
        for &byte in key.iter().chain(value.iter()) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(PRIME);
        }
        hash
    }

    pub fn rebuild_fingerprints(&self) -> anyhow::Result<()> {
        println!("Storage: Rebuilding fingerprints...");
        let keyspaces = self.keyspaces.read().unwrap();

        for (name, (_, history)) in keyspaces.iter() {
            println!("Storage: Scanning history for '{}'...", name);
            let mut h: u64 = 0;
            let mut c: u64 = 0;

            for (k, v) in history.iter() {
                h ^= Self::hash_item(&k, &v);
                c += 1;
            }

            self.fingerprints.insert(
                name.clone(),
                (
                    std::sync::atomic::AtomicU64::new(h),
                    std::sync::atomic::AtomicU64::new(c),
                ),
            );
            println!("Storage: Rebuilt '{}' (Count: {}, Hash: {:x})", name, c, h);
        }
        Ok(())
    }

    pub fn get_global_fingerprint(&self, db_name: &str) -> Option<(u64, u64)> {
        if let Some(entry) = self.fingerprints.get(db_name) {
            let (h_atomic, c_atomic) = entry.value();
            use std::sync::atomic::Ordering;
            let h = h_atomic.load(Ordering::Relaxed);
            let c = c_atomic.load(Ordering::Relaxed);
            Some((h, c))
        } else {
            None
        }
    }

    pub fn persist_fingerprints(&self) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;
        for entry in self.fingerprints.iter() {
            let db_name = entry.key();
            let (h_atomic, c_atomic) = entry.value();
            let h = h_atomic.load(Ordering::Relaxed);
            let c = c_atomic.load(Ordering::Relaxed);

            let mut buf = [0u8; 16];
            BigEndian::write_u64(&mut buf[0..8], c);
            BigEndian::write_u64(&mut buf[8..16], h);

            let key = format!("fp:{}", db_name);
            self.sys_table.insert(key.as_bytes(), &buf)?;
        }
        Ok(())
    }

    pub fn restore_fingerprints(&self) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let mut needs_rebuild: Vec<(String, SqliteTable)> = Vec::new();

        for (name, (_, history_table)) in keyspaces.iter() {
            let key = format!("fp:{}", name);
            if let Some(val) = self.sys_table.get(key.as_bytes())? {
                if val.len() == 16 {
                    let c = BigEndian::read_u64(&val[0..8]);
                    let h = BigEndian::read_u64(&val[8..16]);

                    self.fingerprints.insert(
                        name.clone(),
                        (
                            std::sync::atomic::AtomicU64::new(h),
                            std::sync::atomic::AtomicU64::new(c),
                        ),
                    );
                    println!(
                        "Storage: Restored fingerprint for '{}' (Count: {}, Hash: {:x})",
                        name, c, h
                    );
                    continue;
                }
            }

            println!(
                "Storage: Fingerprint missing for '{}' - will rebuild in background",
                name
            );
            self.fingerprints.insert(
                name.clone(),
                (
                    std::sync::atomic::AtomicU64::new(0),
                    std::sync::atomic::AtomicU64::new(0),
                ),
            );
            needs_rebuild.push((name.clone(), history_table.clone()));
        }

        drop(keyspaces);

        use std::sync::atomic::Ordering;
        self.fingerprints_ready.store(true, Ordering::Release);

        if needs_rebuild.is_empty() {
            println!("Storage: All fingerprints ready (restored from disk)");
        } else {
            println!(
                "Storage: Fingerprints ready (initialized to zero, will rebuild in background)"
            );
            self.spawn_fingerprint_rebuild(needs_rebuild);
        }

        Ok(())
    }

    /// Spawn a background thread to rebuild fingerprints for the given databases.
    fn spawn_fingerprint_rebuild(&self, db_list: Vec<(String, SqliteTable)>) {
        // SqliteTable is Clone+Send, so we can move it into the thread safely
        // Clone Arcs to keep fingerprints alive in background thread
        let fingerprints = self.fingerprints.clone();
        let ready_flag = self.fingerprints_ready.clone();
        let sys_table = self.sys_table.clone();

        std::thread::spawn(move || {
            println!(
                "Storage: Background fingerprint rebuild started for {} database(s)",
                db_list.len()
            );
            let start = std::time::Instant::now();

            for (name, history_table) in db_list {
                let scan_start = std::time::Instant::now();
                let mut hash: u64 = 0;
                let mut count: u64 = 0;

                for (k, v) in history_table.iter() {
                    hash ^= Self::hash_item(&k, &v);
                    count += 1;
                    if count % 100_000 == 0 {
                        println!(
                            "Storage: Fingerprint rebuild for '{}' - scanned {} items ({:.1}s)...",
                            name,
                            count,
                            scan_start.elapsed().as_secs_f64()
                        );
                    }
                }
                println!(
                    "Storage: Fingerprint rebuild for '{}' completed - {} items in {:.1}s",
                    name,
                    count,
                    scan_start.elapsed().as_secs_f64()
                );

                // Update the DashMap entry
                use std::sync::atomic::Ordering;
                if let Some(entry) = fingerprints.get(&name) {
                    let (h_atomic, c_atomic) = entry.value();
                    h_atomic.store(hash, Ordering::Release);
                    c_atomic.store(count, Ordering::Release);
                }

                // Persist to sys table
                let key = format!("fp:{}", name);
                let mut buf = vec![0u8; 16];
                BigEndian::write_u64(&mut buf[0..8], count);
                BigEndian::write_u64(&mut buf[8..16], hash);
                if let Err(e) = sys_table.insert(key.as_bytes(), &buf) {
                    eprintln!(
                        "Storage: Failed to persist rebuilt fingerprint for '{}': {}",
                        name, e
                    );
                }

                println!(
                    "Storage: Rebuilt fingerprint for '{}' (Count: {}, Hash: {:x})",
                    name, count, hash
                );
            }

            ready_flag.store(true, std::sync::atomic::Ordering::Release);
            println!(
                "Storage: Background fingerprint rebuild complete in {:?}",
                start.elapsed()
            );
        });
    }

    /// Wait for fingerprints to be ready. Used by SyncManager before starting gossip.
    pub fn wait_for_fingerprints(&self) {
        use std::sync::atomic::Ordering;

        if self.fingerprints_ready.load(Ordering::Acquire) {
            return;
        }

        println!("Storage: Waiting for fingerprints to be ready...");
        while !self.fingerprints_ready.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        println!("Storage: Fingerprints are now ready");
    }

    fn update_history_hash(&self, db_name: &str, key: &[u8], value: &[u8]) {
        let item_hash = Self::hash_item(key, value);

        if let Some(entry) = self.fingerprints.get(db_name) {
            let (h_stat, c_stat) = entry.value();
            use std::sync::atomic::Ordering;
            h_stat.fetch_xor(item_hash, Ordering::Relaxed);
            c_stat.fetch_add(1, Ordering::Relaxed);
        } else {
            use std::sync::atomic::AtomicU64;
            self.fingerprints.insert(
                db_name.to_string(),
                (AtomicU64::new(item_hash), AtomicU64::new(1)),
            );
        }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        println!("Storage Drop triggered. Ensuring data is flushed...");
        if let Err(e) = self.flush() {
            eprintln!("Failed to flush storage during drop: {}", e);
        } else {
            println!("Storage flushed successfully on drop.");
        }
    }
}
