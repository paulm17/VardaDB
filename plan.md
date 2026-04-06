# VardaDB Storage Migration & Performance Plan

This document outlines the strategy for migrating VardaDB's storage entirely off SQLite — using **redb** for the KV layer, **Tantivy** for full-text search, and **usearch** for vector search — alongside a series of engine-level performance optimizations.

## 1. Objectives
*   **Performance**: Reduce latency for prefix scans (expected 7-8x improvement based on benchmarks).
*   **Efficiency**: Implement zero-copy reads to eliminate heavy `Vec<u8>` allocations.
*   **Reliability**: Replace the fragile manual connection pooling with a robust MVCC-based engine.
*   **Modernization**: Adopt a pure-Rust storage stack, fully replacing SQLite across KV, FTS, and vector workloads.

---

## 2. Technical Debt & Current Bottlenecks

### 🛑 Storage Layer Issues
*   **Materialized Returns**: `prefix()`, `range()`, and `iter()` currently return `Vec<(Vec<u8>, Vec<u8>)>`. For large tables (e.g., history), this causes massive heap churn and memory spikes.
*   **Fragile Pooling**: `get_reader()` and `return_reader()` pattern is prone to connection leaks if a panic occurs between calls.
*   **SQLite-as-KV Overhead**: SQLite is row-oriented; using it for prefix scans on BLOB keys involves significant engine overhead compared to a native B-tree KV store like `redb`.
*   **Vector Bottleneck**: Current f64 → f32 conversion and `unsafe` `slice::from_raw_parts` reinterpretation in the background worker is functional but could be optimized with f16 support.

### 🛑 Engine Issues
*   **Hashing**: The per-item `hash_item` function uses FNV-1a, which has poor collision properties for general use. Note: the XOR *accumulation* in `update_history_hash` is intentional (set-reconciliation / IBLT semantics) and should be preserved.
*   **Locking**: `std::sync::Mutex` is used in `SqliteBackend` (reader pool, writer), `RequestCache`, and `QueryCache`. `parking_lot::Mutex` is already a dependency (used in observability) but not applied to the hot paths.
*   **Allocation**: High frequency of small allocations in the hot path. No global allocator override is set.
*   **QueryCache key hashing**: `QueryCache::hash_query` uses `std::collections::hash_map::DefaultHasher`, which is slower than purpose-built non-cryptographic hashers.

### ⚠️ SQLite-Specific Optimisations (removed by migration, no action needed)
*   **Prepared statements**: `prepare_cached` is a rusqlite concept — statement compilation cached per connection. redb has no SQL layer, so this ceases to exist. No equivalent needed.
*   **WAL & PRAGMA tuning**: All five PRAGMAs (`journal_mode`, `synchronous`, `mmap_size`, `cache_size`, `temp_store`) are SQLite-specific. redb manages its own durability and page cache internally. These go away entirely.

### ⚠️ Requires Redesign During Migration
*   **Filter cache**: `SqliteTable` carries an `Arc<Mutex<LruCache<FilterCacheKey, Vec<u64>>>>` keyed on `(type_name, field_name, op, rusqlite::types::Value)` — caching results of SQL `WHERE` filter pushdown. This is tightly coupled to the SQL query model and cannot be carried over. The equivalent for redb is filtering over B-tree range iterators in Rust; the cache must be redesigned around that access pattern as part of Phase 3.

### ✅ Backend-Agnostic (unaffected by migration)
*   **`QueryCache`**: Caches serialised GraphQL JSON responses at the engine layer, keyed by query hash. Completely independent of the storage backend. Remains valid; being further improved with `moka` in Phase 4.

---

## 3. Migration Roadmap

### Phase 1: Zero-Hanging Fruit (Immediate Optimizations)
These changes are low-risk and provide immediate wins:
*   [ ] **Global Allocator**: Switch to `mimalloc` or `jemalloc` in `main.rs`.
*   [ ] **Locking**: Replace `std::sync::Mutex` with `parking_lot::Mutex` in `SqliteBackend` (reader pool, writer) and `QueryCache`. `parking_lot` is already a dependency — extend its use.
*   [ ] **Hashing (item hash)**: Replace the FNV-1a `hash_item` function in `backend.rs` with `xxhash-rust`. The XOR accumulation in `update_history_hash` is intentional and must be kept.
*   [ ] **QueryCache key hashing**: Replace `DefaultHasher` in `QueryCache::hash_query` with `ahash::AHasher` (faster non-cryptographic hashing, one-line change).
*   [ ] **DashMap**: `RequestCache` is per-request and short-lived — `DashMap` provides no benefit there. `Storage.backends` already uses `DashMap`. No changes needed for `RequestCache`.

### Phase 2: Interface Refactoring (Streaming API)
Before swapping the backend, the trait/interface must support lazy iteration:
*   [ ] Modify `SqliteTable` methods (`prefix`, `range`, `iter`) to return `impl Iterator` or a custom `BoxStream`.
*   [ ] Update `Storage::get_history_range` and fingerprint rebuild logic to use these streaming iterators.
*   [ ] Replace `get_reader/return_reader` with a guard-based pattern (RAII) or move to a model where the backend handles concurrency (like `redb`).

### Phase 3: redb Integration (The Core Migration)

The entire `rusqlite` interface layer is replaced. `rusqlite = { version = "0.31", features = ["bundled", "load_extension"] }` compiles ~200k lines of SQLite C into the binary via FFI. redb is pure Rust with no FFI boundary. The `load_extension` feature exists solely for `sqlite-vec` and is removed alongside the vector migration in Phase 4.

The API mapping is direct:

| rusqlite (current) | redb (replacement) |
|---|---|
| `Connection` (pooled, max 8) | `ReadTransaction` / `WriteTransaction` — open anywhere, no pool needed |
| `prepare_cached(&sql)` + `params![]` | Direct typed table ops — no SQL, no statement compilation overhead |
| `query_map(\|row\| row.get::<_, Vec<u8>>(0))` | `table.range(lower..upper)?` — native B-tree cursor, lazy by default |
| `rusqlite::types::Value` type conversions | Raw `&[u8]` — no type mapping layer at all |
| `get_reader()` / `return_reader()` | Gone — `ReadTransaction` is cheap to open, MVCC handles concurrency |
| `Mutex<Vec<Connection>>` reader pool | Gone — any thread opens a read transaction directly |

*   [ ] **Remove `rusqlite`**: Replace `SqliteBackend` and `SqliteTable` with `RedbBackend`. The `rusqlite` crate and its bundled SQLite C library are dropped entirely.
*   [ ] **Multi-database layout**: One `redb` file per logical database, mirroring the current multi-`.db` setup. `Storage.backends` becomes `DashMap<String, Arc<redb::Database>>` — a near 1:1 mapping.
*   [ ] **Zero-Copy Reads**: redb returns `AccessGuard<&[u8]>` — a direct reference into mmap'd pages. Replaces `row.get::<_, Vec<u8>>(0)?` which allocates a new `Vec<u8>` on every read.
*   [ ] **Typed Tables**: `TableDefinition<&[u8], &[u8]>` enforces schema at compile time. Replaces stringly-typed SQL and `rusqlite::types::Value` conversions.
*   [ ] **Backup / Restore**: `ReadTransaction::copy_to_file()` gives point-in-time snapshots without blocking writers. Replaces the current `PRAGMA wal_checkpoint(TRUNCATE)` shutdown-only approach.

### Phase 4: Search & Vector Evolution
*   [ ] **Tantivy Integration**: Replace SQLite FTS5 with `Tantivy`. SQLite is being fully removed so this is required, not optional. The current setup has two FTS tables (porter-stemmed + unicode61) — Tantivy's tokeniser pipeline will need to replicate this behaviour. Tantivy also replaces the entire manual BM25 stats subsystem (0x05/0x06 prefix keys in the KV store).
*   [ ] **Vector Engine**: Replace `sqlite-vec` with `usearch` (pure Rust, HNSW, supports f32/f16/binary quantisation, serialisable to disk). The existing async channel-based vector worker makes the backend swap relatively contained.
*   [ ] **Quantized Vectors**: Implement `f16` storage for embeddings to halve the vector footprint alongside the `usearch` migration. The current `unsafe slice::from_raw_parts` pattern is already isolated in the vector worker and `sqlite_resolver.rs`.
*   [ ] **Query Cache**: Replace `Mutex<LruCache<u64, String>>` in `QueryCache` with `moka`. `QueryCache` is shared across all requests; the global `Mutex` is contended on every cache hit under concurrent GraphQL load. `moka` provides concurrent reads without a global lock, plus TTL and size bounds.

### Phase 5: Write Path Optimization
*   [ ] **Coalescing Write Buffer**: Implement a background channel that accumulates writes over a 1–5ms window and flushes them in a single `redb` write transaction. This is critical for high-frequency fingerprint updates. The vector worker's existing `SyncSender` + background thread pattern is a direct template for this.

---

## 4. Verification Plan

### Benchmarks
*   **Note**: The existing `million_todos` criterion bench only measures data generation + JSON serialization — it makes no storage calls and will not reflect redb integration gains. New storage-level benchmarks must be written: e.g., prefix scan throughput, fingerprint rebuild time, and concurrent read latency.
*   Measure memory usage during full history scans (fingerprint rebuild).

### Correctness
*   Run the existing `test_framework` and `tests/` suite.


