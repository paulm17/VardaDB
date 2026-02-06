use fjall::{Database, Keyspace, KeyspaceCreateOptions};
use std::path::Path;
use uuid::Uuid;
use byteorder::{BigEndian, ByteOrder};
use jobs::{JobStore, Queue};
use std::sync::Arc;

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
    pub vector_store: crate::storage::vector::store::VectorStore, // VECTORS (Global for now, or need multi-vector store)
    
    pub jobs_store: Arc<JobStore>,      // JOB STORE (Global)
    pub system_queue: Arc<Queue>,       // DEFAULT QUEUE (Global)
    pub node_id: u64,
    pub clock: std::sync::Mutex<crate::storage::timestamp::Timestamp>,
}

impl Storage {
    pub fn new(path: impl AsRef<Path>, node_id_override: Option<u64>) -> anyhow::Result<Self> {
        let db = Database::builder(path).open()?;
        
        // Open System Partition
        let sys_keyspace = db.keyspace("sys", || KeyspaceCreateOptions::default())?;
        let quarantine_keyspace = db.keyspace("quarantine", || KeyspaceCreateOptions::default())?;
        
        // Vectors (Global Index for now - TODO: Split per DB)
        let vectors_keyspace = db.keyspace("vectors", || KeyspaceCreateOptions::default())?;
        let vector_store = crate::storage::vector::store::VectorStore::new(
            vectors_keyspace, 
            crate::storage::vector::config::HNSWConfig::default()
        );

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
        
        println!("Storage: Initialized with Node ID: {}", node_id);

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

        Ok(Self {
            db,
            keyspaces: std::sync::RwLock::new(initial_keyspaces),
            sys_keyspace,
            quarantine_keyspace,
            vector_store,
            jobs_store,
            system_queue,
            node_id,
            clock,
        })
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
        
        Ok(())
    }

    // Direct Insert (Legacy/Raw)
    pub fn insert(&self, db_name: &str, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        let keyspaces = self.keyspaces.read().unwrap();
        let (main, _) = keyspaces.get(db_name).ok_or(anyhow::anyhow!("Database not found"))?;
        main.insert(key, value)?;
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
        self.db.persist(fjall::PersistMode::SyncAll)?;
        Ok(())
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
        let _ = self.sys_keyspace.insert("clock", &next.to_bytes());
        next
    }

    pub fn update_clock(&self, remote_ts: &crate::storage::timestamp::Timestamp) {
        let mut clock = self.clock.lock().unwrap();
        let now = crate::storage::timestamp::Timestamp::physical_now();
        let next = clock.receive(remote_ts, now);
        *clock = next;
        let _ = self.sys_keyspace.insert("clock", &next.to_bytes());
    }

    // --- Vector Operations ---

    pub fn put_vector(&self, uid: u64, vector: Vec<f64>) -> anyhow::Result<()> {
        self.vector_store.insert(uid as u128, vector)?;
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
}
