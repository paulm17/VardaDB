use rusqlite::{Connection, params};
use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Low-level SQLite backend — manages connections and table lifecycle.
/// Replaces `fjall::Database`.
pub struct SqliteBackend {
    /// Single writer connection (WAL mode allows only one writer at a time anyway)
    writer: Mutex<Connection>,
    /// Pool of reader connections for concurrent GraphQL queries
    reader_pool: Mutex<Vec<Connection>>,
    /// Path to the database file (for creating new reader connections)
    path: PathBuf,
}

impl SqliteBackend {
    /// Open (or create) a SQLite database at the given path.
    /// Runs performance PRAGMAs on the connection.
    pub fn new(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let db_path = path.as_ref().join("varda.db");
        
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let writer = Connection::open(&db_path)?;
        Self::apply_pragmas(&writer)?;
        
        Ok(Self {
            writer: Mutex::new(writer),
            reader_pool: Mutex::new(Vec::new()),
            path: db_path,
        })
    }

    /// Apply performance PRAGMAs to a connection.
    fn apply_pragmas(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA mmap_size = 30000000000;
             PRAGMA cache_size = -2000;
             PRAGMA temp_store = MEMORY;
             PRAGMA busy_timeout = 5000;"
        )?;
        Ok(())
    }

    /// Create a standard KV table (key BLOB PRIMARY KEY, value BLOB).
    pub fn create_table(&self, name: &str) -> anyhow::Result<()> {
        let conn = self.writer.lock().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS \"{}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;",
            name
        ))?;
        Ok(())
    }

    /// Create a main data table with an extra `ts` column for LWW.
    pub fn create_main_table(&self, name: &str) -> anyhow::Result<()> {
        let conn = self.writer.lock().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS \"{}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL, ts BLOB NOT NULL) WITHOUT ROWID;",
            name
        ))?;
        Ok(())
    }

    /// Drop a table.
    pub fn drop_table(&self, name: &str) -> anyhow::Result<()> {
        let conn = self.writer.lock().unwrap();
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{}\";", name))?;
        Ok(())
    }

    /// List all table names from sqlite_master.
    pub fn list_tables(&self) -> Vec<String> {
        let conn = self.writer.lock().unwrap();
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.filter_map(|r| r.ok()).collect()
    }

    /// Get a reader connection from the pool (or create a new one).
    pub fn get_reader(&self) -> anyhow::Result<Connection> {
        {
            let mut pool = self.reader_pool.lock().unwrap();
            if let Some(conn) = pool.pop() {
                return Ok(conn);
            }
        }
        // Create new reader connection
        let conn = Connection::open(&self.path)?;
        Self::apply_pragmas(&conn)?;
        Ok(conn)
    }

    /// Return a reader connection to the pool.
    pub fn return_reader(&self, conn: Connection) {
        let mut pool = self.reader_pool.lock().unwrap();
        if pool.len() < 8 {
            pool.push(conn);
        }
        // Drop excess connections
    }

    /// Execute a write operation with the writer connection.
    /// The closure receives a reference to the locked writer connection.
    pub fn with_writer<F, R>(&self, f: F) -> anyhow::Result<R>
    where F: FnOnce(&Connection) -> anyhow::Result<R>
    {
        let conn = self.writer.lock().unwrap();
        f(&conn)
    }

    /// Execute a batch of writes in a single transaction.
    pub fn write_batch<F>(&self, f: F) -> anyhow::Result<()>
    where F: FnOnce(&Connection) -> anyhow::Result<()>
    {
        let conn = self.writer.lock().unwrap();
        conn.execute_batch("BEGIN")?;
        match f(&conn) {
            Ok(()) => {
                conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Checkpoint the WAL file and merge it into the main database.
    /// Call this on shutdown for a clean exit.
    pub fn shutdown(&self) -> anyhow::Result<()> {
        let conn = self.writer.lock().unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

impl Drop for SqliteBackend {
    fn drop(&mut self) {
        if let Ok(conn) = self.writer.lock() {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
    }
}

// ─────────────────── SqliteTable ───────────────────

use std::sync::Arc;

/// A handle to a named table in the SQLite database.
/// Logical equivalent of `fjall::Keyspace`.
/// Clone is cheap (just Arc + String).
#[derive(Clone)]
pub struct SqliteTable {
    pub name: String,
    backend: Arc<SqliteBackend>,
    /// If true, this table has a `ts` column (main tables).
    has_ts: bool,
}

impl SqliteTable {
    pub fn new(name: String, backend: Arc<SqliteBackend>) -> Self {
        Self { name, backend, has_ts: false }
    }

    /// Create a handle for a main table (has a `ts` column).
    pub fn new_main(name: String, backend: Arc<SqliteBackend>) -> Self {
        Self { name, backend, has_ts: true }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    // ── Reads (use reader pool) ──

    pub fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let conn = self.backend.get_reader()?;
        let sql = format!("SELECT value FROM \"{}\" WHERE key = ?1", self.name);
        let result = conn.prepare_cached(&sql)?
            .query_row(params![key], |row| row.get::<_, Vec<u8>>(0))
            .optional()?;
        self.backend.return_reader(conn);
        Ok(result)
    }

    pub fn contains_key(&self, key: &[u8]) -> anyhow::Result<bool> {
        let conn = self.backend.get_reader()?;
        let sql = format!("SELECT 1 FROM \"{}\" WHERE key = ?1 LIMIT 1", self.name);
        let exists = conn.prepare_cached(&sql)?
            .query_row(params![key], |_| Ok(()))
            .optional()?
            .is_some();
        self.backend.return_reader(conn);
        Ok(exists)
    }

    /// Prefix scan: returns all (key, value) pairs where key starts with `prefix`.
    /// Results are ordered by key.
    pub fn prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let upper = compute_prefix_upper_bound(prefix);
        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        let result = if let Some(ref upper) = upper {
            let sql = format!(
                "SELECT key, value FROM \"{}\" WHERE key >= ?1 AND key < ?2 ORDER BY key",
                self.name
            );
            match conn.prepare_cached(&sql) {
                Ok(mut stmt) => {
                    let rows = stmt.query_map(params![prefix, upper.as_slice()], |row| {
                        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
                    });
                    match rows {
                        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
                        Err(_) => vec![],
                    }
                }
                Err(_) => vec![],
            }
        } else {
            // Prefix is all 0xFF bytes — scan from prefix to end
            let sql = format!(
                "SELECT key, value FROM \"{}\" WHERE key >= ?1 ORDER BY key",
                self.name
            );
            match conn.prepare_cached(&sql) {
                Ok(mut stmt) => {
                    let rows = stmt.query_map(params![prefix], |row| {
                        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
                    });
                    match rows {
                        Ok(mapped) => mapped
                            .filter_map(|r| r.ok())
                            .take_while(|(k, _)| k.starts_with(prefix))
                            .collect(),
                        Err(_) => vec![],
                    }
                }
                Err(_) => vec![],
            }
        };
        self.backend.return_reader(conn);
        result
    }

    /// Range scan: returns all (key, value) pairs where `lower <= key < upper`.
    pub fn range(&self, lower: &[u8], upper: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        let sql = format!(
            "SELECT key, value FROM \"{}\" WHERE key >= ?1 AND key < ?2 ORDER BY key",
            self.name
        );
        let result = match conn.prepare_cached(&sql) {
            Ok(mut stmt) => {
                let rows = stmt.query_map(params![lower, upper], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
                });
                match rows {
                    Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
                    Err(_) => vec![],
                }
            }
            Err(_) => vec![],
        };
        self.backend.return_reader(conn);
        result
    }

    /// Iterate over all entries in the table, ordered by key.
    pub fn iter(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        let sql = format!("SELECT key, value FROM \"{}\" ORDER BY key", self.name);
        let result = match conn.prepare_cached(&sql) {
            Ok(mut stmt) => {
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
                });
                match rows {
                    Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
                    Err(_) => vec![],
                }
            }
            Err(_) => vec![],
        };
        self.backend.return_reader(conn);
        result
    }

    // ── Writes (use writer connection via backend) ──

    /// Insert or replace a key-value pair.
    /// For main tables (has_ts=true), auto-provides a zero timestamp for the ts column.
    pub fn insert(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> anyhow::Result<()> {
        self.backend.with_writer(|conn| {
            if self.has_ts {
                let default_ts = [0u8; 16];
                let sql = format!(
                    "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, ts = excluded.ts",
                    self.name
                );
                conn.prepare_cached(&sql)?.execute(params![key.as_ref(), value.as_ref(), &default_ts[..]])?;
            } else {
                let sql = format!(
                    "INSERT INTO \"{}\" (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    self.name
                );
                conn.prepare_cached(&sql)?.execute(params![key.as_ref(), value.as_ref()])?;
            }
            Ok(())
        })
    }

    /// Remove a key.
    pub fn remove(&self, key: impl AsRef<[u8]>) -> anyhow::Result<()> {
        self.backend.with_writer(|conn| {
            let sql = format!("DELETE FROM \"{}\" WHERE key = ?1", self.name);
            conn.prepare_cached(&sql)?.execute(params![key.as_ref()])?;
            Ok(())
        })
    }

    // ── LWW-specific operations (for _main tables with ts column) ──

    /// Get value + timestamp from a main table.
    pub fn get_with_ts(&self, key: &[u8]) -> anyhow::Result<Option<(Vec<u8>, Vec<u8>)>> {
        let conn = self.backend.get_reader()?;
        let sql = format!("SELECT value, ts FROM \"{}\" WHERE key = ?1", self.name);
        let result = conn.prepare_cached(&sql)?
            .query_row(params![key], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .optional()?;
        self.backend.return_reader(conn);
        Ok(result)
    }

    /// Atomic LWW upsert: only writes if the new timestamp is greater than the existing one.
    pub fn upsert_lww(&self, key: &[u8], value: &[u8], ts: &[u8]) -> anyhow::Result<()> {
        self.backend.with_writer(|conn| {
            let sql = format!(
                "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, ts = excluded.ts \
                 WHERE excluded.ts > \"{}\".ts",
                self.name, self.name
            );
            conn.prepare_cached(&sql)?.execute(params![key, value, ts])?;
            Ok(())
        })
    }

    /// Delete from a main table only if the given timestamp is newer.
    /// Returns true if the delete was applied (not stale).
    pub fn delete_lww(&self, key: &[u8], ts: &[u8]) -> anyhow::Result<bool> {
        // Need to check staleness first, then delete if not stale
        self.backend.with_writer(|conn| {
            // Check if existing entry is newer
            let check_sql = format!("SELECT ts FROM \"{}\" WHERE key = ?1", self.name);
            let existing_ts: Option<Vec<u8>> = conn.prepare_cached(&check_sql)?
                .query_row(params![key], |row| row.get::<_, Vec<u8>>(0))
                .optional()?;
            
            if let Some(existing) = existing_ts {
                if existing.as_slice() >= ts {
                    return Ok(false); // Stale
                }
            }
            
            let del_sql = format!("DELETE FROM \"{}\" WHERE key = ?1", self.name);
            conn.prepare_cached(&del_sql)?.execute(params![key])?;
            Ok(true)
        })
    }

    /// Batch insert within a transaction (for put_batch_lww).
    /// The caller must wrap this in a write_batch transaction.
    pub fn batch_insert_on_conn(&self, conn: &Connection, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        let sql = format!(
            "INSERT INTO \"{}\" (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            self.name
        );
        conn.prepare_cached(&sql)?.execute(params![key, value])?;
        Ok(())
    }

    /// Batch LWW upsert within a transaction for main tables.
    pub fn batch_upsert_lww_on_conn(&self, conn: &Connection, key: &[u8], value: &[u8], ts: &[u8]) -> anyhow::Result<()> {
        let sql = format!(
            "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, ts = excluded.ts \
             WHERE excluded.ts > \"{}\".ts",
            self.name, self.name
        );
        conn.prepare_cached(&sql)?.execute(params![key, value, ts])?;
        Ok(())
    }
}

// ─────────────────── KvStore Trait Implementations ───────────────────
// Bridge SqliteTable into the sub-crate abstract KvStore traits.

impl jobs::KvStore for SqliteTable {
    fn kv_insert(&self, key: &[u8], value: &[u8]) -> Result<(), String> {
        self.insert(key, value).map_err(|e| e.to_string())
    }

    fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.get(key.as_ref()).map_err(|e| e.to_string())
    }

    fn kv_remove(&self, key: &[u8]) -> Result<(), String> {
        self.remove(key).map_err(|e| e.to_string())
    }

    fn kv_prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.prefix(prefix)
    }
}

impl permissions::storage::auth_store::KvStore for SqliteTable {
    fn kv_insert(&self, key: &[u8], value: &[u8]) -> Result<(), String> {
        self.insert(key, value).map_err(|e| e.to_string())
    }

    fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.get(key.as_ref()).map_err(|e| e.to_string())
    }

    fn kv_remove(&self, key: &[u8]) -> Result<(), String> {
        self.remove(key).map_err(|e| e.to_string())
    }

    fn kv_prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.prefix(prefix)
    }
}


// ─────────────────── Helpers ───────────────────

/// Compute the exclusive upper bound for a prefix scan.
/// Increments the last byte; handles 0xFF overflow by truncating.
/// Returns None if the prefix is all 0xFF bytes (scan to end of table).
pub fn compute_prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut upper = prefix.to_vec();
    while let Some(last) = upper.last_mut() {
        if *last < 0xFF {
            *last += 1;
            return Some(upper);
        } else {
            upper.pop();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_prefix_upper_bound() {
        assert_eq!(compute_prefix_upper_bound(&[0x01, 0x02]), Some(vec![0x01, 0x03]));
        assert_eq!(compute_prefix_upper_bound(&[0x01, 0xFF]), Some(vec![0x02]));
        assert_eq!(compute_prefix_upper_bound(&[0xFF, 0xFF]), None);
        assert_eq!(compute_prefix_upper_bound(&[0x00]), Some(vec![0x01]));
    }

    #[test]
    fn test_basic_kv_operations() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(SqliteBackend::new(dir.path()).unwrap());
        backend.create_table("test").unwrap();
        let table = SqliteTable::new("test".to_string(), backend.clone());

        // Insert
        table.insert(b"key1", b"value1").unwrap();
        table.insert(b"key2", b"value2").unwrap();

        // Get
        assert_eq!(table.get(b"key1").unwrap(), Some(b"value1".to_vec()));
        assert_eq!(table.get(b"key2").unwrap(), Some(b"value2".to_vec()));
        assert_eq!(table.get(b"key3").unwrap(), None);

        // Contains
        assert!(table.contains_key(b"key1").unwrap());
        assert!(!table.contains_key(b"key3").unwrap());

        // Remove
        table.remove(b"key1").unwrap();
        assert_eq!(table.get(b"key1").unwrap(), None);

        // Update
        table.insert(b"key2", b"updated").unwrap();
        assert_eq!(table.get(b"key2").unwrap(), Some(b"updated".to_vec()));
    }

    #[test]
    fn test_prefix_scan() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(SqliteBackend::new(dir.path()).unwrap());
        backend.create_table("test").unwrap();
        let table = SqliteTable::new("test".to_string(), backend.clone());

        table.insert(b"\x01\x00\x00\x01", b"a").unwrap();
        table.insert(b"\x01\x00\x00\x02", b"b").unwrap();
        table.insert(b"\x01\x00\x01\x01", b"c").unwrap();
        table.insert(b"\x02\x00\x00\x01", b"d").unwrap();

        let results = table.prefix(b"\x01\x00\x00");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, b"a".to_vec());
        assert_eq!(results[1].1, b"b".to_vec());
    }

    #[test]
    fn test_lww_upsert() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(SqliteBackend::new(dir.path()).unwrap());
        backend.create_main_table("main").unwrap();
        let table = SqliteTable::new("main".to_string(), backend.clone());

        let key = b"test_key";
        let ts1 = [0u8; 16]; // Timestamp 0
        let mut ts2 = [0u8; 16];
        ts2[15] = 1; // Timestamp 1 (greater)

        // First write
        table.upsert_lww(key, b"first", &ts1).unwrap();
        let (val, ts) = table.get_with_ts(key).unwrap().unwrap();
        assert_eq!(val, b"first".to_vec());
        assert_eq!(ts, ts1.to_vec());

        // Newer write — should succeed
        table.upsert_lww(key, b"second", &ts2).unwrap();
        let (val, _) = table.get_with_ts(key).unwrap().unwrap();
        assert_eq!(val, b"second".to_vec());

        // Stale write — should be ignored
        table.upsert_lww(key, b"stale", &ts1).unwrap();
        let (val, _) = table.get_with_ts(key).unwrap().unwrap();
        assert_eq!(val, b"second".to_vec()); // Still "second"
    }

    #[test]
    fn test_write_batch() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(SqliteBackend::new(dir.path()).unwrap());
        backend.create_table("test").unwrap();
        let table = SqliteTable::new("test".to_string(), backend.clone());

        backend.write_batch(|conn| {
            table.batch_insert_on_conn(conn, b"k1", b"v1")?;
            table.batch_insert_on_conn(conn, b"k2", b"v2")?;
            table.batch_insert_on_conn(conn, b"k3", b"v3")?;
            Ok(())
        }).unwrap();

        assert_eq!(table.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(table.get(b"k2").unwrap(), Some(b"v2".to_vec()));
        assert_eq!(table.get(b"k3").unwrap(), Some(b"v3".to_vec()));
    }

    #[test]
    fn test_iter() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(SqliteBackend::new(dir.path()).unwrap());
        backend.create_table("test").unwrap();
        let table = SqliteTable::new("test".to_string(), backend.clone());

        table.insert(b"c", b"3").unwrap();
        table.insert(b"a", b"1").unwrap();
        table.insert(b"b", b"2").unwrap();

        let all = table.iter();
        assert_eq!(all.len(), 3);
        // Should be sorted by key
        assert_eq!(all[0].0, b"a".to_vec());
        assert_eq!(all[1].0, b"b".to_vec());
        assert_eq!(all[2].0, b"c".to_vec());
    }
}

