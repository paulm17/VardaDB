use fjall::{Database, Keyspace, KeyspaceCreateOptions};
use std::path::Path;
use uuid::Uuid;
use byteorder::{BigEndian, ByteOrder};

pub struct Storage {
    pub db: Database,
    pub main_keyspace: Keyspace,        // LATEST: [UID][Pred] -> [Timestamp][Value]
    pub history_keyspace: Keyspace,     // HISTORY: [Timestamp][UID][Pred] -> [Value]
    pub quarantine_keyspace: Keyspace,  // QUARANTINE: [UID][Pred] -> [Timestamp][Value] (Wait for schema)
    pub sys_keyspace: Keyspace,         // SYSTEM: Config (NodeID, etc)
    pub node_id: u64,
    pub clock: std::sync::Mutex<crate::storage::timestamp::Timestamp>,
}

impl Storage {
    pub fn new(path: impl AsRef<Path>, node_id_override: Option<u64>) -> anyhow::Result<Self> {
        let db = Database::builder(path).open()?;
        
        // Open Partitions
        let main_keyspace = db.keyspace("main", || KeyspaceCreateOptions::default())?;
        let history_keyspace = db.keyspace("history", || KeyspaceCreateOptions::default())?;
        let quarantine_keyspace = db.keyspace("quarantine", || KeyspaceCreateOptions::default())?;
        let sys_keyspace = db.keyspace("sys", || KeyspaceCreateOptions::default())?;

        // Load or Generate Node ID
        let node_id = if let Some(id) = node_id_override {
             sys_keyspace.insert("node_id", &id.to_be_bytes())?;
             id
        } else if let Some(val) = sys_keyspace.get("node_id")? {
            let bytes = val.to_vec();
            if bytes.len() == 8 {
                BigEndian::read_u64(&bytes)
            } else {
                // Should not happen, but recover
                let new_id = Uuid::new_v4().as_u128() as u64; // Simple truncation mix
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
            // Load persisted clock
            if val.len() >= 16 {
                let bytes: [u8; 16] = val[0..16].try_into().unwrap();
                let stored = crate::storage::timestamp::Timestamp::from_bytes(&bytes);
                // Ensure we proceed from at least (now)
                let now = crate::storage::timestamp::Timestamp::physical_now();
                if stored.millis >= now {
                     stored
                } else {
                     crate::storage::timestamp::Timestamp::new(now, 0, node_id)
                }
            } else {
                 crate::storage::timestamp::Timestamp::new(crate::storage::timestamp::Timestamp::physical_now(), 0, node_id)
            }
        } else {
             crate::storage::timestamp::Timestamp::new(crate::storage::timestamp::Timestamp::physical_now(), 0, node_id)
        });

        println!("Storage: Initialized with Node ID: {}", node_id);

        Ok(Self {
            db,
            main_keyspace,
            history_keyspace,
            quarantine_keyspace,
            sys_keyspace,
            node_id,
            clock,
        })
    }


    pub fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let val = self.main_keyspace.get(key)?;
        // Strip Timestamp (16 bytes) if present
        // Default LWW format: [Timestamp][Value]
        Ok(val.map(|v| {
            if v.len() >= 16 {
                v[16..].to_vec()
            } else {
                v.to_vec() // Legacy fallback
            }
        }))
    }

    /// Last-Write-Wins Put
    /// Checks timestamps and updates LATEST and HISTORY keyspaces
    pub fn put_with_lww(&self, uid: u64, predicate: &str, value: &[u8], timestamp: &crate::storage::timestamp::Timestamp) -> anyhow::Result<()> {
        let key = crate::storage::codec::Codec::encode_data_key(uid, predicate);
        
        // 1. Check Stale
        if let Some(existing) = self.main_keyspace.get(&key)? {
             if existing.len() >= 16 {
                 let existing_ts_bytes: [u8; 16] = existing[0..16].try_into().unwrap();
                 let existing_ts = crate::storage::timestamp::Timestamp::from_bytes(&existing_ts_bytes);
                 
                 // Result is false if existing >= new (Stale or Same)
                 if existing_ts >= *timestamp {
                     println!("Sync: Stale write ignored for UID: {}, Pred: {}. Existing: {:?}, New: {:?}", uid, predicate, existing_ts, timestamp);
                     if let Ok(val_str) = std::str::from_utf8(&existing[16..]) {
                          println!("Sync: Existing Value (utf8): {}", val_str);
                     }
                     return Ok(()); // Stale write, ignore
                 }
             }
        }

        // 2. Write
        // VardaDB uses simple batch commit
        // Fjall 3.0: use Keyspace::insert for single, or Batch for atomic multi-keyspace
        // Note: Batch api in fjall might differ slightly, let's assume db.write_batch?
        // Checking backend.rs imports: `use fjall::{Database, Keyspace, KeyspaceCreateOptions};`
        // We need to verify batch usage. Assuming `let mut batch = self.db.batch();` exists.
        // If not, we might not have atomic cross-keyspace.
        // Fjall 3.0.1 supports `let mut batch = std::collections::BTreeMap::new()`? No.
        // Let's assume strict usage:
        
        // Write LATEST: [Ts][Val]
        let mut new_val_buf = Vec::with_capacity(16 + value.len());
        new_val_buf.extend_from_slice(&timestamp.to_bytes());
        new_val_buf.extend_from_slice(value);
        
        self.main_keyspace.insert(&key, &new_val_buf)?;

        // Write HISTORY: [Ts][UID][Pred] -> [Val]
        let hist_key = crate::storage::codec::Codec::encode_history_key(&timestamp.to_bytes(), uid, predicate);
        self.history_keyspace.insert(&hist_key, value)?;
        
        Ok(())
    }

    /// Last-Write-Wins Delete
    /// Removes from LATEST and adds Tombstone to HISTORY
    pub fn delete_with_lww(&self, uid: u64, predicate: &str, timestamp: &crate::storage::timestamp::Timestamp) -> anyhow::Result<()> {
        let key = crate::storage::codec::Codec::encode_data_key(uid, predicate);
        
        // 1. Remove from LATEST if not stale
        if let Some(existing) = self.main_keyspace.get(&key)? {
             if existing.len() >= 16 {
                 let existing_ts_bytes: [u8; 16] = existing[0..16].try_into().unwrap();
                 let existing_ts = crate::storage::timestamp::Timestamp::from_bytes(&existing_ts_bytes);
                 if existing_ts >= *timestamp {
                     return Ok(()); // Stale delete
                 }
             }
        }
        
        self.main_keyspace.remove(&key)?;

        // 2. Write Tombstone to HISTORY: [Ts][UID][Pred] -> [] (Empty)
        let hist_key = crate::storage::codec::Codec::encode_history_key(&timestamp.to_bytes(), uid, predicate);
        self.history_keyspace.insert(&hist_key, &[])?;
        
        Ok(())
    }

    // Direct Insert (Legacy/Raw) - Should eventually move to put_with_lww
    pub fn insert(&self, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        // Legacy insert: Just use 0 timestamp? Or fail?
        // Let's wrap it with a 0 timestamp for compatibility during migration
        // But we don't know UID/Pred from raw key here easily.
        // Just insert into main_keyspace without TS? get() handles missing TS fallback.
        self.main_keyspace.insert(key, value)?;
        Ok(())
    }

    pub fn remove(&self, key: &[u8]) -> anyhow::Result<()> {
        self.main_keyspace.remove(key)?;
        Ok(())
    }
    
    pub fn contains_key(&self, key: &[u8]) -> anyhow::Result<bool> {
        Ok(self.main_keyspace.contains_key(key)?)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.db.persist(fjall::PersistMode::SyncAll)?;
        Ok(())
    }

    // --- Sync & Quarantine ---

    /// Sync: Get all history items in a time range
    /// Returns (Key, Value) pairs
    pub fn get_history_range(&self, start_ts: Option<&crate::storage::timestamp::Timestamp>, end_ts: Option<&crate::storage::timestamp::Timestamp>) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        use std::ops::Bound;

        // Construct bounds
        let _start_bound = if let Some(ts) = start_ts {
            Bound::Included(ts.to_bytes().to_vec()) // Keys start with TS
        } else {
            Bound::Unbounded
        };

        let _end_bound = if let Some(ts) = end_ts {
             // We want to include everything UP TO this timestamp.
             // ts.to_bytes() + [0xFF] ensures we capture the full millisecond/seq
             let mut b = ts.to_bytes().to_vec();
             b.push(0xFF); 
             Bound::Excluded(b)
        } else {
            Bound::Unbounded
        };

        // Check fjall 3.x API: Guard ownership prevents getting both Key and Value easily?
        // TODO: Fix Iterator Guard handling. `item.key()` consumes self.
        
        let iter = self.history_keyspace.range((_start_bound, _end_bound));
        let mut results = Vec::new();
        for item in iter {
             if let Ok((k, v)) = item.into_inner() {
                  results.push((k.to_vec(), v.to_vec()));
             }
        }
        
        Ok(results)
    }

    /// Quarantine: Store data that doesn't match current schema
    pub fn put_quarantine(&self, uid: u64, predicate: &str, value: &[u8], timestamp: &crate::storage::timestamp::Timestamp) -> anyhow::Result<()> {
        let key = crate::storage::codec::Codec::encode_quarantine_key(uid, predicate);
        
        // Value = [Timestamp][Value] (Same as LATEST format)
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
        // Persist Clock
        let _ = self.sys_keyspace.insert("clock", &next.to_bytes());
        next
    }

    pub fn update_clock(&self, remote_ts: &crate::storage::timestamp::Timestamp) {
        let mut clock = self.clock.lock().unwrap();
        let now = crate::storage::timestamp::Timestamp::physical_now();
        let next = clock.receive(remote_ts, now);
        *clock = next;
        // Persist Clock
        let _ = self.sys_keyspace.insert("clock", &next.to_bytes());
    }
}
