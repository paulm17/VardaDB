use crate::storage::sqlite_backend::{SqliteBackend, SqliteTable};
use byteorder::{BigEndian, ByteOrder};
use permissions::storage::auth_store::AuthStore;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use tracing::{error, info};
use uuid::Uuid;

macro_rules! dbg_info {
    ($($arg:tt)*) => {
        if crate::debug_logging() {
            info!($($arg)*);
        }
    };
}

macro_rules! dbg_println {
    ($($arg:tt)*) => {
        if crate::debug_logging() {
            println!($($arg)*);
        }
    };
}

// Global registry for flushing
static ACTIVE_STORAGES: std::sync::OnceLock<Mutex<Vec<Weak<Storage>>>> = std::sync::OnceLock::new();

extern "C" fn crash_handler(_signum: libc::c_int) {
    dbg_println!("\n[VardaDB] Process exiting. Global shutdown hook triggered...");
    crate::llm::shutdown_all_managed_processes();
    if let Some(mutex) = ACTIVE_STORAGES.get() {
        if let Ok(mut list) = mutex.lock() {
            let count = list.len();
            if count > 0 {
                dbg_println!("[VardaDB] Flushing {} active storage instances...", count);
                for weak in list.drain(..) {
                    if let Some(storage) = weak.upgrade() {
                        let _ = storage.flush();
                    }
                }
                dbg_println!("[VardaDB] Flush complete. Exiting.");
            }
        }
    }
    std::process::exit(0);
}

pub struct Storage {
    pub backends: dashmap::DashMap<String, Arc<SqliteBackend>>,
    pub base_path: PathBuf,
    // Database Management
    // Map: DatabaseName -> (Main Table, History Table)
    pub keyspaces: std::sync::RwLock<std::collections::HashMap<String, (SqliteTable, SqliteTable)>>,

    // System Tables (Global)
    pub sys_table: SqliteTable,        // SYSTEM: Config (NodeID, etc)
    pub quarantine_table: SqliteTable, // QUARANTINE: Global
    pub metrics_table: SqliteTable,    // METRICS: Time-series metrics
    pub traces_table: SqliteTable,     // TRACES: Trace spans
    pub auth_store: AuthStore,         // AUTH: Authorization tuples and attributes
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
    fn initialize_database_tables(
        backend: &SqliteBackend,
        main_table_name: &str,
        history_table_name: &str,
    ) -> anyhow::Result<()> {
        backend.create_main_table(main_table_name)?;
        backend.create_table(history_table_name)?;
        backend.create_native_search_tables()?;
        Ok(())
    }

    pub fn new(path: impl AsRef<Path>, node_id_override: Option<u64>) -> anyhow::Result<Self> {
        let base_path = path.as_ref().to_path_buf();
        let default_db_path = base_path.join("default.db");
        dbg_info!(
            storage_path = %base_path.display(),
            default_db_path = %default_db_path.display(),
            node_id_override = ?node_id_override,
            "Storage: starting initialization"
        );
        let default_backend = Arc::new(SqliteBackend::new(&default_db_path)?);
        dbg_info!(
            default_db_path = %default_db_path.display(),
            "Storage: opened default backend"
        );

        let backends = dashmap::DashMap::new();
        backends.insert("default".to_string(), default_backend.clone());

        // Create System Tables on default database
        default_backend.create_table("sys")?;
        default_backend.create_table("quarantine")?;
        default_backend.create_table("sys_metrics")?;
        default_backend.create_table("sys_traces")?;
        default_backend.create_table("vectors")?;
        default_backend.create_native_search_tables()?;
        default_backend.create_table("auth_tuples")?;
        default_backend.create_table("auth_attributes")?;
        // Auth login tables
        default_backend.create_table("auth_users")?;
        default_backend.create_table("auth_tokens")?;
        default_backend.create_table("auth_confirmations")?;
        default_backend.create_table("auth_identities")?;
        default_backend.create_table("auth_social_state")?;
        default_backend.create_table("auth_keys")?;

        let sys_table = SqliteTable::new("sys".to_string(), default_backend.clone());
        let quarantine_table = SqliteTable::new("quarantine".to_string(), default_backend.clone());
        let metrics_table = SqliteTable::new("sys_metrics".to_string(), default_backend.clone());
        let traces_table = SqliteTable::new("sys_traces".to_string(), default_backend.clone());

        // AuthZ Store
        let auth_tuples_table =
            SqliteTable::new("auth_tuples".to_string(), default_backend.clone());
        let auth_attributes_table =
            SqliteTable::new("auth_attributes".to_string(), default_backend.clone());
        let auth_store = AuthStore::new(
            std::sync::Arc::new(auth_tuples_table)
                as std::sync::Arc<dyn permissions::storage::auth_store::KvStore>,
            std::sync::Arc::new(auth_attributes_table)
                as std::sync::Arc<dyn permissions::storage::auth_store::KvStore>,
        );

        // Vector Worker (Bounded Channel)
        let (tx, rx) = std::sync::mpsc::sync_channel::<(u64, Vec<f64>)>(5000);
        let worker_backend = default_backend.clone();

        std::thread::spawn(move || {
            dbg_println!("Storage: Vector Background Worker Started");
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
            dbg_println!("Storage: Vector Background Worker Stopped");
        });

        // Auto-discover databases from registry
        let mut initial_keyspaces = std::collections::HashMap::new();

        // Always ensure "default" database exists
        Self::initialize_database_tables(
            default_backend.as_ref(),
            "default_main",
            "default_history",
        )?;
        let default_main =
            SqliteTable::new_main("default_main".to_string(), default_backend.clone());
        let default_history =
            SqliteTable::new("default_history".to_string(), default_backend.clone());
        initial_keyspaces.insert("default".to_string(), (default_main, default_history));

        let registry = sys_table.prefix(b"db:");
        dbg_info!(
            registry_entries = registry.len(),
            "Storage: loaded database registry entries"
        );
        for (k, v) in registry {
            let db_name = String::from_utf8(k[3..].to_vec()).unwrap_or_default();
            let db_path_str = String::from_utf8(v).unwrap_or_default();
            if db_name.is_empty() || db_path_str.is_empty() {
                continue;
            }

            let db_path = std::path::PathBuf::from(db_path_str);
            dbg_info!(
                db_name = %db_name,
                db_path = %db_path.display(),
                "Storage: attempting auto-load of registered database"
            );
            if !db_path.exists() {
                dbg_println!("Storage [Warning]: Registered database '{}' file not found at {:?}. Skipping auto-load.", db_name, db_path);
                error!(
                    db_name = %db_name,
                    db_path = %db_path.display(),
                    "Storage: registered database file missing during auto-load"
                );
                continue;
            }

            match SqliteBackend::new(&db_path) {
                Ok(b) => {
                    let b_arc = Arc::new(b);
                    let main_name = format!("{}_main", db_name);
                    let hist_name = format!("{}_history", db_name);

                    Self::initialize_database_tables(b_arc.as_ref(), &main_name, &hist_name)?;

                    backends.insert(db_name.clone(), b_arc.clone());

                    let main_table = SqliteTable::new_main(main_name, b_arc.clone());
                    let hist_table = SqliteTable::new(hist_name, b_arc.clone());

                    initial_keyspaces.insert(db_name.clone(), (main_table, hist_table));
                    dbg_println!("Storage: Discovered and loaded database '{}'", db_name);
                    dbg_info!(
                        db_name = %db_name,
                        db_path = %db_path.display(),
                        "Storage: discovered and loaded database"
                    );
                }
                Err(e) => {
                    error!(
                        "Storage [Error]: Failed to open database '{}' at {:?}: {}",
                        db_name, db_path, e
                    );
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

        dbg_info!("Storage: Initialized with Node ID: {}", node_id);

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
            backends,
            base_path,
            keyspaces: std::sync::RwLock::new(initial_keyspaces),
            sys_table,
            quarantine_table,
            metrics_table,
            traces_table,
            auth_store,
            node_id,
            clock,
            vector_tx: tx,
            fingerprints: std::sync::Arc::new(dashmap::DashMap::new()),
            fingerprints_ready: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        dbg_info!(
            database_count = storage.keyspaces.read().unwrap().len(),
            backend_count = storage.backends.len(),
            "Storage: core initialization complete, restoring fingerprints"
        );

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
                    libc::signal(
                        libc::SIGINT,
                        crash_handler as *const () as libc::sighandler_t,
                    );
                    libc::signal(
                        libc::SIGTERM,
                        crash_handler as *const () as libc::sighandler_t,
                    );
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
        self.create_database_with_path(name, None)
    }

    pub fn create_database_with_path(
        &self,
        name: &str,
        custom_path: Option<String>,
    ) -> anyhow::Result<()> {
        let db_path = if let Some(p) = custom_path {
            PathBuf::from(p)
        } else {
            self.base_path.join(format!("{}.db", name))
        };

        let new_backend = Arc::new(SqliteBackend::new(&db_path)?);

        let main_name = format!("{}_main", name);
        let history_name = format!("{}_history", name);

        Self::initialize_database_tables(new_backend.as_ref(), &main_name, &history_name)?;

        let main_table = SqliteTable::new_main(main_name.clone(), new_backend.clone());
        let history_table = SqliteTable::new(history_name, new_backend.clone());

        self.backends.insert(name.to_string(), new_backend);

        let mut lock = self.keyspaces.write().unwrap();
        lock.insert(name.to_string(), (main_table, history_table));

        // Add to db_registry system table
        let registry_key = format!("db:{}", name);
        self.sys_table.insert(
            registry_key.as_bytes(),
            db_path.to_string_lossy().as_bytes(),
        )?;

        Ok(())
    }

    pub fn update_db_path(&self, name: &str, new_path: &str) -> anyhow::Result<()> {
        if name == "default" {
            return Err(anyhow::anyhow!("Cannot update path of default database"));
        }

        // Only allow if database already actually exists in registry
        let registry_key = format!("db:{}", name);
        if let Ok(None) = self.sys_table.get(registry_key.as_bytes()) {
            return Err(anyhow::anyhow!(
                "Database does not exist or missing registry entry"
            ));
        }

        let db_path = PathBuf::from(new_path);

        // Ensure new backend can be initialized
        let new_backend = Arc::new(SqliteBackend::new(&db_path)?);

        let main_name = format!("{}_main", name);
        let history_name = format!("{}_history", name);

        Self::initialize_database_tables(new_backend.as_ref(), &main_name, &history_name)?;

        // Update registry
        self.sys_table.insert(
            registry_key.as_bytes(),
            db_path.to_string_lossy().as_bytes(),
        )?;

        let main_table = SqliteTable::new_main(main_name, new_backend.clone());
        let history_table = SqliteTable::new(history_name, new_backend.clone());

        self.backends.insert(name.to_string(), new_backend);

        let mut lock = self.keyspaces.write().unwrap();
        lock.insert(name.to_string(), (main_table, history_table));

        dbg_println!(
            "Storage: Updated path for database '{}' to {:?}",
            name,
            db_path
        );

        Ok(())
    }

    pub fn list_databases(&self) -> Vec<(String, String)> {
        let lock = self.keyspaces.read().unwrap();
        let mut result = Vec::with_capacity(lock.len());

        for name in lock.keys() {
            let path = if name == "default" {
                self.base_path
                    .join("default.db")
                    .to_string_lossy()
                    .to_string()
            } else {
                let registry_key = format!("db:{}", name);
                if let Ok(Some(bytes)) = self.sys_table.get(registry_key.as_bytes()) {
                    String::from_utf8(bytes).unwrap_or_else(|_| "Unknown".to_string())
                } else {
                    "Unknown".to_string()
                }
            };
            result.push((name.clone(), path));
        }

        result
    }

    pub fn get_database(&self, name: &str) -> Option<(SqliteTable, SqliteTable)> {
        let lock = self.keyspaces.read().unwrap();
        lock.get(name).cloned()
    }

    pub fn delete_database(&self, name: &str) -> anyhow::Result<()> {
        if name == "default" {
            return Err(anyhow::anyhow!("Cannot delete default database"));
        }

        let registry_key = format!("db:{}", name);
        let db_path: Option<String> = self
            .sys_table
            .get(registry_key.as_bytes())
            .ok()
            .flatten()
            .map(|bytes| String::from_utf8(bytes).unwrap_or_default());

        {
            let mut lock = self.keyspaces.write().unwrap();
            if lock.remove(name).is_none() {
                return Err(anyhow::anyhow!("Database not found"));
            }
        };

        if let Some((_, backend)) = self.backends.remove(name) {
            let main_name = format!("{}_main", name);
            let history_name = format!("{}_history", name);
            let _ = backend.drop_table(&main_name);
            let _ = backend.drop_table(&history_name);
        }

        let _ = self.sys_table.remove(registry_key.as_bytes());

        if let Some(path) = db_path {
            if !path.is_empty() {
                let db_file = std::path::PathBuf::from(&path);
                let _ = std::fs::remove_file(&db_file);
                let _ = std::fs::remove_file(db_file.with_extension("db-wal"));
                let _ = std::fs::remove_file(db_file.with_extension("db-shm"));
            }
        }

        let schema_path = self.base_path.join(format!("{}_schema.graphql", name));
        let _ = std::fs::remove_file(&schema_path);

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

        let backend = self
            .backends
            .get(db_name)
            .ok_or(anyhow::anyhow!("Database not found"))?
            .clone();
        let batch_start = Instant::now();
        backend.write_batch(|conn| {
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
        dbg_println!("Storage: Flush starting...");

        // Persist clock state
        {
            let clock = self.clock.lock().unwrap();
            let _ = self.sys_table.insert("clock", &clock.to_bytes());
        }

        // Persist fingerprints
        if let Err(e) = self.persist_fingerprints() {
            eprintln!("Storage: Failed to persist fingerprints: {}", e);
        }

        for entry in self.backends.iter() {
            if let Err(e) = entry.value().shutdown() {
                eprintln!(
                    "Storage: Failed to shutdown backend '{}': {}",
                    entry.key(),
                    e
                );
            }
        }

        dbg_println!("Storage: Flush complete (WAL checkpoint done)");
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
        if let Some(backend) = self.backends.get("default") {
            backend.with_writer(|conn| {
                conn.execute(
                    "DELETE FROM vec_data WHERE uid = ?1",
                    rusqlite::params![uid_i64],
                )?;
                Ok(())
            })?;
        }
        Ok(())
    }

    pub fn search_vectors(&self, query: &[f64], k: usize) -> anyhow::Result<Vec<(u64, f64)>> {
        let vec_f32: Vec<f32> = query.iter().map(|v| *v as f32).collect();
        let vec_bytes =
            unsafe { std::slice::from_raw_parts(vec_f32.as_ptr() as *const u8, vec_f32.len() * 4) };

        let backend = self
            .backends
            .get("default")
            .ok_or(anyhow::anyhow!("Missing default DB"))?;
        let conn = backend.get_reader()?;
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

        backend.return_reader(conn);
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
        dbg_println!("Storage: Rebuilding fingerprints...");
        let keyspaces = self.keyspaces.read().unwrap();

        for (name, (_, history)) in keyspaces.iter() {
            dbg_println!("Storage: Scanning history for '{}'...", name);
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
            dbg_println!("Storage: Rebuilt '{}' (Count: {}, Hash: {:x})", name, c, h);
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
        dbg_info!(
            fingerprint_count = self.fingerprints.len(),
            "Storage: persisting fingerprints to sys table"
        );
        for entry in self.fingerprints.iter() {
            let db_name = entry.key();
            let (h_atomic, c_atomic) = entry.value();
            let h = h_atomic.load(Ordering::Relaxed);
            let c = c_atomic.load(Ordering::Relaxed);

            let mut buf = [0u8; 16];
            BigEndian::write_u64(&mut buf[0..8], c);
            BigEndian::write_u64(&mut buf[8..16], h);

            let key = format!("fp:{}", db_name);
            dbg_info!(
                db_name = %db_name,
                fingerprint_key = %key,
                count = c,
                hash = format_args!("{:x}", h),
                "Storage: writing fingerprint record"
            );
            self.sys_table.insert(key.as_bytes(), &buf)?;
        }
        dbg_info!("Storage: fingerprint persistence complete");
        Ok(())
    }

    pub fn restore_fingerprints(&self) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let mut needs_rebuild: Vec<(String, SqliteTable)> = Vec::new();
        dbg_info!(
            keyspace_count = keyspaces.len(),
            "Storage: restoring fingerprints"
        );

        for (name, (_, history_table)) in keyspaces.iter() {
            let key = format!("fp:{}", name);
            dbg_info!(
                db_name = %name,
                fingerprint_key = %key,
                "Storage: checking fingerprint record"
            );
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
                    dbg_println!(
                        "Storage: Restored fingerprint for '{}' (Count: {}, Hash: {:x})",
                        name,
                        c,
                        h
                    );
                    dbg_info!(
                        db_name = %name,
                        count = c,
                        hash = format_args!("{:x}", h),
                        "Storage: restored fingerprint from sys table"
                    );
                    continue;
                }
                error!(
                    db_name = %name,
                    fingerprint_key = %key,
                    value_len = val.len(),
                    "Storage: fingerprint record had unexpected length"
                );
            }

            dbg_println!(
                "Storage: Fingerprint missing for '{}' - will rebuild in background",
                name
            );
            dbg_info!(
                db_name = %name,
                "Storage: fingerprint missing, scheduling background rebuild"
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
            dbg_println!("Storage: All fingerprints ready (restored from disk)");
            dbg_info!("Storage: all fingerprints restored from disk");
        } else {
            dbg_println!(
                "Storage: Fingerprints ready (initialized to zero, will rebuild in background)"
            );
            dbg_info!(
                rebuild_count = needs_rebuild.len(),
                "Storage: initialized placeholder fingerprints and spawning rebuild thread"
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
        let db_names: Vec<String> = db_list.iter().map(|(name, _)| name.clone()).collect();
        dbg_info!(
            rebuild_count = db_names.len(),
            databases = ?db_names,
            "Storage: spawning background fingerprint rebuild thread"
        );

        std::thread::spawn(move || {
            dbg_println!(
                "Storage: Background fingerprint rebuild started for {} database(s)",
                db_list.len()
            );
            let start = std::time::Instant::now();
            dbg_info!(
                rebuild_count = db_list.len(),
                "Storage: background fingerprint rebuild thread started"
            );

            for (name, history_table) in db_list {
                let scan_start = std::time::Instant::now();
                let mut hash: u64 = 0;
                let mut count: u64 = 0;
                dbg_info!(
                    db_name = %name,
                    "Storage: background fingerprint rebuild scanning database"
                );

                let rows = history_table.iter();
                dbg_info!(
                    db_name = %name,
                    row_count = rows.len(),
                    "Storage: background fingerprint rebuild loaded history rows"
                );

                for (k, v) in rows {
                    let next_count = count + 1;
                    if next_count <= 10 || next_count % 1_000 == 0 {
                        dbg_info!(
                            db_name = %name,
                            row_index = next_count,
                            key_len = k.len(),
                            value_len = v.len(),
                            "Storage: background fingerprint rebuild hashing row"
                        );
                    }
                    hash ^= Self::hash_item(&k, &v);
                    count = next_count;
                    if count <= 10 || count % 1_000 == 0 {
                        dbg_info!(
                            db_name = %name,
                            row_index = count,
                            running_hash = format_args!("{:x}", hash),
                            "Storage: background fingerprint rebuild hashed row"
                        );
                    }
                    if count % 100_000 == 0 {
                        dbg_println!(
                            "Storage: Fingerprint rebuild for '{}' - scanned {} items ({:.1}s)...",
                            name,
                            count,
                            scan_start.elapsed().as_secs_f64()
                        );
                    }
                }
                dbg_info!(
                    db_name = %name,
                    total_count = count,
                    final_hash = format_args!("{:x}", hash),
                    "Storage: background fingerprint rebuild finished hashing rows"
                );
                dbg_println!(
                    "Storage: Fingerprint rebuild for '{}' completed - {} items in {:.1}s",
                    name,
                    count,
                    scan_start.elapsed().as_secs_f64()
                );
                dbg_info!(
                    db_name = %name,
                    count,
                    hash = format_args!("{:x}", hash),
                    elapsed_secs = scan_start.elapsed().as_secs_f64(),
                    "Storage: background fingerprint rebuild scan complete"
                );

                // Update the DashMap entry
                use std::sync::atomic::Ordering;
                if let Some(entry) = fingerprints.get(&name) {
                    let (h_atomic, c_atomic) = entry.value();
                    h_atomic.store(hash, Ordering::Release);
                    c_atomic.store(count, Ordering::Release);
                    dbg_info!(
                        db_name = %name,
                        "Storage: updated in-memory fingerprint after rebuild"
                    );
                } else {
                    error!(
                        db_name = %name,
                        "Storage: missing in-memory fingerprint entry during rebuild"
                    );
                }

                // Persist to sys table
                let key = format!("fp:{}", name);
                let mut buf = vec![0u8; 16];
                BigEndian::write_u64(&mut buf[0..8], count);
                BigEndian::write_u64(&mut buf[8..16], hash);
                dbg_info!(
                    db_name = %name,
                    fingerprint_key = %key,
                    "Storage: persisting rebuilt fingerprint"
                );
                if let Err(e) = sys_table.insert(key.as_bytes(), &buf) {
                    eprintln!(
                        "Storage: Failed to persist rebuilt fingerprint for '{}': {}",
                        name, e
                    );
                }

                dbg_println!(
                    "Storage: Rebuilt fingerprint for '{}' (Count: {}, Hash: {:x})",
                    name,
                    count,
                    hash
                );
            }

            ready_flag.store(true, std::sync::atomic::Ordering::Release);
            dbg_println!(
                "Storage: Background fingerprint rebuild complete in {:?}",
                start.elapsed()
            );
            dbg_info!(
                elapsed_secs = start.elapsed().as_secs_f64(),
                "Storage: background fingerprint rebuild thread complete"
            );
        });
    }

    /// Wait for fingerprints to be ready. Used by SyncManager before starting gossip.
    pub fn wait_for_fingerprints(&self) {
        use std::sync::atomic::Ordering;

        if self.fingerprints_ready.load(Ordering::Acquire) {
            return;
        }

        dbg_println!("Storage: Waiting for fingerprints to be ready...");
        while !self.fingerprints_ready.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        dbg_println!("Storage: Fingerprints are now ready");
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
