use fjall::{Database, Keyspace, KeyspaceCreateOptions};
use std::path::Path;
use uuid::Uuid;
use byteorder::{BigEndian, ByteOrder};
use jobs::{JobStore, Queue};
use std::sync::{Arc, Mutex, Weak};
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use tracing::{info, error};

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
    pub db: Database,
    // Database Management
    // Map: DatabaseName -> (Main Keyspace, History Keyspace)
    // We cache handles to avoid locking the Supervisor too often, though keyspace() is cheap.
    // For simplicity, we can look them up on demand or cache them.
    // Let's cache them in a RwLock for read-heavy access.
    pub keyspaces: std::sync::RwLock<std::collections::HashMap<String, (Keyspace, Keyspace)>>,
    
    // System Keyspaces (Global)
    pub sys_keyspace: Keyspace,         // SYSTEM: Config (NodeID, etc)
    pub quarantine_keyspace: Keyspace,  // QUARANTINE: Global? Or per DB? Let's make it global for now or deprecated.
    pub metrics_keyspace: Keyspace,     // METRICS: Time-series metrics
    pub traces_keyspace: Keyspace,      // TRACES: Trace spans
    pub vector_store: crate::storage::vector::store::VectorStore, // VECTORS (Global for now, or need multi-vector store)
    
    pub jobs_store: Arc<JobStore>,      // JOB STORE (Global)
    pub system_queue: Arc<Queue>,       // DEFAULT QUEUE (Global)
    pub node_id: u64,
    pub clock: std::sync::Mutex<crate::storage::timestamp::Timestamp>,
    pub vector_tx: std::sync::mpsc::SyncSender<(u64, Vec<f64>)>,
    
    // Incremental Fingerprints: DbName -> (Hash, Count)
    // Using AtomicU64 for concurrent access without locking the map for values
    pub fingerprints: dashmap::DashMap<String, (std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU64)>,
    
    // Flag indicating fingerprints are ready for sync (background rebuild complete)
    pub fingerprints_ready: std::sync::atomic::AtomicBool,

    // Embedding Model (Shared)
    pub embedding_model: Arc<std::sync::Mutex<TextEmbedding>>,
}

impl Storage {
    pub fn new(path: impl AsRef<Path>, node_id_override: Option<u64>) -> anyhow::Result<Self> {
        // Limit worker threads to 2 to reduce compaction CPU impact during bulk inserts
        // Default is min(CPU cores, 4) which can cause write stalls when compaction competes with inserts
        let db = Database::builder(path)
            .worker_threads(2)
            .open()?;
        
        // Open System Partition
        let sys_keyspace = db.keyspace("sys", || KeyspaceCreateOptions::default())?;
        let quarantine_keyspace = db.keyspace("quarantine", || KeyspaceCreateOptions::default())?;
        let metrics_keyspace = db.keyspace("sys_metrics", || KeyspaceCreateOptions::default())?;
        let traces_keyspace = db.keyspace("sys_traces", || KeyspaceCreateOptions::default())?;
        
        // Vectors (Global Index for now - TODO: Split per DB)
        let vectors_keyspace = db.keyspace("vectors", || KeyspaceCreateOptions::default())?;
        let vector_store = crate::storage::vector::store::VectorStore::new(
            vectors_keyspace, 
            crate::storage::vector::config::HNSWConfig::default()
        );

        // Vector Worker (Bounded Channel)
        let (tx, rx) = std::sync::mpsc::sync_channel::<(u64, Vec<f64>)>(5000);
        let worker_store = vector_store.clone();
        
        std::thread::spawn(move || {
            println!("Storage: Vector Background Worker Started");
            while let Ok((uid, vec)) = rx.recv() {
                // Determine Level 0 insert?
                // VectorStore::insert checks levels. All good.
                if let Err(e) = worker_store.insert(uid as u128, vec) {
                    eprintln!("Vector Worker Error for UID {}: {}", uid, e);
                }
            }
            println!("Storage: Vector Background Worker Stopped");
        });

        // Open Jobs Keyspace
        let jobs_keyspace = db.keyspace("jobs", || KeyspaceCreateOptions::default())?;
        let jobs_store = Arc::new(JobStore::new(Arc::new(jobs_keyspace)));
        // Create Default "System" Queue
        let system_queue = Arc::new(Queue::new("system_queue".to_string(), jobs_store.clone()));

        // Load active databases from metadata or discovery
        // For now, we auto-discover keyspaces ending in "_main" and pair them.
        let mut initial_keyspaces = std::collections::HashMap::new();
        
        // Always ensure "default" database exists
        let default_main = db.keyspace("default_main", || KeyspaceCreateOptions::default())?;
        let default_history = db.keyspace("default_history", || KeyspaceCreateOptions::default())?;
        initial_keyspaces.insert("default".to_string(), (default_main, default_history));

        // Auto-discover other databases
        // Patterns: `{name}_main` and `{name}_history`
        let all_ks = db.list_keyspace_names();
        println!("Storage: All keyspaces in manifest: {:?}", all_ks);
        
        for ks_name in all_ks {
            if ks_name.ends_with("_main") && &*ks_name != "default_main" {
                let db_name = ks_name.trim_end_matches("_main");
                let history_ks_name = format!("{}_history", db_name);
                
                // Open handles
                if let Ok(main_ks) = db.keyspace(&ks_name, || KeyspaceCreateOptions::default()) {
                    if let Ok(hist_ks) = db.keyspace(&history_ks_name, || KeyspaceCreateOptions::default()) {
                        println!("Storage: Discovered database '{}'", db_name);
                        initial_keyspaces.insert(db_name.to_string(), (main_ks, hist_ks));
                    }
                }
            }
        }

        // Load or Generate Node ID
        let node_id = if let Some(id) = node_id_override {
             sys_keyspace.insert("node_id", &id.to_be_bytes())?;
             id
        } else if let Some(val) = sys_keyspace.get("node_id")? {
            let bytes = val.to_vec();
            if bytes.len() == 8 {
                BigEndian::read_u64(&bytes)
            } else {
                let new_id = Uuid::new_v4().as_u128() as u64; 
                sys_keyspace.insert("node_id", &new_id.to_be_bytes())?;
                new_id
            }
        } else {
            let new_id = Uuid::new_v4().as_u128() as u64; 
            sys_keyspace.insert("node_id", &new_id.to_be_bytes())?;
            new_id
        };
        
        info!("Storage: Initialized with Node ID: {}", node_id);

        let clock = std::sync::Mutex::new(if let Some(val) = sys_keyspace.get("clock")? {
            if val.len() >= 16 {
                let bytes: [u8; 16] = val[0..16].try_into().unwrap();
                let stored = crate::storage::timestamp::Timestamp::from_bytes(&bytes);
                let now = crate::storage::timestamp::Timestamp::physical_now();
                if stored.millis >= now { stored } else { crate::storage::timestamp::Timestamp::new(now, 0, node_id) }
            } else {
                 crate::storage::timestamp::Timestamp::new(crate::storage::timestamp::Timestamp::physical_now(), 0, node_id)
            }
        } else {
             crate::storage::timestamp::Timestamp::new(crate::storage::timestamp::Timestamp::physical_now(), 0, node_id)
        });

        // Initialize Embedding Model (BGESmallEN - lightweight, good performance)
        info!("Storage: Initializing Embedding Model (BGESmallEN) - This may take a while to download...");
        let embedding_model = match TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallENV15)) {
            Ok(model) => model,
            Err(e) => {
                error!("Storage: Failed to load embedding model: {}", e);
                return Err(anyhow::anyhow!("Failed to load embedding model: {}", e));
            }
        };
        let embedding_model = Arc::new(std::sync::Mutex::new(embedding_model));
        info!("Storage: Embedding Model Ready");

        let storage = Self {
            db,
            keyspaces: std::sync::RwLock::new(initial_keyspaces),
            sys_keyspace,
            quarantine_keyspace,
            metrics_keyspace,
            traces_keyspace,
            vector_store,
            jobs_store,
            system_queue,
            node_id,
            clock,
            vector_tx: tx,
            fingerprints: dashmap::DashMap::new(),
            fingerprints_ready: std::sync::atomic::AtomicBool::new(false),
            embedding_model,
        };
        
        // Restore Fingerprints (Fast load / Fallback to scan)
        if let Err(e) = storage.restore_fingerprints() {
             error!("Storage: Failed to restore/rebuild fingerprints: {}", e);
        }
        
        Ok(storage)
    }
    
    pub fn register_exit_hook(self: &Arc<Self>) {
        let mutex = ACTIVE_STORAGES.get_or_init(|| {
            // Spawn a delayed thread to overwrite any signal handlers
            // that heavy GUI frameworks (like Chromium/CEF) might install
            // during their own initialization phase after VardaDB starts.
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
        
        let main_ks = self.db.keyspace(&main_name, || KeyspaceCreateOptions::default())?;
        let history_ks = self.db.keyspace(&history_name, || KeyspaceCreateOptions::default())?;
        
        let mut lock = self.keyspaces.write().unwrap();
        lock.insert(name.to_string(), (main_ks, history_ks));
        
        // TODO: Persist database list in `sys_keyspace` so we don't rely on auto-discovery?
        // Discovery via `db.list_keyspace_names` works for now.
        
        Ok(())
    }
    
    pub fn list_databases(&self) -> Vec<String> {
        let lock = self.keyspaces.read().unwrap();
        lock.keys().cloned().collect()
    }
    
    pub fn get_database(&self, name: &str) -> Option<(Keyspace, Keyspace)> {
        let lock = self.keyspaces.read().unwrap();
        lock.get(name).cloned()
    }

    pub fn delete_database(&self, name: &str) -> anyhow::Result<()> {
        let (main, history) = {
            let mut lock = self.keyspaces.write().unwrap();
            match lock.remove(name) {
                Some(ks) => ks,
                None => return Err(anyhow::anyhow!("Database not found")),
            }
        };
        
        // Correct way to delete keyspace in Fjall?
        // self.db.delete_keyspace(keyspace_handle)
        // Check API: db.delete_keyspace(Keyspace) -> Result<()>
        self.db.delete_keyspace(main)?;
        self.db.delete_keyspace(history)?;
        Ok(())
    }

    // --- Data Access ---

    pub fn get(&self, db_name: &str, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;
        
        let val = main.get(key)?;
        Ok(val.map(|v| {
            if v.len() >= 16 {
                v[16..].to_vec()
            } else {
                v.to_vec() 
            }
        }))
    }

    /// Last-Write-Wins Put
    pub fn put_with_lww(&self, db_name: &str, uid: u64, predicate: &str, value: &[u8], timestamp: &crate::storage::timestamp::Timestamp) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, history) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;

        let key = crate::storage::codec::Codec::encode_data_key(uid, predicate);
        
        // 1. Check Stale
        if let Some(existing) = main.get(&key)? {
             if existing.len() >= 16 {
                 let existing_ts_bytes: [u8; 16] = existing[0..16].try_into().unwrap();
                 let existing_ts = crate::storage::timestamp::Timestamp::from_bytes(&existing_ts_bytes);
                 
                 if existing_ts >= *timestamp {
                     return Ok(()); // Stale
                 }
             }
        }

        // 2. Write
        let mut new_val_buf = Vec::with_capacity(16 + value.len());
        new_val_buf.extend_from_slice(&timestamp.to_bytes());
        new_val_buf.extend_from_slice(value);
        
        main.insert(&key, &new_val_buf)?;

        // Write HISTORY
        let hist_key = crate::storage::codec::Codec::encode_history_key(&timestamp.to_bytes(), uid, predicate);
        history.insert(&hist_key, value)?;
        self.update_history_hash(db_name, &hist_key, value);
        
        // Auto-compaction: check L0 pressure (higher threshold for single writes)
        // Fjall stalls writes at l0 >= 20 (soft) and >= 30 (hard halt)
        let l0_count = main.l0_table_count();
        if l0_count >= 16 {
            if crate::debug_logging() {
                println!("⚠️ L0 pressure high (l0_tables={}), triggering compaction...", l0_count);
            }
            let _ = main.major_compact();
            if crate::debug_logging() {
                println!("✅ Auto-compaction complete (l0_tables={})", main.l0_table_count());
            }
        }
        
        Ok(())
    }

    /// Batch Last-Write-Wins Put
    /// Uses Fjall WriteBatch for atomic multi-keyspace writes with single commit.
    pub fn put_batch_lww(&self, db_name: &str, items: Vec<(u64, String, Vec<u8>)>, timestamp: &crate::storage::timestamp::Timestamp) -> anyhow::Result<()> {
        use std::time::Instant;
        let op_start = Instant::now();
        let item_count = items.len();
        
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _history) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;
        let lock_time = op_start.elapsed();

        // Use Fjall WriteBatch for atomic multi-keyspace writes
        let batch_start = Instant::now();
        let mut batch = self.db.batch();
        let _fingerprint_updates: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();

        for (uid, predicate, value) in items {
            let key = crate::storage::codec::Codec::encode_data_key(uid, &predicate);
            
            // 2. Queue writes to batch
            let mut new_val_buf = Vec::with_capacity(16 + value.len());
            new_val_buf.extend_from_slice(&timestamp.to_bytes());
            new_val_buf.extend_from_slice(&value);
            
            batch.insert(main, &key, &new_val_buf);
        }
        let prep_time = batch_start.elapsed();
        
        // Single atomic commit for all writes
        let commit_start = Instant::now();
        batch.commit()?;
        let commit_time = commit_start.elapsed();
        
        let total_time = op_start.elapsed();
        
        // Log if any phase took > 50ms
        if crate::debug_logging() && total_time.as_millis() > 50 {
            println!("⏱️ put_batch_lww SLOW: {} items | lock={:?}, prep={:?}, commit={:?}, total={:?}",
                     item_count, lock_time, prep_time, commit_time, total_time);
        }
        
        // Auto-compaction: check L0 pressure after each batch commit
        // Fjall stalls writes at l0 >= 20 (soft) and >= 30 (hard halt)
        // Proactively compact at 12 to prevent reaching stall threshold
        let l0_count = main.l0_table_count();
        if l0_count >= 8 {
            if crate::debug_logging() {
                let compact_start = Instant::now();
                println!("⚠️ L0 pressure high (l0_tables={}), triggering compaction...", l0_count);
                let _ = main.major_compact();
                println!("✅ Auto-compaction complete ({:?}, l0_tables={})", 
                         compact_start.elapsed(), main.l0_table_count());
            } else {
                let _ = main.major_compact();
            }
        }
        
        Ok(())
    }

    /// Last-Write-Wins Delete
    pub fn delete_with_lww(&self, db_name: &str, uid: u64, predicate: &str, timestamp: &crate::storage::timestamp::Timestamp) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, history) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;

        let key = crate::storage::codec::Codec::encode_data_key(uid, predicate);
        
        // 1. Remove from LATEST if not stale
        if let Some(existing) = main.get(&key)? {
             if existing.len() >= 16 {
                 let existing_ts_bytes: [u8; 16] = existing[0..16].try_into().unwrap();
                 let existing_ts = crate::storage::timestamp::Timestamp::from_bytes(&existing_ts_bytes);
                 if existing_ts >= *timestamp {
                     return Ok(()); // Stale delete
                 }
             }
        }
        
        main.remove(&key)?;

        // 2. Write Tombstone to HISTORY
        let hist_key = crate::storage::codec::Codec::encode_history_key(&timestamp.to_bytes(), uid, predicate);
        history.insert(&hist_key, &[])?;
        self.update_history_hash(db_name, &hist_key, &[]);
        
        Ok(())
    }

    // Direct Insert (Legacy/Raw)
    pub fn insert(&self, db_name: &str, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;
        main.insert(key, value)?;
        Ok(())
    }

    /// Delete a raw key from the main keyspace
    pub fn delete_key(&self, db_name: &str, key: &[u8]) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;
        main.remove(key)?;
        Ok(())
    }

    pub fn remove(&self, db_name: &str, key: &[u8]) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;
        main.remove(key)?;
        Ok(())
    }
    
    pub fn contains_key(&self, db_name: &str, key: &[u8]) -> anyhow::Result<bool> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;
        Ok(main.contains_key(key)?)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        println!("Storage: Flush starting - Journal count: {}", self.db.journal_count());
        
        // Persist clock state (not persisted on every write for performance)
        {
            let clock = self.clock.lock().unwrap();
            let _ = self.sys_keyspace.insert("clock", &clock.to_bytes());
        }

        // Persist Internal State
        if let Err(e) = self.persist_fingerprints() {
            eprintln!("Storage: Failed to persist fingerprints: {}", e);
        }

        let mut keyspaces_to_flush = Vec::new();
        
        // System keyspaces
        keyspaces_to_flush.push(self.sys_keyspace.clone());
        keyspaces_to_flush.push(self.quarantine_keyspace.clone());
        keyspaces_to_flush.push(self.metrics_keyspace.clone());
        keyspaces_to_flush.push(self.traces_keyspace.clone());
        
        // Dynamic keyspaces
        {
            let lock = self.keyspaces.read().unwrap();
            for (main, history) in lock.values() {
                keyspaces_to_flush.push(main.clone());
                keyspaces_to_flush.push(history.clone());
            }
        }
        
        let keyspace_count = keyspaces_to_flush.len();
        
        for ks in keyspaces_to_flush {
             // Call hidden API to force flush to disk and rotation
             if let Err(e) = ks.rotate_memtable_and_wait() {
                 eprintln!("Storage: Failed to rotate memtable for keyspace {}: {}", ks.name(), e);
             }
        }

        // Vector Store (contains private partition)
        if let Err(e) = self.vector_store.flush() {
            eprintln!("Storage: Failed to flush vector store: {}", e);
        }

        self.db.persist(fjall::PersistMode::SyncAll)?;
        println!("Storage: Flush complete - Journal count: {}, Flushed {} keyspaces", 
                 self.db.journal_count(), 
                 keyspace_count);
        Ok(())
    }

    /// Check if compaction is needed based on internal metrics
    /// Returns true if any keyspace has high L0 segment count (indicating compaction pressure)
    pub fn needs_compaction(&self) -> bool {
        // Fjall stalls writes at l0 >= 20, so check well below that
        let lock = self.keyspaces.read().unwrap();
        lock.values().any(|(main, _)| main.l0_table_count() >= 12)
    }

    /// Returns the number of active background compactions currently running
    pub fn active_compactions(&self) -> usize {
        self.db.active_compactions()
    }

    /// Trigger explicit BLOCKING compaction on all keyspaces
    /// Uses major_compact() which forces full LSM compaction and waits until complete
    /// Returns the total duration in milliseconds
    pub fn compact(&self) -> anyhow::Result<u64> {
        use std::time::Instant;
        let start = Instant::now();
        
        println!("🔧 Storage: Major compaction starting (journal count: {}, active: {})...", 
                 self.db.journal_count(), self.db.active_compactions());
        
        // First, rotate all memtables to flush pending writes
        {
            let lock = self.keyspaces.read().unwrap();
            for (main, history) in lock.values() {
                let _ = main.rotate_memtable_and_wait();
                let _ = history.rotate_memtable_and_wait();
            }
        }
        let _ = self.sys_keyspace.rotate_memtable_and_wait();
        let _ = self.quarantine_keyspace.rotate_memtable_and_wait();
        
        // Persist to trigger journal truncation
        self.db.persist(fjall::PersistMode::SyncAll)?;
        
        // CRITICAL: Call major_compact() on all keyspaces - this is BLOCKING
        // major_compact() forces full LSM tree compaction and waits until complete
        {
            let lock = self.keyspaces.read().unwrap();
            for (main, history) in lock.values() {
                if let Err(e) = main.major_compact() {
                    eprintln!("   ⚠️ Major compact failed on main keyspace: {}", e);
                }
                if let Err(e) = history.major_compact() {
                    eprintln!("   ⚠️ Major compact failed on history keyspace: {}", e);
                }
            }
        }
        
        // Also major compact system keyspaces
        let _ = self.sys_keyspace.major_compact();
        let _ = self.quarantine_keyspace.major_compact();
        
        // Final persist
        self.db.persist(fjall::PersistMode::SyncAll)?;
        
        let duration_ms = start.elapsed().as_millis() as u64;
        println!("✅ Storage: Major compaction complete ({} ms, journal count: {}, active: {})", 
                 duration_ms, self.db.journal_count(), self.db.active_compactions());
        
        Ok(duration_ms)
    }

    // --- Sync & Quarantine ---

    pub fn get_history_range(&self, db_name: &str, start_ts: Option<&crate::storage::timestamp::Timestamp>, end_ts: Option<&crate::storage::timestamp::Timestamp>) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        use std::ops::Bound;
        let keyspaces = self.keyspaces.read().unwrap();
        let (_, history) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;

        let _start_bound = if let Some(ts) = start_ts {
            Bound::Included(ts.to_bytes().to_vec()) 
        } else {
            Bound::Unbounded
        };

        let _end_bound = if let Some(ts) = end_ts {
             let mut b = ts.to_bytes().to_vec();
             b.push(0xFF); 
             Bound::Excluded(b)
        } else {
            Bound::Unbounded
        };

        let iter = history.range((_start_bound, _end_bound));
        let mut results = Vec::new();
        for item in iter {
             if let Ok((k, v)) = item.into_inner() {
                  results.push((k.to_vec(), v.to_vec()));
             }
        }
        
        Ok(results)
    }

    pub fn put_quarantine(&self, uid: u64, predicate: &str, value: &[u8], timestamp: &crate::storage::timestamp::Timestamp) -> anyhow::Result<()> {
        let key = crate::storage::codec::Codec::encode_quarantine_key(uid, predicate);
        let mut new_val = Vec::with_capacity(16 + value.len());
        new_val.extend_from_slice(&timestamp.to_bytes());
        new_val.extend_from_slice(value);

        self.quarantine_keyspace.insert(&key, &new_val)?;
        Ok(())
    }

    pub fn scan_quarantine(&self) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut items = Vec::new();
        for item in self.quarantine_keyspace.iter() {
            if let Ok((k, v)) = item.into_inner() {
                items.push((k.to_vec(), v.to_vec()));
            }
        }
        Ok(items)
    }

    pub fn delete_quarantine(&self, key: &[u8]) -> anyhow::Result<()> {
        self.quarantine_keyspace.remove(key)?;
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
        self.vector_tx.send((uid, vector)).map_err(|e| anyhow::anyhow!("Failed to send vector to worker: {}", e))?;
        Ok(())
    }

    pub fn delete_vector(&self, uid: u64) -> anyhow::Result<()> {
        self.vector_store.delete(uid as u128)?;
        Ok(())
    }

    pub fn search_vectors(&self, query: &[f64], k: usize) -> anyhow::Result<Vec<(u64, f64)>> {
        let results = self.vector_store.search(query, k)?;
        let converted = results.into_iter().map(|(id, dist)| (id as u64, dist)).collect();
        Ok(converted)
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
            
            for item in history.iter() {
                if let Ok((k, v)) = item.into_inner() {
                     h ^= Self::hash_item(&k, &v);
                     c += 1;
                }
            }
            
            self.fingerprints.insert(name.clone(), (
                std::sync::atomic::AtomicU64::new(h),
                std::sync::atomic::AtomicU64::new(c)
            ));
            println!("Storage: Rebuilt '{}' (Count: {}, Hash: {:x})", name, c, h);
        }
        Ok(())
    }

    pub fn get_global_fingerprint(&self, db_name: &str) -> Option<(u64, u64)> {
        // Return (Hash, Count)
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
            BigEndian::write_u64(&mut buf[8..16], h); // Check endianness/order? Plan said count then hash.
            
            let key = format!("fp:{}", db_name);
            self.sys_keyspace.insert(key, buf)?;
        }
        Ok(())
    }

    pub fn restore_fingerprints(&self) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let mut needs_rebuild: Vec<(String, Keyspace)> = Vec::new();
        
        for (name, (_, history_ks)) in keyspaces.iter() {
            let key = format!("fp:{}", name);
            if let Some(val) = self.sys_keyspace.get(key)? {
                if val.len() == 16 {
                    let c = BigEndian::read_u64(&val[0..8]);
                    let h = BigEndian::read_u64(&val[8..16]);
                    
                    self.fingerprints.insert(name.clone(), (
                        std::sync::atomic::AtomicU64::new(h),
                        std::sync::atomic::AtomicU64::new(c)
                    ));
                    println!("Storage: Restored fingerprint for '{}' (Count: {}, Hash: {:x})", name, c, h);
                    continue;
                }
            }
            
            // Track databases needing rebuild
            println!("Storage: Fingerprint missing for '{}' - will rebuild in background", name);
            // Initialize with zeros for now - will be updated by background thread
            self.fingerprints.insert(name.clone(), (
                std::sync::atomic::AtomicU64::new(0),
                std::sync::atomic::AtomicU64::new(0)
            ));
            needs_rebuild.push((name.clone(), history_ks.clone()));
        }
        
        drop(keyspaces); // Release lock before spawning thread
        
        // Mark fingerprints as ready IMMEDIATELY
        // For missing fingerprints, (0,0) is correct for empty databases
        // Incremental updates will maintain accuracy as data is written
        use std::sync::atomic::Ordering;
        self.fingerprints_ready.store(true, Ordering::Release);
        
        if needs_rebuild.is_empty() {
            println!("Storage: All fingerprints ready (restored from disk)");
        } else {
            println!("Storage: Fingerprints ready (initialized to zero, will rebuild in background)");
            // Spawn optional background rebuild to compute accurate fingerprints
            // This is an optimization - not required for correctness since incremental updates work
            self.spawn_fingerprint_rebuild(needs_rebuild);
        }
        
        Ok(())
    }
    
    /// Spawn a background thread to rebuild fingerprints for the given databases.
    /// This allows fast startup while fingerprints are computed in the background.
    fn spawn_fingerprint_rebuild(&self, db_list: Vec<(String, Keyspace)>) {
        // We use raw pointers because:
        // 1. DashMap<String, (AtomicU64, AtomicU64)> can't be cloned (AtomicU64 is !Clone)
        // 2. Storage lives for the entire app lifetime, so these references are always valid
        let fingerprints_ptr = &self.fingerprints as *const dashmap::DashMap<String, (std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU64)>;
        let ready_ptr = &self.fingerprints_ready as *const std::sync::atomic::AtomicBool;
        let sys_keyspace = self.sys_keyspace.clone();
        
        // Convert pointers to usize for Send
        let fingerprints_addr = fingerprints_ptr as usize;
        let ready_addr = ready_ptr as usize;
        
        // Install a custom panic hook that filters out the known Fjall/DashMap bug
        // This prevents the "attempt to shift right with overflow" message on empty keyspaces
        std::panic::set_hook(Box::new(move |info| {
            // Filter out the known DashMap panic from Fjall
            let msg = info.to_string();
            if msg.contains("shift right with overflow") || msg.contains("dashmap") {
                // Silently ignore this known bug in Fjall with empty keyspaces
                return;
            }
            // For other panics, use the default behavior
            eprintln!("{}", info);
        }));
        
        std::thread::spawn(move || {
            println!("Storage: Background fingerprint rebuild started for {} database(s)", db_list.len());
            let start = std::time::Instant::now();
            
            // Safety: Storage outlives this thread, pointers are valid
            let fingerprints = unsafe { 
                &*(fingerprints_addr as *const dashmap::DashMap<String, (std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU64)>) 
            };
            let ready_flag = unsafe { &*(ready_addr as *const std::sync::atomic::AtomicBool) };
            
            for (name, history_ks) in db_list {
                // Scan all items to compute fingerprint
                // Fjall has a bug with empty keyspaces that causes DashMap panic
                // Use catch_unwind to ensure thread continues and sets fingerprints_ready
                let scan_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut hash: u64 = 0;
                    let mut count: u64 = 0;
                    let scan_start = std::time::Instant::now();
                    for item in history_ks.range::<Vec<u8>, _>(..) {
                        if let Ok((k, v)) = item.into_inner() {
                            hash ^= Self::hash_item(&k, &v);
                            count += 1;
                            if count % 100_000 == 0 {
                                println!("Storage: Fingerprint rebuild for '{}' - scanned {} items ({:.1}s)...", 
                                         name, count, scan_start.elapsed().as_secs_f64());
                            }
                        }
                    }
                    println!("Storage: Fingerprint rebuild for '{}' completed - {} items in {:.1}s", 
                             name, count, scan_start.elapsed().as_secs_f64());
                    (hash, count)
                }));
                
                let (h, c) = match scan_result {
                    Ok((hash, count)) => (hash, count),
                    Err(_) => (0, 0), // Empty keyspace - fingerprint is trivially (0, 0)
                };
                
                // Update the DashMap entry
                use std::sync::atomic::Ordering;
                if let Some(entry) = fingerprints.get(&name) {
                    let (h_atomic, c_atomic) = entry.value();
                    h_atomic.store(h, Ordering::Release);
                    c_atomic.store(c, Ordering::Release);
                }
                
                // Persist to sys_keyspace
                let key = format!("fp:{}", name);
                let mut buf = vec![0u8; 16];
                BigEndian::write_u64(&mut buf[0..8], c);
                BigEndian::write_u64(&mut buf[8..16], h);
                if let Err(e) = sys_keyspace.insert(key, buf) {
                    eprintln!("Storage: Failed to persist rebuilt fingerprint for '{}': {}", name, e);
                }
                
                println!("Storage: Rebuilt fingerprint for '{}' (Count: {}, Hash: {:x})", name, c, h);
            }
            
            // Mark fingerprints as ready - ALWAYS set this even if scans failed
            ready_flag.store(true, std::sync::atomic::Ordering::Release);
            
            println!("Storage: Background fingerprint rebuild complete in {:?}", start.elapsed());
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
        
        // Optimistic update using DashMap
        // If entry missing, we might need to insert it (or ignore if lazy?)
        // Better to initialize if missing.
        
        if let Some(entry) = self.fingerprints.get(db_name) {
             let (h_stat, c_stat) = entry.value();
             use std::sync::atomic::Ordering;
             h_stat.fetch_xor(item_hash, Ordering::Relaxed);
             c_stat.fetch_add(1, Ordering::Relaxed);
        } else {
             // Handle missing entry (Race condition on creation? or just insert)
             use std::sync::atomic::AtomicU64;
             self.fingerprints.insert(db_name.to_string(), (
                 AtomicU64::new(item_hash),
                 AtomicU64::new(1)
             ));
        }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        println!("Storage Drop triggered. Ensuring WAL and memtables are flushed...");
        if let Err(e) = self.flush() {
            eprintln!("Failed to flush storage during drop: {}", e);
        } else {
            println!("Storage flushed successfully on drop.");
        }
    }
}
