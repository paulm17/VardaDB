# VardaDB Storage Migration: SQLite → redb + Tantivy + usearch

Migrate VardaDB's entire storage stack from SQLite to a pure-Rust stack: **redb** for KV, **Tantivy** for full-text search, and **usearch** for vector search. Includes engine-level performance optimizations.

---

## User Review Required

> [!IMPORTANT]
> **Breaking change — data migration**: Existing `.db` files (SQLite) will not be readable by the new engine. A one-time offline migration tool (Phase 3) will be provided, but existing deployments must run it before upgrading. Alternatively, we could support a "dual-read" mode during a transition period. Please advise which approach you prefer.

> [!WARNING]
> **Tantivy index directory layout**: Tantivy stores its index as a directory of segment files, not a single file. Each database will need a `tantivy/` subdirectory alongside its `redb` file. This changes the on-disk layout from `{name}.db` → `{name}.redb` + `{name}_tantivy/`.

> [!IMPORTANT]
> **usearch persistence**: usearch indexes can be serialised to/from disk, but the index must be explicitly saved on shutdown and loaded on startup. The current sqlite-vec approach piggybacks on SQLite's WAL for durability. With usearch, we need explicit save/load lifecycle management. Are you okay with the vector index being rebuilt from source data if the save file is missing?

> [!IMPORTANT]
> **`parking_lot` vs `std::sync`**: The plan replaces `std::sync::Mutex` with `parking_lot::Mutex` in Phase 1. `parking_lot` is already a dependency. This also affects the `auth` and `permissions` sub-crates indirectly, since `SqliteTable` implements their `KvStore` traits. The trait signatures use `&self` (not `&mut self`), so this is transparent. Just confirming you're comfortable with this.

---

## Proposed Changes

The migration is structured into 5 phases, matching the plan.md roadmap. Each phase is self-contained and testable independently.

---

### Phase 1: Zero-Hanging Fruit (Immediate Optimizations)

Low-risk, high-reward changes that are independent of the storage migration. These can ship immediately.

#### [MODIFY] [main.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/main.rs)
- Add `#[global_allocator]` with `mimalloc` (or `jemalloc` — see Open Questions)
- Single line at the top of the file

#### [MODIFY] [Cargo.toml](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/Cargo.toml)
- Add `mimalloc = { version = "0.1", default-features = false }` (or `tikv-jemallocator`)
- Add `xxhash-rust = { version = "0.8", features = ["xxh3"] }` for item hashing
- Add `ahash = "0.8"` for QueryCache key hashing

#### [MODIFY] [backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/backend.rs)
- **Hashing**: Replace the FNV-1a `hash_item` (lines 794-804) with `xxhash_rust::xxh3::xxh3_64`. The XOR accumulation in `update_history_hash` is preserved — only the per-item hash function changes.
- **Locking**: Replace `std::sync::Mutex` with `parking_lot::Mutex` for `clock` field (line 62) and `ACTIVE_STORAGES` (line 26).

#### [MODIFY] [sqlite_backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/sqlite_backend.rs)
- **Locking**: Replace `std::sync::Mutex` with `parking_lot::Mutex` for `writer` (line 24), `reader_pool` (line 26), and `filter_cache` (line 273).
- Update `.lock().unwrap()` → `.lock()` (parking_lot doesn't return `Result`).

#### [MODIFY] [cache.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/engine/cache.rs)
- **Locking**: Replace `std::sync::Mutex` with `parking_lot::Mutex` for `inner` (line 14).
- **Hashing**: Replace `DefaultHasher` in `hash_query` (line 48) with `ahash::AHasher`.

#### [MODIFY] [resolver.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/engine/resolver.rs)
- **Locking**: Replace `std::sync::Mutex` with `parking_lot::Mutex` for all three fields in `RequestCache` (lines 31-33).

**Files touched**: 6  
**Risk**: Low — all changes are drop-in replacements  
**Verification**: Run existing `cargo test` suite; no behavioral changes expected

---

### Phase 2: Interface Refactoring (Streaming API)

Refactor `SqliteTable` methods to return iterators instead of materialised `Vec`s. This prepares the interface for redb's lazy `AccessGuard` pattern and provides immediate memory wins on the SQLite backend.

#### [MODIFY] [sqlite_backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/sqlite_backend.rs)

Introduce a new trait or modify the existing `SqliteTable` API:

```rust
/// A lazy key-value iterator that holds a reader connection.
/// Drops the connection back to the pool when the iterator is dropped.
pub struct KvIterator {
    rows: Vec<(Vec<u8>, Vec<u8>)>, // Phase 2: still materialized internally
    pos: usize,
}

impl Iterator for KvIterator {
    type Item = (Vec<u8>, Vec<u8>);
    fn next(&mut self) -> Option<Self::Item> { ... }
}
```

- Change `prefix()`, `range()`, and `iter()` return types from `Vec<(Vec<u8>, Vec<u8>)>` → `KvIterator`
- This is a **transitional** type — in Phase 3, `KvIterator` will wrap a redb `Range` cursor instead of a materialized `Vec`
- **Guard-based reader pattern**: Replace `get_reader()` / `return_reader()` with an RAII guard:
  ```rust
  pub struct ReaderGuard<'a> {
      conn: Option<Connection>,
      pool: &'a SqliteBackend,
  }
  impl Drop for ReaderGuard<'_> {
      fn drop(&mut self) {
          if let Some(conn) = self.conn.take() {
              self.pool.return_reader(conn);
          }
      }
  }
  ```

#### [MODIFY] [backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/backend.rs)
- Update `get_history_range()` (line 670) to return `impl Iterator` instead of `Vec`
- Update `scan_quarantine()` to return an iterator
- Update fingerprint rebuild logic (`rebuild_fingerprints`, `spawn_fingerprint_rebuild`) to consume iterators

#### [MODIFY] [sqlite_resolver.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/bridge/sqlite_resolver.rs)
- Update all call sites of `prefix()`, `range()`, `iter()` to consume iterators
- Key locations:
  - `preload_objects_for_uids()` (line 52) — range scan
  - `load_object_fields()` (line 80) — prefix scan  
  - `load_related_uids()` (line 138) — prefix scan
  - `sorted_index_scan()` (line 371) — prefix scan
  - `rebuild_order_index_for_field()` (line 423) — range scan
  - `scan_nodes` impl (line ~3300+) — prefix scan
  - All `filter_via_order_index_*` methods

**Files touched**: 3  
**Risk**: Medium — API surface change, but type system enforces correctness  
**Verification**: Full `cargo test` suite must pass; no behavioral changes

---

### Phase 3: redb Integration (The Core Migration)

Replace `SqliteBackend` + `SqliteTable` with `RedbBackend` + `RedbTable`. Remove the `rusqlite` dependency entirely.

#### [NEW] [redb_backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/redb_backend.rs)

The new storage backend. Key design:

```rust
use redb::{Database, ReadTransaction, WriteTransaction, TableDefinition};

const KV_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("kv");
const KV_TABLE_TS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("kv_ts");

pub struct RedbBackend {
    db: Database,
    path: PathBuf,
}

pub struct RedbTable {
    pub name: String,
    backend: Arc<RedbBackend>,
    has_ts: bool,
    // Note: filter_cache is removed — to be redesigned for B-tree iteration
}
```

**API mapping** (from plan.md):

| Current (`SqliteTable`)   | New (`RedbTable`)                                  |
|---------------------------|-----------------------------------------------------|
| `get(key)` → `Vec<u8>`   | `get(key)` → `AccessGuard<&[u8]>` (zero-copy)      |
| `prefix(p)` → `Vec<..>`  | `prefix(p)` → `RedbIterator` (lazy B-tree cursor)   |
| `range(l, u)` → `Vec<..>`| `range(l, u)` → `RedbIterator` (native range)       |
| `iter()` → `Vec<..>`     | `iter()` → `RedbIterator` (full table scan)          |
| `insert(k, v)`           | `insert(k, v)` via `WriteTransaction`                |
| `upsert_lww(k, v, ts)`   | Read-then-write within single `WriteTransaction`     |
| `write_batch(closure)`   | Single `WriteTransaction` wrapping all ops           |

**Concurrency model change**:
- No reader pool — `db.begin_read()` is cheap (MVCC snapshot)
- No writer mutex — `db.begin_write()` serializes internally
- The `ReaderGuard` from Phase 2 becomes a thin wrapper around `ReadTransaction`

**Multi-database layout**: One `.redb` file per logical database, mirroring the current multi-`.db` setup:
```
varda_db_data/
  default.redb          # was default.db
  myapp.redb            # was myapp.db
```

`Storage.backends` type changes: `DashMap<String, Arc<SqliteBackend>>` → `DashMap<String, Arc<RedbBackend>>`

**LWW upsert strategy**: redb has no SQL `ON CONFLICT` clause. The LWW check becomes:
```rust
fn upsert_lww(&self, key: &[u8], value: &[u8], ts: &[u8]) -> Result<()> {
    let write_txn = self.backend.db.begin_write()?;
    {
        let mut table = write_txn.open_table(KV_TABLE_TS)?;
        // Read current ts
        if let Some(existing) = table.get(key)? {
            let existing_bytes = existing.value();
            let (_, existing_ts) = split_value_ts(existing_bytes);
            if existing_ts >= ts {
                return Ok(()); // Stale write
            }
        }
        // Write value + ts
        let mut combined = Vec::with_capacity(value.len() + ts.len());
        combined.extend_from_slice(ts);
        combined.extend_from_slice(value);
        table.insert(key, combined.as_slice())?;
    }
    write_txn.commit()?;
    Ok(())
}
```

> [!NOTE]
> For main tables with LWW, the value encoding changes: the ts is stored as a prefix of the value blob (`[ts:16][value:N]`) rather than in a separate SQL column. This simplifies the table schema to a single `TableDefinition<&[u8], &[u8]>`.

#### [MODIFY] [backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/backend.rs)
- Replace all `SqliteBackend` / `SqliteTable` references with `RedbBackend` / `RedbTable`
- `Storage` struct fields change:
  - `backends: DashMap<String, Arc<RedbBackend>>`
  - `keyspaces: RwLock<HashMap<String, (RedbTable, RedbTable)>>`
  - `sys_table: RedbTable`
  - All system tables become `RedbTable`
- Vector worker: Replace `rusqlite` SQL with direct redb table ops for vector storage (vectors stored as raw bytes in a redb table, pending usearch migration in Phase 4)
- `search_vectors()`: Temporarily disabled or uses brute-force scan until Phase 4
- `flush()` / `shutdown()`: No WAL checkpoint needed — just persist fingerprints and drop database handles
- `needs_compaction()` / `compact()`: redb manages its own page reuse; these remain no-ops
- **Backup**: `ReadTransaction::copy_to_file()` provides point-in-time snapshots

#### [DELETE] [sqlite_backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/sqlite_backend.rs)
- Entire file removed. All SQLite-specific code (PRAGMAs, connection pooling, FTS5 table creation, prepared statements) is eliminated.

#### [MODIFY] [sqlite_resolver.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/bridge/sqlite_resolver.rs)
- **Rename** to `resolver_impl.rs` (or keep name, but remove all `rusqlite` imports)
- `SqliteResolver` → rename to `VardaResolver` (it's no longer SQLite-specific)
- Remove `json_to_sqlite_value()` (line 516) — no more rusqlite types
- `filter_by_field_value()` / `filter_by_field_contains()` / `filter_by_field_in()`: These currently use SQL pushdown. Replace with Rust-side iteration over B-tree range cursors with in-memory filtering. The order index (`0x09` prefix keys) already provides efficient range lookups via `prefix()` and `range()` — these continue to work unchanged on redb.
- `filter_via_table_scan_impl()`: Replaced with Rust iteration + JSON deserialization + comparison. No SQL involved.
- `write_term_index()` / `remove_term_index()` / `search_text_bm25()`: **Temporarily stubbed** — these use FTS5 raw SQL. They will be re-implemented in Phase 4 with Tantivy.
- `search_hybrid()`: **Temporarily stubbed** — depends on both FTS5 and sqlite-vec. Re-implemented in Phase 4.
- `search_vectors()`: **Temporarily stubbed** until Phase 4.

#### [MODIFY] [Cargo.toml](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/Cargo.toml)
- Remove: `rusqlite`, `sqlite-vec`
- Add: `redb = "2.x"` (latest stable)

#### [MODIFY] [mod.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/mod.rs)
- Replace `pub mod sqlite_backend;` with `pub mod redb_backend;`

#### KvStore trait implementations
The `auth::state::KvStore` and `permissions::storage::auth_store::KvStore` trait impls (currently on `SqliteTable`) must be re-implemented on `RedbTable`. The trait surface is small: `kv_insert`, `kv_get`, `kv_remove`, `kv_prefix`.

#### [NEW] [migrate_sqlite_to_redb.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/migrate_sqlite_to_redb.rs)
- Offline migration tool: reads all data from SQLite, writes to redb
- Invokable via `vardadb db migrate` CLI command
- Handles: main tables, history tables, system tables, database registry

**Files touched**: ~8  
**Risk**: High — this is the core migration  
**Verification**: Full `cargo test` suite + new storage-level benchmarks (see Verification Plan)

---

### Phase 4: Search & Vector Evolution

Replace SQLite FTS5 with Tantivy and sqlite-vec with usearch. This restores the full-text search and vector search capabilities that were temporarily stubbed in Phase 3.

#### [NEW] [tantivy_search.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/tantivy_search.rs)

Tantivy integration module:

```rust
pub struct SearchEngine {
    /// Per-database Tantivy indexes
    indexes: DashMap<String, tantivy::Index>,
    base_path: PathBuf,
}
```

**Tokenizer pipeline** (replicating FTS5 behavior):
- `fts_data` (porter + unicode61) → Tantivy `TextAnalyzer` with `UnicodeTokenizer` + `LowerCaser` + `Stemmer(English)`
- `fts_term_data` (unicode61 only) → Tantivy `TextAnalyzer` with `UnicodeTokenizer` + `LowerCaser`

**Schema per index**:
```rust
let mut schema_builder = Schema::builder();
schema_builder.add_u64_field("uid", STORED | INDEXED);
schema_builder.add_text_field("field", STRING | STORED);  // exact match on field name
schema_builder.add_text_field("text_content", TEXT | STORED);
```

**BM25**: Tantivy provides BM25 scoring natively via `TopDocs` collector — the entire manual BM25 stats subsystem (0x05/0x06 prefix keys) is eliminated.

**Key methods**:
- `index_document(db_name, uid, field, text, strategy)` — replaces `write_term_index()`
- `remove_document(db_name, uid, field, strategy)` — replaces `remove_term_index()`
- `search_bm25(db_name, query, field, strategy, k)` → `Vec<(u64, f64)>` — replaces `search_text_bm25()`
- `commit(db_name)` — flush pending writes to disk

#### [NEW] [vector_engine.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/vector_engine.rs)

usearch integration module:

```rust
pub struct VectorEngine {
    /// Per-database vector indexes
    indexes: DashMap<String, usearch::Index>,
    base_path: PathBuf,
    dimensions: usize,
}
```

**Key methods**:
- `add_vector(db_name, uid, vector: &[f32])` — HNSW insertion
- `remove_vector(db_name, uid)` — remove by label
- `search(db_name, query: &[f32], k: usize)` → `Vec<(u64, f32)>` — ANN search
- `save(db_name)` / `load(db_name)` — persistence to/from disk
- **f16 quantization**: usearch natively supports `ScalarKind::F16` — configure at index creation time to halve storage

**Vector worker changes**: The existing `SyncSender<(u64, Vec<f64>)>` background thread pattern is preserved, but the worker body changes from SQL INSERT to `VectorEngine::add_vector()`. The f64→f32 conversion remains (or f64→f16 with usearch).

#### [MODIFY] [backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/backend.rs)
- Add `search_engine: Arc<SearchEngine>` and `vector_engine: Arc<VectorEngine>` fields to `Storage`
- Initialize in `Storage::new()`
- `search_vectors()`: Delegate to `VectorEngine::search()`
- `put_vector()`: Delegate to vector worker → `VectorEngine::add_vector()`
- `delete_vector()`: Delegate to `VectorEngine::remove_vector()`
- `flush()`: Call `search_engine.commit()` and `vector_engine.save()` for all databases

#### [MODIFY] [sqlite_resolver.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/bridge/sqlite_resolver.rs)
- `write_term_index()` / `remove_term_index()`: Delegate to `SearchEngine`
- `search_text_bm25()`: Delegate to `SearchEngine::search_bm25()`
- `search_hybrid()`: RRF fusion between `SearchEngine::search_bm25()` and `VectorEngine::search()` — implemented in pure Rust (no SQL CTE)
- Remove all remaining `rusqlite` imports and types

#### [MODIFY] [Cargo.toml](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/Cargo.toml)
- Add: `tantivy = "0.22"`, `usearch = "2.x"`
- Remove: `lru` (replaced by `moka` below for QueryCache; `filter_cache` is removed entirely)

#### [MODIFY] [cache.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/engine/cache.rs)
- Replace `Mutex<LruCache<u64, String>>` with `moka::sync::Cache<u64, String>`
- `moka` provides:
  - Lock-free concurrent reads (no global `Mutex` contention)
  - Built-in TTL and max size bounds
  - Transparent eviction
- `get()` / `put()` become direct `Cache` method calls — no `lock()` needed
- Add `moka = "0.12"` to Cargo.toml

**Files touched**: ~6 new/modified  
**Risk**: Medium — search and vector are well-isolated behind the Resolver trait  
**Verification**: `text_search_test.rs`, `hybrid_search_test.rs`, `vector_*_test.rs`, `multi_vector_test.rs`

---

### Phase 5: Write Path Optimization

#### [NEW] [write_coalescer.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/write_coalescer.rs)

Background write coalescing buffer:

```rust
pub struct WriteCoalescer {
    tx: SyncSender<WriteOp>,
}

enum WriteOp {
    Put { db: String, key: Vec<u8>, value: Vec<u8> },
    PutLww { db: String, uid: u64, predicate: String, value: Vec<u8>, ts: Timestamp },
    Delete { db: String, key: Vec<u8> },
    Flush(oneshot::Sender<()>),
}
```

- Background thread accumulates writes over a configurable window (1–5ms)
- Flushes all accumulated ops in a single `redb::WriteTransaction`
- Pattern is modeled on the existing vector worker's `SyncSender` + background thread
- Critical for high-frequency fingerprint updates and batch mutations

#### [MODIFY] [backend.rs](file:///Volumes/Data/Users/paul/development/src/github/VardaDB/src/storage/backend.rs)
- Add `write_coalescer: WriteCoalescer` field to `Storage`
- Route `put_with_lww()`, `insert()`, `delete_key()` through the coalescer for non-blocking writes
- `flush()` sends a `Flush` op and waits for the oneshot response

**Files touched**: 2  
**Risk**: Medium — write ordering must be carefully preserved  
**Verification**: Existing CRUD tests + new concurrent write benchmarks

---

## Open Questions

> [!IMPORTANT]
> **1. Global allocator choice**: `mimalloc` vs `jemalloc`? Both are excellent. `mimalloc` tends to have lower latency on macOS; `jemalloc` is more battle-tested on Linux. Since you develop on macOS, I'd lean toward `mimalloc` for Phase 1 with a feature flag to switch. Thoughts?

> [!IMPORTANT]
> **2. Data migration strategy**: Should we build:
> - (a) An offline `vardadb db migrate` command that converts SQLite → redb (one-time, clean break), or
> - (b) A dual-backend mode where both SQLite and redb coexist temporarily?
>
> Option (a) is simpler and the plan currently assumes it. Option (b) adds significant complexity but allows gradual rollout.

> [!IMPORTANT]
> **3. Vector search during Phase 3 gap**: Between removing sqlite-vec (Phase 3) and adding usearch (Phase 4), vector search will be unavailable. Is this acceptable, or should Phases 3 and 4 be merged? Merging them increases the blast radius of a single phase significantly.

> [!IMPORTANT]
> **4. FTS search during Phase 3 gap**: Same question for full-text search. Tantivy replaces FTS5 in Phase 4. During Phase 3, text search will be stubbed. Is an interim brute-force text search acceptable?

> [!IMPORTANT]
> **5. Tantivy per-database or global?**: Should each logical database get its own Tantivy index directory, or should we use a single global index with a `db_name` field for filtering? Per-database is simpler for isolation and deletion; global is slightly more efficient for cross-database search (if that's ever needed).

---

## Verification Plan

### Automated Tests

**Phase 1**:
```bash
cargo test                                    # all existing tests pass
cargo test --test filter_order_index_test     # filter pushdown unchanged
cargo test --test text_search_test            # FTS unchanged
```

**Phase 2**:
```bash
cargo test                                    # iterator API is transparent
```

**Phase 3**:
```bash
cargo test -- --skip search --skip vector --skip hybrid  # skip stubbed features
cargo test --test e2e_storage_test            # core KV operations
cargo test --test crud_test                   # CRUD via GraphQL
cargo test --test filter_order_index_test     # order index scans
cargo test --test pagination_test             # prefix scan pagination
```

**Phase 4**:
```bash
cargo test                                    # all tests pass, including search/vector
cargo test --test text_search_test
cargo test --test hybrid_search_test
cargo test --test vector_api_test
cargo test --test vector_integration_test
cargo test --test multi_vector_test
```

**Phase 5**:
```bash
cargo test                                    # write coalescing is transparent
```

### New Storage-Level Benchmarks

The existing `million_todos` benchmark only measures JSON serialization. New criterion benchmarks will be added:

```bash
cargo bench --bench storage_bench
```

Benchmarks to write:
1. **Prefix scan throughput**: 100k keys with shared prefix, measure scan time
2. **Point read latency**: Random key lookups (p50/p99)
3. **Fingerprint rebuild time**: Full history table scan + hash
4. **Concurrent read latency**: 8 threads doing parallel prefix scans
5. **Write batch throughput**: 10k LWW upserts in a single transaction

### Manual Verification
- Verify the frontend (`new_gospel`) continues to work end-to-end after each phase
- Run `vardadb cli` REPL to verify interactive operations
- Test `vardadb db create/list/delete` management commands
