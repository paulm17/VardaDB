use byteorder::ByteOrder;
use lru::LruCache;
use rusqlite::ffi::{sqlite3_auto_extension, sqlite3_reset_auto_extension};
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tracing::{error, info};

/// Default dimensionality of the `vec_data` embedding column.
pub const DEFAULT_VECTOR_DIMS: usize = 384;

/// Process-wide override for the vector dimensionality (set from `VardaConfig`).
static CONFIGURED_VECTOR_DIMS: AtomicUsize = AtomicUsize::new(0);

/// Set the vector dimensionality used when creating new `vec_data` tables.
/// Called by `init_system` from `[search] vector_dims` in `VardaConfig`.
pub fn set_configured_vector_dims(dims: usize) {
    CONFIGURED_VECTOR_DIMS.store(dims.max(1), Ordering::SeqCst);
}

/// The explicitly configured dims, if any (config value takes priority over env).
pub fn configured_vector_dims() -> Option<usize> {
    match CONFIGURED_VECTOR_DIMS.load(Ordering::SeqCst) {
        0 => None,
        n => Some(n),
    }
}

/// Effective dims for newly created `vec_data` tables:
/// config override > `VARDADB_VECTOR_DIMS` env var > [`DEFAULT_VECTOR_DIMS`].
pub fn effective_vector_dims() -> usize {
    configured_vector_dims()
        .or_else(|| {
            std::env::var("VARDADB_VECTOR_DIMS")
                .ok()
                .and_then(|s| s.trim().parse::<usize>().ok())
                .filter(|d| *d > 0)
        })
        .unwrap_or(DEFAULT_VECTOR_DIMS)
}

/// Extract the embedding dimensionality from a `CREATE VIRTUAL TABLE ... USING vec0(...)`
/// DDL string by locating the `float[N]` column type.
fn parse_vec_dims_from_ddl(ddl: &str) -> Option<usize> {
    let marker = "float[";
    let start = ddl.to_ascii_lowercase().find(marker)? + marker.len();
    let rest = &ddl[start..];
    let end = rest.find(']')?;
    rest[..end].trim().parse::<usize>().ok().filter(|d| *d > 0)
}

macro_rules! dbg_info {
    ($($arg:tt)*) => {
        if crate::debug_logging() {
            info!($($arg)*);
        }
    };
}

/// Low-level SQLite backend — manages connections and table lifecycle.
/// Replaces `fjall::Database`.
pub struct SqliteBackend {
    /// Single writer connection (WAL mode allows only one writer at a time anyway)
    writer: Mutex<Connection>,
    /// Pool of reader connections for concurrent GraphQL queries
    reader_pool: Mutex<Vec<Connection>>,
    /// Path to the database file (for creating new reader connections)
    path: PathBuf,
    /// Effective dimensionality of this backend's `vec_data` table
    /// (0 until `create_native_search_tables` has run).
    vector_dims: AtomicUsize,
}

impl SqliteBackend {
    /// Open (or create) a SQLite database at the exact given path.
    /// Runs performance PRAGMAs on the connection.
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        dbg_info!(db_path = %db_path.display(), "SqliteBackend: opening database");

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
            dbg_info!(parent = %parent.display(), "SqliteBackend: ensured parent directory exists");
        }

        // Register sqlite-vec extension exactly once (sqlite3_auto_extension is
        // cumulative — calling it N times registers the init function N times,
        // causing each new connection to run it N times → SIGTRAP with ≥3 DBs).
        static SQLITE_VEC_INIT: std::sync::Once = std::sync::Once::new();
        SQLITE_VEC_INIT.call_once(|| unsafe {
            sqlite3_reset_auto_extension();
            sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        });

        let writer = Connection::open(&db_path)?;
        dbg_info!(db_path = %db_path.display(), "SqliteBackend: writer connection opened");
        Self::apply_pragmas(&writer)?;
        dbg_info!(db_path = %db_path.display(), "SqliteBackend: pragmas applied to writer connection");

        Ok(Self {
            writer: Mutex::new(writer),
            reader_pool: Mutex::new(Vec::new()),
            path: db_path,
            vector_dims: AtomicUsize::new(0),
        })
    }

    /// Apply performance PRAGMAs to a connection.
    fn apply_pragmas(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA mmap_size = 30000000000;
             PRAGMA cache_size = -65536;
             PRAGMA temp_store = MEMORY;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(())
    }

    /// Create a standard KV table (key BLOB PRIMARY KEY, value BLOB).
    pub fn create_table(&self, name: &str) -> anyhow::Result<()> {
        dbg_info!(db_path = %self.path.display(), table = %name, "SqliteBackend: creating table if needed");
        let conn = self.writer.lock().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS \"{}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;",
            name
        ))?;
        dbg_info!(db_path = %self.path.display(), table = %name, "SqliteBackend: create_table complete");
        Ok(())
    }

    /// Create Full-Text Search and Vector tables for native search.
    ///
    /// The `vec_data` dimensionality comes from the process-wide configuration
    /// (`set_configured_vector_dims` / `VARDADB_VECTOR_DIMS`), defaulting to
    /// [`DEFAULT_VECTOR_DIMS`]. If a `vec_data` table already exists (created with
    /// different dims), the existing schema wins — its dims are introspected and
    /// reported via [`SqliteBackend::vector_dims`] so writers validate against it.
    pub fn create_native_search_tables(&self) -> anyhow::Result<()> {
        dbg_info!(db_path = %self.path.display(), "SqliteBackend: creating native search tables if needed");
        let conn = self.writer.lock().unwrap();
        // FTS tables are created unconditionally (idempotent); only vec_data
        // creation is gated on the dims probe so an existing table's schema
        // always wins.
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts_data USING fts5(uid UNINDEXED, field UNINDEXED, text_content, tokenize='porter unicode61');
             CREATE VIRTUAL TABLE IF NOT EXISTS fts_term_data USING fts5(uid UNINDEXED, field UNINDEXED, text_content, tokenize='unicode61');
             CREATE VIRTUAL TABLE IF NOT EXISTS fts_trigram_data USING fts5(uid UNINDEXED, field UNINDEXED, text_content, tokenize='trigram');",
        )?;
        let effective_dims = if let Some(existing) = Self::existing_vec_dims(&conn)? {
            existing
        } else {
            let dims = effective_vector_dims();
            conn.execute_batch(&format!(
                "CREATE VIRTUAL TABLE IF NOT EXISTS vec_data USING vec0(uid INTEGER PRIMARY KEY, embedding float[{dims}]);"
            ))?;
            dims
        };
        self.vector_dims
            .store(effective_dims, Ordering::Release);
        dbg_info!(
            db_path = %self.path.display(),
            vector_dims = effective_dims,
            "SqliteBackend: native search table setup complete"
        );
        Ok(())
    }

    /// Introspect the dimensionality of an already-existing `vec_data` table.
    fn existing_vec_dims(conn: &Connection) -> anyhow::Result<Option<usize>> {
        let ddl: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'vec_data'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(ddl.and_then(|s| parse_vec_dims_from_ddl(&s)))
    }

    /// Effective dimensionality of this backend's `vec_data` table.
    /// Returns [`DEFAULT_VECTOR_DIMS`] before `create_native_search_tables` has run.
    pub fn vector_dims(&self) -> usize {
        match self.vector_dims.load(Ordering::Acquire) {
            0 => effective_vector_dims(),
            n => n,
        }
    }

    /// Create a main data table with an extra `ts` column for LWW.
    pub fn create_main_table(&self, name: &str) -> anyhow::Result<()> {
        dbg_info!(db_path = %self.path.display(), table = %name, "SqliteBackend: creating main table if needed");
        let conn = self.writer.lock().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS \"{}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL, ts BLOB NOT NULL) WITHOUT ROWID;",
            name
        ))?;
        dbg_info!(db_path = %self.path.display(), table = %name, "SqliteBackend: create_main_table complete");
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
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.filter_map(|r| r.ok()).collect()
    }

    /// Get a reader connection from the pool (or create a new one).
    pub fn get_reader(&self) -> anyhow::Result<Connection> {
        let start = std::time::Instant::now();
        {
            let mut pool = self.reader_pool.lock().unwrap();
            if let Some(conn) = pool.pop() {
                let elapsed = start.elapsed();
                if elapsed.as_millis() > 5 && crate::debug_logging() {
                    eprintln!(
                        "[STORAGE] get_reader (pool hit) path={} elapsed_ms={}",
                        self.path.display(),
                        elapsed.as_millis()
                    );
                }
                return Ok(conn);
            }
        }
        // Create new reader connection. Auto-extension is already registered globally by `new`
        // but it doesn't hurt to ensure it or just let the connection open.
        // Actually sqlite3_auto_extension applies to all subsequent db connections.
        let conn = Connection::open(&self.path)?;
        Self::apply_pragmas(&conn)?;
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 5 && crate::debug_logging() {
            eprintln!(
                "[STORAGE] get_reader (new conn) path={} elapsed_ms={}",
                self.path.display(),
                elapsed.as_millis()
            );
        }
        Ok(conn)
    }

    /// Return a reader connection to the pool.
    pub fn return_reader(&self, conn: Connection) {
        let mut pool = self.reader_pool.lock().unwrap();
        if pool.len() < 8 {
            pool.push(conn);
        } else {
            dbg_info!(
                db_path = %self.path.display(),
                pool_size = pool.len(),
                "SqliteBackend: dropping excess reader connection"
            );
        }
        // Drop excess connections
    }

    /// Execute a write operation with the writer connection.
    /// The closure receives a reference to the locked writer connection.
    pub fn with_writer<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&Connection) -> anyhow::Result<R>,
    {
        dbg_info!(db_path = %self.path.display(), "SqliteBackend: acquiring writer lock");
        let conn = self.writer.lock().unwrap();
        dbg_info!(db_path = %self.path.display(), "SqliteBackend: writer lock acquired");
        let result = f(&conn);
        match &result {
            Ok(_) => {
                dbg_info!(db_path = %self.path.display(), "SqliteBackend: writer operation complete")
            }
            Err(e) => {
                error!(db_path = %self.path.display(), error = %e, "SqliteBackend: writer operation failed")
            }
        }
        result
    }

    /// Execute a batch of writes in a single transaction.
    pub fn write_batch<F>(&self, f: F) -> anyhow::Result<()>
    where
        F: FnOnce(&Connection) -> anyhow::Result<()>,
    {
        dbg_info!(db_path = %self.path.display(), "SqliteBackend: starting write batch");
        let conn = self.writer.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        f(&tx)?;
        tx.commit()?;
        dbg_info!(db_path = %self.path.display(), "SqliteBackend: write batch committed");
        Ok(())
    }

    /// Checkpoint the WAL file and merge it into the main database.
    /// Call this on shutdown for a clean exit.
    pub fn shutdown(&self) -> anyhow::Result<()> {
        dbg_info!(db_path = %self.path.display(), "SqliteBackend: running WAL checkpoint for shutdown");
        let conn = self.writer.lock().unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        dbg_info!(db_path = %self.path.display(), "SqliteBackend: shutdown checkpoint complete");
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

/// Cache key for filter_by_field_value results.
#[derive(Clone, Hash, Eq, PartialEq)]
struct FilterCacheKey {
    type_name: String,
    field_name: String,
    op: String,
    target: String,
}

impl FilterCacheKey {
    fn new(type_name: &str, field_name: &str, op: &str, target: &rusqlite::types::Value) -> Self {
        Self {
            type_name: type_name.to_string(),
            field_name: field_name.to_string(),
            op: op.to_string(),
            target: format!("{:?}", target),
        }
    }
}

/// A handle to a named table in the SQLite database.
/// Logical equivalent of `fjall::Keyspace`.
/// Clone is cheap (just Arc + String).
#[derive(Clone)]
pub struct SqliteTable {
    pub name: String,
    backend: Arc<SqliteBackend>,
    /// If true, this table has a `ts` column (main tables).
    has_ts: bool,
    insert_sql: String,
    upsert_lww_sql: String,
    /// LRU cache for filter_by_field_value results (1024 entries max)
    filter_cache: Arc<Mutex<LruCache<FilterCacheKey, Vec<u64>>>>,
}

impl SqliteTable {
    pub fn new(name: String, backend: Arc<SqliteBackend>) -> Self {
        let insert_sql = format!(
        "INSERT INTO \"{}\" (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        name
    );
        let upsert_lww_sql = String::new(); // History tables don't use upsert_lww
        Self {
            name,
            backend,
            has_ts: false,
            insert_sql,
            upsert_lww_sql,
            filter_cache: Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(1024).unwrap()))),
        }
    }

    /// Create a handle for a main table (has a `ts` column).
    pub fn new_main(name: String, backend: Arc<SqliteBackend>) -> Self {
        let insert_sql = format!(
        "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, ts = excluded.ts",
        name
    );
        let upsert_lww_sql = format!(
            "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, ts = excluded.ts \
         WHERE excluded.ts > \"{}\".ts",
            name, name
        );
        Self {
            name,
            backend,
            has_ts: true,
            insert_sql,
            upsert_lww_sql,
            filter_cache: Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(1024).unwrap()))),
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }

    // ── Reads (use reader pool) ──

    pub fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let conn = self.backend.get_reader()?;
        let sql = format!("SELECT value FROM \"{}\" WHERE key = ?1", self.name);
        let result = conn.prepare_cached(&sql).and_then(|mut stmt| {
            stmt.query_row(params![key], |row| row.get::<_, Vec<u8>>(0))
                .optional()
        });
        self.backend.return_reader(conn);
        if let Err(e) = &result {
            error!(table = %self.name, error = %e, "SqliteTable: get failed");
        }
        Ok(result?)
    }

    pub fn contains_key(&self, key: &[u8]) -> anyhow::Result<bool> {
        let conn = self.backend.get_reader()?;
        let sql = format!("SELECT 1 FROM \"{}\" WHERE key = ?1 LIMIT 1", self.name);
        let exists = conn
            .prepare_cached(&sql)
            .and_then(|mut stmt| stmt.query_row(params![key], |_| Ok(())).optional());
        self.backend.return_reader(conn);
        Ok(exists?.is_some())
    }

    /// Prefix scan: returns all (key, value) pairs where key starts with `prefix`.
    /// Results are ordered by key.
    pub fn prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let start = std::time::Instant::now();
        let upper = compute_prefix_upper_bound(prefix);
        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(e) => {
                error!(table = %self.name, error = %e, "SqliteTable: prefix scan failed to get reader");
                return vec![];
            }
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
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 10 && crate::debug_logging() {
            eprintln!(
                "[STORAGE] prefix table={} prefix_len={} result_count={} elapsed_ms={}",
                self.name,
                prefix.len(),
                result.len(),
                elapsed.as_millis()
            );
        }
        result
    }

    /// Range scan: returns all (key, value) pairs where `lower <= key < upper`.
    pub fn range(&self, lower: &[u8], upper: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let start = std::time::Instant::now();
        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(e) => {
                error!(table = %self.name, error = %e, "SqliteTable: range scan failed to get reader");
                return vec![];
            }
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
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 10 && crate::debug_logging() {
            eprintln!(
                "[STORAGE] range table={} lower_len={} upper_len={} result_count={} elapsed_ms={}",
                self.name,
                lower.len(),
                upper.len(),
                result.len(),
                elapsed.as_millis()
            );
        }
        result
    }

    pub fn count_prefix(&self, prefix: &[u8]) -> anyhow::Result<usize> {
        let upper = compute_prefix_upper_bound(prefix);
        let conn = self.backend.get_reader()?;

        let result = if let Some(ref upper) = upper {
            let sql = format!(
                "SELECT COUNT(*) FROM \"{}\" WHERE key >= ?1 AND key < ?2",
                self.name
            );
            conn.prepare_cached(&sql).and_then(|mut stmt| {
                stmt.query_row(params![prefix, upper.as_slice()], |row| {
                    row.get::<_, i64>(0)
                })
            })
        } else {
            let sql = format!("SELECT COUNT(*) FROM \"{}\" WHERE key >= ?1", self.name);
            conn.prepare_cached(&sql)
                .and_then(|mut stmt| stmt.query_row(params![prefix], |row| row.get::<_, i64>(0)))
        };

        self.backend.return_reader(conn);
        result
            .map(|count| count.max(0) as usize)
            .map_err(anyhow::Error::from)
    }

    /// Iterate over all entries in the table, ordered by key.
    pub fn iter(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let start = std::time::Instant::now();
        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(e) => {
                error!(table = %self.name, error = %e, "SqliteTable: full scan failed to get reader");
                return vec![];
            }
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
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 10 && crate::debug_logging() {
            eprintln!(
                "[STORAGE] iter table={} result_count={} elapsed_ms={}",
                self.name,
                result.len(),
                elapsed.as_millis()
            );
        }
        result
    }

    // ── Writes (use writer connection via backend) ──

    /// Insert or replace a key-value pair.
    /// For main tables (has_ts=true), auto-provides a zero timestamp for the ts column.
    pub fn insert(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> anyhow::Result<()> {
        self.backend.with_writer(|conn| {
            if self.has_ts {
                let default_ts = [0u8; 16];
                conn.prepare_cached(&self.insert_sql)?.execute(params![
                    key.as_ref(),
                    value.as_ref(),
                    &default_ts[..]
                ])?;
            } else {
                conn.prepare_cached(&self.insert_sql)?
                    .execute(params![key.as_ref(), value.as_ref()])?;
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
        let result = conn.prepare_cached(&sql).and_then(|mut stmt| {
            stmt.query_row(params![key], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .optional()
        });
        self.backend.return_reader(conn);
        Ok(result?)
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
            conn.prepare_cached(&sql)?
                .execute(params![key, value, ts])?;
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
            let existing_ts: Option<Vec<u8>> = conn
                .prepare_cached(&check_sql)?
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
    pub fn batch_insert_on_conn(
        &self,
        conn: &Connection,
        key: &[u8],
        value: &[u8],
    ) -> anyhow::Result<()> {
        if self.has_ts {
            let default_ts = [0u8; 16];
            conn.prepare_cached(&self.insert_sql)?
                .execute(params![key, value, &default_ts[..]])?;
        } else {
            conn.prepare_cached(&self.insert_sql)?
                .execute(params![key, value])?;
        }
        Ok(())
    }

    /// Batch LWW upsert within a transaction for main tables.
    pub fn batch_upsert_lww_on_conn(
        &self,
        conn: &Connection,
        key: &[u8],
        value: &[u8],
        ts: &[u8],
    ) -> anyhow::Result<()> {
        conn.prepare_cached(&self.upsert_lww_sql)?
            .execute(params![key, value, ts])?;
        Ok(())
    }

    /// Encode f64 as 8 sortable bytes (same algorithm used when writing the 0x09 index).
    fn encode_sortable_f64(value: f64) -> [u8; 8] {
        let bits = value.to_bits();
        let sortable = if bits & (1 << 63) != 0 {
            !bits
        } else {
            bits ^ (1 << 63)
        };
        sortable.to_be_bytes()
    }

    /// Convert a rusqlite Value into the byte encoding used in the 0x09 order index.
    /// Returns None for Null and Blob (which are not indexed).
    fn sqlite_value_to_order_index_bytes(val: &rusqlite::types::Value) -> Option<Vec<u8>> {
        match val {
            rusqlite::types::Value::Text(s) => Some(s.as_bytes().to_vec()),
            rusqlite::types::Value::Integer(i) => {
                Some(Self::encode_sortable_f64(*i as f64).to_vec())
            }
            rusqlite::types::Value::Real(f) => Some(Self::encode_sortable_f64(*f).to_vec()),
            _ => None,
        }
    }

    /// Equality lookup using the 0x09 ascending order index.
    ///
    /// Executes a prefix scan on:
    ///   [0x09][type][0x00][field][0x00][0x00][enc_v][0x00]
    ///
    /// Every key with this prefix encodes exactly one UID in its final 8 bytes.
    /// Time complexity: O(log N + K) where K = number of matching rows.
    fn filter_via_order_index_eq(
        &self,
        type_name: &str,
        field_name: &str,
        enc_v: &[u8],
    ) -> Vec<u64> {
        use crate::storage::codec::Codec;

        let mut prefix = Codec::encode_order_index_prefix(type_name, field_name, false);
        prefix.extend_from_slice(enc_v);
        prefix.push(0x00);

        self.prefix(&prefix)
            .into_iter()
            .filter_map(|(k, _)| Codec::decode_order_index_uid(&k))
            .filter(|uid| *uid != 0)
            .collect()
    }

    /// Range scan using the 0x09 ascending order index.
    ///
    /// All ascending entries for (type, field) live in the key range:
    ///   [ encode_order_index_prefix(type, field, false),
    ///     encode_order_index_prefix(type, field, true) )
    ///
    /// For a boundary value enc_v the "boundary prefix" is:
    ///   [0x09][type][0x00][field][0x00][0x00][enc_v][0x00]
    ///
    /// Operator mapping:
    ///   >=  lower = boundary,                    upper = desc_prefix
    ///   >   lower = upper_bound(boundary),       upper = desc_prefix
    ///   <=  lower = asc_prefix,                  upper = upper_bound(boundary)
    ///   <   lower = asc_prefix,                  upper = boundary
    ///
    /// `less_than`: true for < / <=, false for > / >=
    /// `inclusive`: true for <= / >=, false for < / >
    fn filter_via_order_index_range(
        &self,
        type_name: &str,
        field_name: &str,
        enc_v: &[u8],
        less_than: bool,
        inclusive: bool,
    ) -> Vec<u64> {
        use crate::storage::codec::Codec;

        let asc_prefix = Codec::encode_order_index_prefix(type_name, field_name, false);
        let desc_prefix = Codec::encode_order_index_prefix(type_name, field_name, true);

        let mut boundary = asc_prefix.clone();
        boundary.extend_from_slice(enc_v);
        boundary.push(0x00);

        let (lower, upper) = if less_than {
            let lower = asc_prefix;
            let upper = if inclusive {
                compute_prefix_upper_bound(&boundary).unwrap_or(desc_prefix)
            } else {
                boundary
            };
            (lower, upper)
        } else {
            let lower = if inclusive {
                boundary
            } else {
                compute_prefix_upper_bound(&boundary).unwrap_or(desc_prefix.clone())
            };
            let upper = desc_prefix;
            (lower, upper)
        };

        self.range(&lower, &upper)
            .into_iter()
            .filter_map(|(k, _)| Codec::decode_order_index_uid(&k))
            .filter(|uid| *uid != 0)
            .collect()
    }

    /// Filter pushdown: scan the main table for keys matching a specific field name
    /// and apply a comparison operator on the JSON value stored in the blob.
    ///
    /// Key encoding: [0x01][UID:8 bytes big-endian][field_name bytes]
    /// Value encoding: [timestamp:16 bytes][json_payload bytes]
    ///
    /// The SQL query:
    /// 1. Scans all keys with prefix [0x01] that end with the field name suffix
    /// 2. Extracts the JSON payload by skipping the 16-byte timestamp prefix
    /// 3. Applies the comparison operator in SQL via json_extract
    /// 4. Returns the UID bytes (key[1..9]) for matching rows
    ///
    /// `op` must be one of: "=", "!=", ">", "<", ">=", "<=", "LIKE"
    /// `target` is a properly typed SQLite value matching what json_extract returns
    pub fn filter_by_field_value(
        &self,
        type_name: &str,
        field_name: &str,
        op: &str,
        target: rusqlite::types::Value,
    ) -> Vec<u64> {
        let start = std::time::Instant::now();

        let cache_key = FilterCacheKey::new(type_name, field_name, op, &target);
        {
            let mut cache = self.filter_cache.lock().unwrap();
            if let Some(cached) = cache.get(&cache_key) {
                let elapsed = start.elapsed();
                if elapsed.as_millis() > 10 && crate::debug_logging() {
                    eprintln!(
                        "[STORAGE] filter_by_field_value CACHE HIT \
                         table={} type={} field={} op={} result_count={} elapsed_ms={}",
                        self.name,
                        type_name,
                        field_name,
                        op,
                        cached.len(),
                        elapsed.as_millis()
                    );
                }
                return cached.clone();
            }
        }

        let result_vec = if !type_name.is_empty() {
            match Self::sqlite_value_to_order_index_bytes(&target) {
                Some(enc_v) => match op {
                    "=" => {
                        let order_results =
                            self.filter_via_order_index_eq(type_name, field_name, &enc_v);
                        if order_results.is_empty() {
                            self.filter_via_table_scan_impl(field_name, op, target)
                        } else {
                            order_results
                        }
                    }
                    ">" => self
                        .filter_via_order_index_range(type_name, field_name, &enc_v, false, false),
                    ">=" => self
                        .filter_via_order_index_range(type_name, field_name, &enc_v, false, true),
                    "<" => self
                        .filter_via_order_index_range(type_name, field_name, &enc_v, true, false),
                    "<=" => {
                        self.filter_via_order_index_range(type_name, field_name, &enc_v, true, true)
                    }
                    "!=" => {
                        let mut lt = self.filter_via_order_index_range(
                            type_name, field_name, &enc_v, true, false,
                        );
                        let gt = self.filter_via_order_index_range(
                            type_name, field_name, &enc_v, false, false,
                        );
                        lt.extend(gt);
                        lt.sort_unstable();
                        lt.dedup();
                        lt
                    }
                    _ => self.filter_via_table_scan_impl(field_name, op, target),
                },
                None => self.filter_via_table_scan_impl(field_name, op, target),
            }
        } else {
            self.filter_via_table_scan_impl(field_name, op, target)
        };

        let mut cache = self.filter_cache.lock().unwrap();
        cache.put(cache_key, result_vec.clone());

        let elapsed = start.elapsed();
        if elapsed.as_millis() > 10 && crate::debug_logging() {
            eprintln!(
                "[STORAGE] filter_by_field_value \
                 table={} type={} field={} op={} result_count={} elapsed_ms={}",
                self.name,
                type_name,
                field_name,
                op,
                result_vec.len(),
                elapsed.as_millis()
            );
        }
        result_vec
    }

    fn filter_via_table_scan_impl(
        &self,
        field_name: &str,
        op: &str,
        target: rusqlite::types::Value,
    ) -> Vec<u64> {
        let field_bytes = field_name.as_bytes();
        let field_len = field_bytes.len();
        let data_prefix: u8 = 0x01;
        let expected_key_len = 1 + 8 + field_len;

        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let sql = format!(
            "SELECT substr(key, 2, 8) FROM \"{}\" \
             WHERE substr(key, 1, 1) = ?1 \
             AND length(key) = ?2 \
             AND substr(key, 10) = ?3 \
             AND json_extract(CAST(substr(value, 17) AS TEXT), '$') {} ?4",
            self.name, op
        );

        let result = (|| -> Result<Vec<u64>, rusqlite::Error> {
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params![
                    &[data_prefix][..],
                    expected_key_len as i64,
                    field_bytes,
                    target,
                ],
                |row| {
                    let uid_bytes: Vec<u8> = row.get(0)?;
                    if uid_bytes.len() == 8 {
                        Ok(byteorder::BigEndian::read_u64(&uid_bytes))
                    } else {
                        Ok(0)
                    }
                },
            )?;
            Ok(rows
                .filter_map(|r| r.ok())
                .filter(|uid| *uid != 0)
                .collect())
        })();

        self.backend.return_reader(conn);
        result.unwrap_or_default()
    }

    /// Filter pushdown for `contains` (string LIKE %target%)
    pub fn filter_by_field_contains(&self, field_name: &str, substring: &str) -> Vec<u64> {
        let field_bytes = field_name.as_bytes();
        let field_len = field_bytes.len();
        let data_prefix: u8 = 0x01;
        let expected_key_len = 1 + 8 + field_len;
        let like_pattern = format!("%{}%", substring);

        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let sql = format!(
            "SELECT substr(key, 2, 8) FROM \"{}\" \
             WHERE substr(key, 1, 1) = ?1 \
             AND length(key) = ?2 \
             AND substr(key, 10) = ?3 \
             AND CAST(json_extract(CAST(substr(value, 17) AS TEXT), '$') AS TEXT) LIKE ?4",
            self.name
        );

        let result = (|| -> Result<Vec<u64>, rusqlite::Error> {
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params![
                    &[data_prefix][..],
                    expected_key_len as i64,
                    field_bytes,
                    like_pattern,
                ],
                |row| {
                    let uid_bytes: Vec<u8> = row.get(0)?;
                    if uid_bytes.len() == 8 {
                        Ok(byteorder::BigEndian::read_u64(&uid_bytes))
                    } else {
                        Ok(0)
                    }
                },
            )?;
            Ok(rows
                .filter_map(|r| r.ok())
                .filter(|uid| *uid != 0)
                .collect())
        })();

        self.backend.return_reader(conn);
        result.unwrap_or_default()
    }

    /// Filter pushdown for `in` (value IN set)
    pub fn filter_by_field_in(
        &self,
        type_name: &str,
        field_name: &str,
        target_values: &[rusqlite::types::Value],
    ) -> Vec<u64> {
        if target_values.is_empty() {
            return vec![];
        }

        if !type_name.is_empty() {
            let encoded: Vec<Option<Vec<u8>>> = target_values
                .iter()
                .map(|v| Self::sqlite_value_to_order_index_bytes(v))
                .collect();

            if encoded.iter().all(|e| e.is_some()) {
                let mut uids: Vec<u64> = Vec::new();
                for enc_v in encoded.into_iter().flatten() {
                    let mut these = self.filter_via_order_index_eq(type_name, field_name, &enc_v);
                    uids.append(&mut these);
                }
                uids.sort_unstable();
                uids.dedup();
                return uids;
            }
        }

        let field_bytes = field_name.as_bytes();
        let field_len = field_bytes.len();
        let data_prefix: u8 = 0x01;
        let expected_key_len = 1 + 8 + field_len;

        let conn = match self.backend.get_reader() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let placeholders: Vec<String> = target_values
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 4))
            .collect();

        let sql = format!(
            "SELECT substr(key, 2, 8) FROM \"{}\" \
             WHERE substr(key, 1, 1) = ?1 \
             AND length(key) = ?2 \
             AND substr(key, 10) = ?3 \
             AND json_extract(CAST(substr(value, 17) AS TEXT), '$') IN ({})",
            self.name,
            placeholders.join(", ")
        );

        let result = (|| -> Result<Vec<u64>, rusqlite::Error> {
            let mut stmt = conn.prepare(&sql)?;
            let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            all_params.push(Box::new(vec![data_prefix]));
            all_params.push(Box::new(expected_key_len as i64));
            all_params.push(Box::new(field_bytes.to_vec()));
            for tv in target_values {
                all_params.push(Box::new(tv.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                all_params.iter().map(|p| p.as_ref()).collect();

            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                let uid_bytes: Vec<u8> = row.get(0)?;
                if uid_bytes.len() == 8 {
                    Ok(byteorder::BigEndian::read_u64(&uid_bytes))
                } else {
                    Ok(0)
                }
            })?;
            Ok(rows
                .filter_map(|r| r.ok())
                .filter(|uid| *uid != 0)
                .collect())
        })();

        self.backend.return_reader(conn);
        result.unwrap_or_default()
    }
}

// ─────────────────── KvStore Trait Implementations ───────────────────
// Bridge SqliteTable into the sub-crate abstract KvStore traits.

impl auth::state::KvStore for SqliteTable {
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
    fn test_parse_vec_dims_from_ddl() {
        assert_eq!(
            parse_vec_dims_from_ddl(
                "CREATE VIRTUAL TABLE vec_data USING vec0(uid INTEGER PRIMARY KEY, embedding float[384])"
            ),
            Some(384)
        );
        assert_eq!(
            parse_vec_dims_from_ddl(
                "CREATE VIRTUAL TABLE vec_data USING vec0(uid INTEGER PRIMARY KEY, embedding float[1024])"
            ),
            Some(1024)
        );
        assert_eq!(
            parse_vec_dims_from_ddl(
                "CREATE VIRTUAL TABLE vec_data USING vec0(uid INTEGER PRIMARY KEY, embedding FLOAT[768])"
            ),
            Some(768)
        );
        // No float[] column → unknown
        assert_eq!(parse_vec_dims_from_ddl("CREATE TABLE t(a)"), None);
        assert_eq!(
            parse_vec_dims_from_ddl("CREATE VIRTUAL TABLE v USING vec0(uid, embedding)"),
            None
        );
    }

    #[test]
    fn test_effective_vector_dims_defaults_and_override() {
        // Without configuration or env var the default applies.
        let saved = configured_vector_dims();
        std::env::remove_var("VARDADB_VECTOR_DIMS");
        CONFIGURED_VECTOR_DIMS.store(0, Ordering::SeqCst);
        assert_eq!(effective_vector_dims(), DEFAULT_VECTOR_DIMS);

        set_configured_vector_dims(1024);
        assert_eq!(configured_vector_dims(), Some(1024));
        assert_eq!(effective_vector_dims(), 1024);

        // Restore prior state.
        match saved {
            Some(d) => set_configured_vector_dims(d),
            None => CONFIGURED_VECTOR_DIMS.store(0, Ordering::SeqCst),
        }
    }

    #[test]
    fn test_native_search_tables_report_existing_dims() {
        let dir = tempdir().unwrap();
        let backend = SqliteBackend::new(dir.path().join("dims.db")).unwrap();
        set_configured_vector_dims(512);
        backend.create_native_search_tables().unwrap();
        assert_eq!(backend.vector_dims(), 512);

        // Reopening the same file must introspect 512 even if config changes.
        drop(backend);
        let reopened = SqliteBackend::new(dir.path().join("dims.db")).unwrap();
        set_configured_vector_dims(384);
        reopened.create_native_search_tables().unwrap();
        assert_eq!(reopened.vector_dims(), 512);

        // Restore default.
        set_configured_vector_dims(DEFAULT_VECTOR_DIMS);
        CONFIGURED_VECTOR_DIMS.store(0, Ordering::SeqCst);
    }

    #[test]
    fn test_prefix_upper_bound() {
        assert_eq!(
            compute_prefix_upper_bound(&[0x01, 0x02]),
            Some(vec![0x01, 0x03])
        );
        assert_eq!(compute_prefix_upper_bound(&[0x01, 0xFF]), Some(vec![0x02]));
        assert_eq!(compute_prefix_upper_bound(&[0xFF, 0xFF]), None);
        assert_eq!(compute_prefix_upper_bound(&[0x00]), Some(vec![0x01]));
    }

    #[test]
    fn test_basic_kv_operations() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(SqliteBackend::new(dir.path().join("test.db")).unwrap());
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
        let backend = Arc::new(SqliteBackend::new(dir.path().join("test.db")).unwrap());
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
        let backend = Arc::new(SqliteBackend::new(dir.path().join("test.db")).unwrap());
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
        let backend = Arc::new(SqliteBackend::new(dir.path().join("test.db")).unwrap());
        backend.create_table("test").unwrap();
        let table = SqliteTable::new("test".to_string(), backend.clone());

        backend
            .write_batch(|conn| {
                table.batch_insert_on_conn(conn, b"k1", b"v1")?;
                table.batch_insert_on_conn(conn, b"k2", b"v2")?;
                table.batch_insert_on_conn(conn, b"k3", b"v3")?;
                Ok(())
            })
            .unwrap();

        assert_eq!(table.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(table.get(b"k2").unwrap(), Some(b"v2".to_vec()));
        assert_eq!(table.get(b"k3").unwrap(), Some(b"v3".to_vec()));
    }

    #[test]
    fn test_iter() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(SqliteBackend::new(dir.path().join("test.db")).unwrap());
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
