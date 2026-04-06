use byteorder::ByteOrder;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, TableHandle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{error, info};

macro_rules! dbg_info {
    ($($arg:tt)*) => {
        if crate::debug_logging() {
            info!($($arg)*)
        }
    };
}

/// Low-level redb backend — manages a single redb database file.
/// Replaces `SqliteBackend`.
///
/// # Durability Guarantee
///
/// All write transactions use `Durability::Immediate` (the default).
/// Every commit calls fsync to ensure data is persisted to disk before returning.
/// This guarantees zero data loss on power failure at the cost of write latency.
///
/// DO NOT use `db.begin_write_with_txn_config()` with `Durability::Eventual`
/// anywhere in this codebase — that would break the durability guarantee.
pub struct RedbBackend {
    db: Database,
    path: PathBuf,
}

impl RedbBackend {
    /// Open (or create) a redb database at the exact given path.
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        dbg_info!(db_path = %db_path.display(), "RedbBackend: opening database");

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
            dbg_info!(parent = %parent.display(), "RedbBackend: ensured parent directory exists");
        }

        let db = Database::create(&db_path)?;
        dbg_info!(db_path = %db_path.display(), "RedbBackend: database opened");

        Ok(Self { db, path: db_path })
    }

    /// Create a table by opening a write transaction and defining it.
    /// redb creates tables on first use — this just ensures the table exists.
    pub fn create_table(&self, name: &str) -> anyhow::Result<()> {
        dbg_info!(db_path = %self.path.display(), table = %name, "RedbBackend: creating table if needed");
        let leaked_name: &'static str = Box::leak(name.to_string().into_boxed_str());
        let table_def = TableDefinition::<&[u8], &[u8]>::new(leaked_name);
        let write_txn = self.db.begin_write()?;
        {
            let _table = write_txn.open_table(table_def)?;
        }
        write_txn.commit()?;
        dbg_info!(db_path = %self.path.display(), table = %name, "RedbBackend: create_table complete");
        Ok(())
    }

    /// Create a main data table. In redb, there's no schema difference between
    /// a "main" table and a regular table — the ts is encoded in the value.
    pub fn create_main_table(&self, name: &str) -> anyhow::Result<()> {
        self.create_table(name)
    }

    /// Native search tables are stubbed — FTS and vector are out of scope.
    pub fn create_native_search_tables(&self) -> anyhow::Result<()> {
        dbg_info!(db_path = %self.path.display(), "RedbBackend: native search tables stubbed (FTS/vector out of scope)");
        Ok(())
    }

    /// Drop a table.
    pub fn drop_table(&self, name: &str) -> anyhow::Result<()> {
        let leaked_name: &'static str = Box::leak(name.to_string().into_boxed_str());
        let table_def = TableDefinition::<&[u8], &[u8]>::new(leaked_name);
        let write_txn = self.db.begin_write()?;
        let _ = write_txn.delete_table(table_def)?;
        write_txn.commit()?;
        Ok(())
    }

    /// List all table names.
    pub fn list_tables(&self) -> Vec<String> {
        let read_txn = match self.db.begin_read() {
            Ok(t) => t,
            Err(_) => return vec![],
        };
        read_txn
            .list_tables()
            .map(|iter| iter.map(|t| t.name().to_string()).collect())
            .unwrap_or_default()
    }

    /// Execute a batch of writes in a single transaction.
    pub fn write_batch<F>(&self, f: F) -> anyhow::Result<()>
    where
        F: FnOnce(&redb::WriteTransaction) -> anyhow::Result<()>,
    {
        dbg_info!(db_path = %self.path.display(), "RedbBackend: starting write batch");
        let write_txn = self.db.begin_write()?;
        f(&write_txn)?;
        write_txn.commit()?;
        dbg_info!(db_path = %self.path.display(), "RedbBackend: write batch committed");
        Ok(())
    }

    /// No-op for redb. redb manages its own durability.
    pub fn shutdown(&self) -> anyhow::Result<()> {
        dbg_info!(db_path = %self.path.display(), "RedbBackend: shutdown (no-op for redb)");
        Ok(())
    }

    /// Get the database path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ─────────────────── RedbTable ───────────────────

/// A handle to a named table in the redb database.
/// Replaces `SqliteTable`. Clone is cheap (just Arc + static str + bool).
///
/// The table name is leaked to `&'static str` because redb's `TableDefinition`
/// requires `'static` lifetime for the name. This is fine since table names
/// are persistent and few in number.
#[derive(Clone)]
pub struct RedbTable {
    pub name: String,
    /// Leaked static reference to the table name for redb
    table_name: &'static str,
    backend: Arc<RedbBackend>,
    /// If true, this table stores values with a 16-byte timestamp prefix (main tables).
    has_ts: bool,
}

impl RedbTable {
    /// Create a handle for a regular table (no timestamp prefix).
    pub fn new(name: String, backend: Arc<RedbBackend>) -> Self {
        let table_name: &'static str = Box::leak(name.clone().into_boxed_str());
        Self {
            name,
            table_name,
            backend,
            has_ts: false,
        }
    }

    /// Create a handle for a main table (values have a 16-byte ts prefix).
    pub fn new_main(name: String, backend: Arc<RedbBackend>) -> Self {
        let table_name: &'static str = Box::leak(name.clone().into_boxed_str());
        Self {
            name,
            table_name,
            backend,
            has_ts: true,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn table_def(&self) -> TableDefinition<'static, &'static [u8], &'static [u8]> {
        TableDefinition::new(self.table_name)
    }

    // ── Reads ──

    pub fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let read_txn = self.backend.db.begin_read()?;
        let table = match read_txn.open_table(self.table_def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        match table.get(key)? {
            Some(guard) => {
                let bytes = guard.value();
                if self.has_ts && bytes.len() > 16 {
                    // Main table: value is [ts:16][actual_value:N]
                    // Return just the actual value (skip ts prefix)
                    Ok(Some(bytes[16..].to_vec()))
                } else {
                    Ok(Some(bytes.to_vec()))
                }
            }
            None => Ok(None),
        }
    }

    pub fn contains_key(&self, key: &[u8]) -> anyhow::Result<bool> {
        let read_txn = self.backend.db.begin_read()?;
        let table = match read_txn.open_table(self.table_def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        Ok(table.get(key)?.is_some())
    }

    /// Prefix scan: returns all (key, value) pairs where key starts with `prefix`.
    /// Results are ordered by key (B-tree natural order).
    pub fn prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let start = std::time::Instant::now();
        let read_txn = match self.backend.db.begin_read() {
            Ok(t) => t,
            Err(e) => {
                error!(table = %self.name, error = %e, "RedbTable: prefix scan failed");
                return vec![];
            }
        };
        let table = match read_txn.open_table(self.table_def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return vec![],
            Err(e) => {
                error!(table = %self.name, error = %e, "RedbTable: prefix scan failed to open table");
                return vec![];
            }
        };

        let upper = compute_prefix_upper_bound(prefix);
        let result = if let Some(ref upper) = upper {
            match table.range::<&[u8]>(prefix..upper.as_slice()) {
                Ok(iter) => iter
                    .filter_map(|r| r.ok())
                    .map(|(k, v)| {
                        let value = if self.has_ts && v.value().len() > 16 {
                            v.value()[16..].to_vec()
                        } else {
                            v.value().to_vec()
                        };
                        (k.value().to_vec(), value)
                    })
                    .collect(),
                Err(_) => vec![],
            }
        } else {
            // Prefix is all 0xFF — scan from prefix to end
            match table.range::<&[u8]>(prefix..) {
                Ok(iter) => iter
                    .filter_map(|r| r.ok())
                    .take_while(|(k, _)| k.value().starts_with(prefix))
                    .map(|(k, v)| {
                        let value = if self.has_ts && v.value().len() > 16 {
                            v.value()[16..].to_vec()
                        } else {
                            v.value().to_vec()
                        };
                        (k.value().to_vec(), value)
                    })
                    .collect(),
                Err(_) => vec![],
            }
        };

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
        let read_txn = match self.backend.db.begin_read() {
            Ok(t) => t,
            Err(e) => {
                error!(table = %self.name, error = %e, "RedbTable: range scan failed");
                return vec![];
            }
        };
        let table = match read_txn.open_table(self.table_def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return vec![],
            Err(e) => {
                error!(table = %self.name, error = %e, "RedbTable: range scan failed to open table");
                return vec![];
            }
        };

        let result = match table.range::<&[u8]>(lower..upper) {
            Ok(iter) => iter
                .filter_map(|r| r.ok())
                .map(|(k, v)| {
                    let value = if self.has_ts && v.value().len() > 16 {
                        v.value()[16..].to_vec()
                    } else {
                        v.value().to_vec()
                    };
                    (k.value().to_vec(), value)
                })
                .collect(),
            Err(_) => vec![],
        };

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
        let read_txn = self.backend.db.begin_read()?;
        let table = match read_txn.open_table(self.table_def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(0),
            Err(e) => return Err(e.into()),
        };

        let upper = compute_prefix_upper_bound(prefix);
        let count = if let Some(ref upper) = upper {
            match table.range::<&[u8]>(prefix..upper.as_slice()) {
                Ok(iter) => iter.filter_map(|r| r.ok()).count(),
                Err(_) => 0,
            }
        } else {
            match table.range::<&[u8]>(prefix..) {
                Ok(iter) => iter
                    .filter_map(|r| r.ok())
                    .take_while(|(k, _)| k.value().starts_with(prefix))
                    .count(),
                Err(_) => 0,
            }
        };

        Ok(count)
    }

    /// Iterate over all entries in the table, ordered by key.
    pub fn iter(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let start = std::time::Instant::now();
        let read_txn = match self.backend.db.begin_read() {
            Ok(t) => t,
            Err(e) => {
                error!(table = %self.name, error = %e, "RedbTable: full scan failed");
                return vec![];
            }
        };
        let table = match read_txn.open_table(self.table_def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return vec![],
            Err(e) => {
                error!(table = %self.name, error = %e, "RedbTable: full scan failed to open table");
                return vec![];
            }
        };

        let result = match table.range::<&[u8]>(..) {
            Ok(iter) => iter
                .filter_map(|r| r.ok())
                .map(|(k, v)| {
                    let value = if self.has_ts && v.value().len() > 16 {
                        v.value()[16..].to_vec()
                    } else {
                        v.value().to_vec()
                    };
                    (k.value().to_vec(), value)
                })
                .collect(),
            Err(_) => vec![],
        };

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

    // ── Writes ──

    /// Insert or replace a key-value pair.
    /// For main tables (has_ts=true), auto-provides a zero timestamp prefix.
    pub fn insert(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> anyhow::Result<()> {
        let write_txn = self.backend.db.begin_write()?;
        {
            let mut table = write_txn.open_table(self.table_def())?;
            if self.has_ts {
                let default_ts = [0u8; 16];
                let mut combined = Vec::with_capacity(16 + value.as_ref().len());
                combined.extend_from_slice(&default_ts);
                combined.extend_from_slice(value.as_ref());
                table.insert(key.as_ref(), combined.as_slice())?;
            } else {
                table.insert(key.as_ref(), value.as_ref())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Remove a key.
    pub fn remove(&self, key: impl AsRef<[u8]>) -> anyhow::Result<()> {
        let write_txn = self.backend.db.begin_write()?;
        {
            let mut table = write_txn.open_table(self.table_def())?;
            table.remove(key.as_ref())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    // ── LWW-specific operations ──

    /// Get value + timestamp from a main table.
    /// Returns (value_without_ts, ts_bytes).
    pub fn get_with_ts(&self, key: &[u8]) -> anyhow::Result<Option<(Vec<u8>, Vec<u8>)>> {
        let read_txn = self.backend.db.begin_read()?;
        let table = match read_txn.open_table(self.table_def()) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        match table.get(key)? {
            Some(guard) => {
                let bytes = guard.value();
                if bytes.len() >= 16 {
                    let ts = bytes[..16].to_vec();
                    let value = bytes[16..].to_vec();
                    Ok(Some((value, ts)))
                } else {
                    Ok(Some((bytes.to_vec(), vec![0u8; 16])))
                }
            }
            None => Ok(None),
        }
    }

    /// Atomic LWW upsert: only writes if the new timestamp is greater than the existing one.
    /// Value is stored as [ts:16][value:N].
    pub fn upsert_lww(&self, key: &[u8], value: &[u8], ts: &[u8]) -> anyhow::Result<()> {
        let write_txn = self.backend.db.begin_write()?;
        {
            let mut table = write_txn.open_table(self.table_def())?;
            // Check existing ts
            let should_write = match table.get(key)? {
                Some(existing) => {
                    let existing_bytes = existing.value();
                    if existing_bytes.len() >= 16 {
                        ts > &existing_bytes[..16]
                    } else {
                        true
                    }
                }
                None => true,
            };

            if should_write {
                let mut combined = Vec::with_capacity(16 + value.len());
                combined.extend_from_slice(ts);
                combined.extend_from_slice(value);
                table.insert(key, combined.as_slice())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Delete from a main table only if the given timestamp is newer.
    /// Returns true if the delete was applied (not stale).
    pub fn delete_lww(&self, key: &[u8], ts: &[u8]) -> anyhow::Result<bool> {
        let write_txn = self.backend.db.begin_write()?;
        let deleted = {
            let mut table = write_txn.open_table(self.table_def())?;
            let should_delete = match table.get(key)? {
                Some(existing) => {
                    let existing_bytes = existing.value();
                    if existing_bytes.len() >= 16 {
                        ts > &existing_bytes[..16]
                    } else {
                        true
                    }
                }
                None => false, // Nothing to delete
            };

            if should_delete {
                table.remove(key)?;
                true
            } else {
                false
            }
        };
        write_txn.commit()?;
        Ok(deleted)
    }

    /// Batch insert within an existing write transaction.
    pub fn batch_insert_on_txn(
        &self,
        write_txn: &redb::WriteTransaction,
        key: &[u8],
        value: &[u8],
    ) -> anyhow::Result<()> {
        let mut table = write_txn.open_table(self.table_def())?;
        if self.has_ts {
            let default_ts = [0u8; 16];
            let mut combined = Vec::with_capacity(16 + value.len());
            combined.extend_from_slice(&default_ts);
            combined.extend_from_slice(value);
            table.insert(key, combined.as_slice())?;
        } else {
            table.insert(key, value)?;
        }
        Ok(())
    }

    /// Batch LWW upsert within an existing write transaction.
    pub fn batch_upsert_lww_on_txn(
        &self,
        write_txn: &redb::WriteTransaction,
        key: &[u8],
        value: &[u8],
        ts: &[u8],
    ) -> anyhow::Result<()> {
        let mut table = write_txn.open_table(self.table_def())?;
        let should_write = match table.get(key)? {
            Some(existing) => {
                let existing_bytes = existing.value();
                if existing_bytes.len() >= 16 {
                    ts > &existing_bytes[..16]
                } else {
                    true
                }
            }
            None => true,
        };

        if should_write {
            let mut combined = Vec::with_capacity(16 + value.len());
            combined.extend_from_slice(ts);
            combined.extend_from_slice(value);
            table.insert(key, combined.as_slice())?;
        }
        Ok(())
    }

    /// Filter pushdown — scan via B-tree order index keys.
    pub fn filter_by_field_value(
        &self,
        type_name: &str,
        field_name: &str,
        op: &str,
        target: FilterTarget,
    ) -> Vec<u64> {
        let enc_v = match target.to_order_index_bytes() {
            Some(v) => v,
            None => return self.filter_via_table_scan_impl(field_name, op, &target),
        };

        if !type_name.is_empty() {
            match op {
                "=" => {
                    let order_results =
                        self.filter_via_order_index_eq(type_name, field_name, &enc_v);
                    if order_results.is_empty() {
                        self.filter_via_table_scan_impl(field_name, op, &target)
                    } else {
                        order_results
                    }
                }
                ">" => {
                    self.filter_via_order_index_range(type_name, field_name, &enc_v, false, false)
                }
                ">=" => {
                    self.filter_via_order_index_range(type_name, field_name, &enc_v, false, true)
                }
                "<" => {
                    self.filter_via_order_index_range(type_name, field_name, &enc_v, true, false)
                }
                "<=" => {
                    self.filter_via_order_index_range(type_name, field_name, &enc_v, true, true)
                }
                "!=" => {
                    let mut lt = self
                        .filter_via_order_index_range(type_name, field_name, &enc_v, true, false);
                    let gt = self
                        .filter_via_order_index_range(type_name, field_name, &enc_v, false, false);
                    lt.extend(gt);
                    lt.sort_unstable();
                    lt.dedup();
                    lt
                }
                _ => self.filter_via_table_scan_impl(field_name, op, &target),
            }
        } else {
            self.filter_via_table_scan_impl(field_name, op, &target)
        }
    }

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

    fn filter_via_table_scan_impl(
        &self,
        field_name: &str,
        op: &str,
        target: &FilterTarget,
    ) -> Vec<u64> {
        let field_bytes = field_name.as_bytes();
        let field_len = field_bytes.len();
        let data_prefix: u8 = 0x01;
        let expected_key_len = 1 + 8 + field_len;

        let prefix = vec![data_prefix];
        let entries = self.prefix(&prefix);

        let mut result = Vec::new();
        for (key, value) in entries {
            if key.len() != expected_key_len {
                continue;
            }
            if &key[9..] != field_bytes {
                continue;
            }
            let uid = byteorder::BigEndian::read_u64(&key[1..9]);
            if uid == 0 {
                continue;
            }
            if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&value) {
                if target.compare(&json_val, op) {
                    result.push(uid);
                }
            }
        }
        result
    }

    /// Filter pushdown for `contains` (string LIKE %target%)
    pub fn filter_by_field_contains(&self, field_name: &str, substring: &str) -> Vec<u64> {
        let field_bytes = field_name.as_bytes();
        let field_len = field_bytes.len();
        let data_prefix: u8 = 0x01;
        let expected_key_len = 1 + 8 + field_len;

        let prefix = vec![data_prefix];
        let entries = self.prefix(&prefix);

        let mut result = Vec::new();
        let lower_substring = substring.to_lowercase();
        for (key, value) in entries {
            if key.len() != expected_key_len || &key[9..] != field_bytes {
                continue;
            }
            let uid = byteorder::BigEndian::read_u64(&key[1..9]);
            if uid == 0 {
                continue;
            }
            if let Ok(serde_json::Value::String(s)) = serde_json::from_slice(&value) {
                if s.to_lowercase().contains(&lower_substring) {
                    result.push(uid);
                }
            }
        }
        result
    }

    /// Filter pushdown for `in` (value IN set)
    pub fn filter_by_field_in(
        &self,
        type_name: &str,
        field_name: &str,
        target_values: &[FilterTarget],
    ) -> Vec<u64> {
        if target_values.is_empty() {
            return vec![];
        }

        if !type_name.is_empty() {
            let encoded: Vec<Option<Vec<u8>>> = target_values
                .iter()
                .map(|v| v.to_order_index_bytes())
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

        // Fall back to table scan
        let field_bytes = field_name.as_bytes();
        let field_len = field_bytes.len();
        let data_prefix: u8 = 0x01;
        let expected_key_len = 1 + 8 + field_len;
        let prefix = vec![data_prefix];
        let entries = self.prefix(&prefix);

        let mut result = Vec::new();
        for (key, value) in entries {
            if key.len() != expected_key_len || &key[9..] != field_bytes {
                continue;
            }
            let uid = byteorder::BigEndian::read_u64(&key[1..9]);
            if uid == 0 {
                continue;
            }
            if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&value) {
                if target_values.iter().any(|t| t.compare(&json_val, "=")) {
                    result.push(uid);
                }
            }
        }
        result
    }
}

// ─────────────────── FilterTarget ───────────────────

/// A typed filter value for comparison operations.
/// Represents a value for backend-agnostic filter pushdown.
/// Used to compare against stored JSON values in predicates.
#[derive(Clone, Debug)]
pub enum FilterTarget {
    Text(String),
    Integer(i64),
    Real(f64),
    Boolean(bool),
    Null,
}

impl FilterTarget {
    fn encode_sortable_f64(value: f64) -> [u8; 8] {
        let bits = value.to_bits();
        let sortable = if bits & (1 << 63) != 0 {
            !bits
        } else {
            bits ^ (1 << 63)
        };
        sortable.to_be_bytes()
    }

    /// Convert to bytes for the order index.
    pub fn to_order_index_bytes(&self) -> Option<Vec<u8>> {
        match self {
            FilterTarget::Text(s) => Some(s.as_bytes().to_vec()),
            FilterTarget::Integer(i) => Some(Self::encode_sortable_f64(*i as f64).to_vec()),
            FilterTarget::Real(f) => Some(Self::encode_sortable_f64(*f).to_vec()),
            _ => None,
        }
    }

    /// Compare a JSON value against this target using the given operator.
    pub fn compare(&self, json_val: &serde_json::Value, op: &str) -> bool {
        match (self, json_val) {
            (FilterTarget::Text(target), serde_json::Value::String(actual)) => match op {
                "=" => actual == target,
                "!=" => actual != target,
                ">" => actual.as_str() > target.as_str(),
                ">=" => actual.as_str() >= target.as_str(),
                "<" => actual.as_str() < target.as_str(),
                "<=" => actual.as_str() <= target.as_str(),
                "LIKE" => {
                    let pattern = target.replace('%', "");
                    actual.to_lowercase().contains(&pattern.to_lowercase())
                }
                _ => false,
            },
            (FilterTarget::Integer(target), serde_json::Value::Number(n)) => {
                if let Some(actual) = n.as_i64() {
                    match op {
                        "=" => actual == *target,
                        "!=" => actual != *target,
                        ">" => actual > *target,
                        ">=" => actual >= *target,
                        "<" => actual < *target,
                        "<=" => actual <= *target,
                        _ => false,
                    }
                } else if let Some(actual) = n.as_f64() {
                    let target_f = *target as f64;
                    match op {
                        "=" => (actual - target_f).abs() < f64::EPSILON,
                        "!=" => (actual - target_f).abs() >= f64::EPSILON,
                        ">" => actual > target_f,
                        ">=" => actual >= target_f,
                        "<" => actual < target_f,
                        "<=" => actual <= target_f,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            (FilterTarget::Real(target), serde_json::Value::Number(n)) => {
                if let Some(actual) = n.as_f64() {
                    match op {
                        "=" => (actual - target).abs() < f64::EPSILON,
                        "!=" => (actual - target).abs() >= f64::EPSILON,
                        ">" => actual > *target,
                        ">=" => actual >= *target,
                        "<" => actual < *target,
                        "<=" => actual <= *target,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            (FilterTarget::Boolean(target), serde_json::Value::Bool(actual)) => match op {
                "=" => actual == target,
                "!=" => actual != target,
                _ => false,
            },
            (FilterTarget::Integer(target), serde_json::Value::Bool(actual)) => {
                let actual_int = if *actual { 1i64 } else { 0i64 };
                match op {
                    "=" => actual_int == *target,
                    "!=" => actual_int != *target,
                    _ => false,
                }
            }
            (FilterTarget::Null, serde_json::Value::Null) => matches!(op, "="),
            (FilterTarget::Null, _) => matches!(op, "!="),
            (_, serde_json::Value::Null) => matches!(op, "!="),
            _ => false,
        }
    }
}

// ─────────────────── KvStore Trait Implementations ───────────────────

impl auth::state::KvStore for RedbTable {
    fn kv_insert(&self, key: &[u8], value: &[u8]) -> Result<(), String> {
        self.insert(key, value).map_err(|e| e.to_string())
    }

    fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.get(key).map_err(|e| e.to_string())
    }

    fn kv_remove(&self, key: &[u8]) -> Result<(), String> {
        self.remove(key).map_err(|e| e.to_string())
    }

    fn kv_prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.prefix(prefix)
    }
}

impl permissions::storage::auth_store::KvStore for RedbTable {
    fn kv_insert(&self, key: &[u8], value: &[u8]) -> Result<(), String> {
        self.insert(key, value).map_err(|e| e.to_string())
    }

    fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.get(key).map_err(|e| e.to_string())
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
        let backend = Arc::new(RedbBackend::new(dir.path().join("test.redb")).unwrap());
        backend.create_table("test").unwrap();
        let table = RedbTable::new("test".to_string(), backend.clone());

        table.insert(b"key1", b"value1").unwrap();
        table.insert(b"key2", b"value2").unwrap();

        assert_eq!(table.get(b"key1").unwrap(), Some(b"value1".to_vec()));
        assert_eq!(table.get(b"key2").unwrap(), Some(b"value2".to_vec()));
        assert_eq!(table.get(b"key3").unwrap(), None);

        assert!(table.contains_key(b"key1").unwrap());
        assert!(!table.contains_key(b"key3").unwrap());

        table.remove(b"key1").unwrap();
        assert_eq!(table.get(b"key1").unwrap(), None);

        table.insert(b"key2", b"updated").unwrap();
        assert_eq!(table.get(b"key2").unwrap(), Some(b"updated".to_vec()));
    }

    #[test]
    fn test_prefix_scan() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(RedbBackend::new(dir.path().join("test.redb")).unwrap());
        backend.create_table("test").unwrap();
        let table = RedbTable::new("test".to_string(), backend.clone());

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
        let backend = Arc::new(RedbBackend::new(dir.path().join("test.redb")).unwrap());
        backend.create_main_table("main").unwrap();
        let table = RedbTable::new_main("main".to_string(), backend.clone());

        let key = b"test_key";
        let ts1 = [0u8; 16];
        let mut ts2 = [0u8; 16];
        ts2[15] = 1;

        table.upsert_lww(key, b"first", &ts1).unwrap();
        let (val, ts) = table.get_with_ts(key).unwrap().unwrap();
        assert_eq!(val, b"first".to_vec());
        assert_eq!(ts, ts1.to_vec());

        table.upsert_lww(key, b"second", &ts2).unwrap();
        let (val, _) = table.get_with_ts(key).unwrap().unwrap();
        assert_eq!(val, b"second".to_vec());

        table.upsert_lww(key, b"stale", &ts1).unwrap();
        let (val, _) = table.get_with_ts(key).unwrap().unwrap();
        assert_eq!(val, b"second".to_vec());
    }

    #[test]
    fn test_write_batch() {
        let dir = tempdir().unwrap();
        let backend = Arc::new(RedbBackend::new(dir.path().join("test.redb")).unwrap());
        backend.create_table("test").unwrap();
        let table = RedbTable::new("test".to_string(), backend.clone());

        backend
            .write_batch(|txn| {
                table.batch_insert_on_txn(txn, b"k1", b"v1")?;
                table.batch_insert_on_txn(txn, b"k2", b"v2")?;
                table.batch_insert_on_txn(txn, b"k3", b"v3")?;
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
        let backend = Arc::new(RedbBackend::new(dir.path().join("test.redb")).unwrap());
        backend.create_table("test").unwrap();
        let table = RedbTable::new("test".to_string(), backend.clone());

        table.insert(b"c", b"3").unwrap();
        table.insert(b"a", b"1").unwrap();
        table.insert(b"b", b"2").unwrap();

        let all = table.iter();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].0, b"a".to_vec());
        assert_eq!(all[1].0, b"b".to_vec());
        assert_eq!(all[2].0, b"c".to_vec());
    }
}
