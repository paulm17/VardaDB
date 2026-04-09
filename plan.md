# VardaDB Search Engine Migration Plan

## Overview

Replace SQLite FTS5 and sqlite-vec with Tantivy (BM25 full-text search) and usearch (HNSW vector search) from the VardaDB_redb repository. SQLite remains as the KV store. This migration brings 11 progressive feature stages, each mirroring the corresponding VardaDB_redb branch.

**Prerequisite (Stage 0)**: Before starting Stage 01, we must first port the base `SearchEngine` and `VectorEngine` structs and integrate them into the existing Storage/SqliteResolver architecture. This is the foundation all 11 stages build on.

### Deprecated system

Much of this work has already been completed.  You can view this in a deprecated version of VardaDB:  /Volumes/Data/Users/paul/development/src/github/VardaDB_redb

You can also view each milestone that was done to compare code:

https://github.com/paulm17/VardaDB/tree/01-fuzzy-matching
https://github.com/paulm17/VardaDB/tree/02-phrase-queries
https://github.com/paulm17/VardaDB/tree/03-field-boosting
https://github.com/paulm17/VardaDB/tree/04-rrf-weighting
https://github.com/paulm17/VardaDB/tree/05-highlighting.md
https://github.com/paulm17/VardaDB/tree/06-bm25-stats
https://github.com/paulm17/VardaDB/tree/07-tantivy-commits
https://github.com/paulm17/VardaDB/tree/08-trigram-index
https://github.com/paulm17/VardaDB/tree/09-faceted-search
https://github.com/paulm17/VardaDB/tree/10-geo-spatial
https://github.com/paulm17/VardaDB/tree/11-resolve-list-hnsw

### What is being replaced

| Component | Current (SQLite) | Replacement |
|-----------|-----------------|-------------|
| BM25 text search | FTS5 virtual tables (`fts_data`, `fts_term_data`) | Tantivy index (`SearchEngine`) |
| Vector search | sqlite-vec virtual table (`vec_data`) | usearch HNSW (`VectorEngine`) |
| Hybrid search | SQL CTE with FULL OUTER JOIN | Rust RRF fusion in memory |
| Vector persistence | SQLite WAL | usearch `.usearch` files |
| Text index durability | SQLite transactional | Tantivy commit on write |

### Files created across all stages

- `src/storage/tantivy_search.rs` — New: Tantivy SearchEngine (from VardaDB_redb)
- `src/storage/vector_engine.rs` — New: usearch VectorEngine (from VardaDB_redb)
- `src/storage/geohash.rs` — New: Geohash encoding/decoding (Stage 10)

### Files modified across all stages

- `Cargo.toml` — Add `tantivy`, `usearch`, `xxhash-rust` dependencies; remove `sqlite-vec`
- `src/storage/mod.rs` — Register new modules
- `src/storage/backend.rs` — Replace `vec_data`/FTS5 with `SearchEngine` + `VectorEngine`; remove sqlite-vec worker
- `src/storage/sqlite_backend.rs` — Remove `create_native_search_tables()`, sqlite-vec auto-extension
- `src/bridge/sqlite_resolver.rs` — Replace all FTS5/sqlite-vec SQL with Tantivy/usearch API calls
- `src/engine/schema.rs` — Add new directives (`@facet`, `@search(by: [trigram])`, etc.)

### New Cargo dependencies

```toml
tantivy = "0.22.1"
usearch = "2.1"
xxhash-rust = { version = "0.8", features = ["xxh3"] }
```

### Removed Cargo dependencies

```toml
sqlite-vec = "0.1"  # replaced by usearch
```

---

## Stage 0: Foundation — Port SearchEngine + VectorEngine

### Why

All 11 feature stages depend on Tantivy and usearch being integrated into the codebase. This stage establishes the base by porting the `SearchEngine` and `VectorEngine` structs from VardaDB_redb and wiring them into `Storage` and `SqliteResolver`, replacing FTS5 and sqlite-vec entirely.

### Existing code being replaced

1. **`src/storage/sqlite_backend.rs`** (lines 44-53): `sqlite3_auto_extension` for sqlite-vec
2. **`src/storage/sqlite_backend.rs`** (lines 92-104): `create_native_search_tables()` — creates `fts_data`, `fts_term_data`, `vec_data` virtual tables
3. **`src/storage/backend.rs`** (lines 63): `vector_tx: SyncSender<(u64, Vec<f64>)>` — sqlite-vec background worker channel
4. **`src/storage/backend.rs`** (lines 138-160): Vector background worker thread that writes to `vec_data`
5. **`src/storage/backend.rs`** (lines 740-790): `put_vector()`, `delete_vector()`, `search_vectors()` — sqlite-vec SQL operations
6. **`src/bridge/sqlite_resolver.rs`** (lines 863-925): `write_term_index()`, `remove_term_index()` — FTS5 INSERT/DELETE SQL
7. **`src/bridge/sqlite_resolver.rs`** (lines 928-991): `search_text_bm25()` — FTS5 `bm25()` SQL query
8. **`src/bridge/sqlite_resolver.rs`** (lines 993-1080): `search_hybrid()` — SQL CTE FULL OUTER JOIN RRF

### New code to write

1. **`src/storage/tantivy_search.rs`** — Copy `SearchEngine` from VardaDB_redb `src/storage/tantivy_search.rs` (main branch, 424 lines). This provides:
   - `SearchEngine::new(base_path)` — creates per-database Tantivy indexes at `{base_path}/{db_name}_tantivy/`
   - `SearchEngine::index_document(db_name, uid, field, text)` — indexes into both `term_content` and `fulltext_content` fields
   - `SearchEngine::remove_document(db_name, uid, field)` — per-field deletion via composite `doc_id = xxh3(uid, field)`
   - `SearchEngine::search_bm25(db_name, query, field, strategy, k, require_all)` — BM25 ranked search with AND/OR semantics
   - `SearchEngine::commit(db_name)` / `SearchEngine::commit_all()` — durability
   - Two tokenizers: `term_tokenizer` (lowercase only), `fulltext_tokenizer` (lowercase + Porter stemmer)

2. **`src/storage/vector_engine.rs`** — Copy `VectorEngine` from VardaDB_redb `src/storage/vector_engine.rs` (236 lines). This provides:
   - `VectorEngine::new(base_path)` — creates per-database usearch indexes at `{base_path}/{db_name}_vectors.usearch`
   - `VectorEngine::add_vector(db_name, uid, &[f32])` — lazy dimension detection, f16 quantization, cosine similarity
   - `VectorEngine::remove_vector(db_name, uid)` — delete vector
   - `VectorEngine::search(db_name, &[f32], k)` — returns `(uid, cosine_distance)` pairs
   - `VectorEngine::save(db_name)` / `VectorEngine::save_all()` — persist to disk
   - Config: connectivity=16, expansion_add=128, expansion_search=64, ScalarKind::F16

3. **`src/storage/backend.rs`** — Modify `Storage` struct:
   - Add `pub search_engine: SearchEngine` and `pub vector_engine: VectorEngine` fields
   - Remove `vector_tx: SyncSender` field
   - Remove sqlite-vec background worker thread
   - `put_vector()` → delegates to `self.vector_engine.add_vector()`
   - `delete_vector()` → delegates to `self.vector_engine.remove_vector()`
   - `search_vectors()` → delegates to `self.vector_engine.search()`
   - `flush()` → add `self.search_engine.commit_all()` and `self.vector_engine.save_all()`
   - Remove call to `create_native_search_tables()` from `initialize_database_tables()`

4. **`src/storage/sqlite_backend.rs`** — Remove:
   - `sqlite3_auto_extension` / `sqlite3_reset_auto_extension` for sqlite-vec
   - `create_native_search_tables()` method entirely

5. **`src/storage/mod.rs`** — Add `pub mod tantivy_search;` and `pub mod vector_engine;`

6. **`Cargo.toml`** — Add `tantivy = "0.22.1"`, `usearch = "2.1"`, `xxhash-rust = { version = "0.8", features = ["xxh3"] }`; remove `sqlite-vec = "0.1"`

7. **`src/bridge/sqlite_resolver.rs`** — Rewrite search methods:
   - `write_term_index()` → `self.storage.search_engine.index_document()`
   - `remove_term_index()` → `self.storage.search_engine.remove_document()`
   - `search_text_bm25()` → `self.storage.search_engine.search_bm25()`
   - `search_hybrid()` → Rust code: call `search_bm25()` + `vector_engine.search()`, then fuse in memory with RRF k=60

### Tests

```rust
// tests/stage0_search_engine_test.rs
// 1. Test index_document + search_bm25 with "term" strategy returns correct UID with BM25 score
// 2. Test index_document + search_bm25 with "fulltext" strategy (stemming)
// 3. Test remove_document removes document from search results
// 4. Test require_all=true (AND) vs require_all=false (OR) semantics
// 5. Test commit persists data (create index, commit, create new SearchEngine instance, search works)

// tests/stage0_vector_engine_test.rs
// 1. Test add_vector + search returns nearest neighbors by cosine distance
// 2. Test remove_vector removes vector from search results
// 3. Test dimension mismatch is handled gracefully (skip + warn)
// 4. Test save/load round-trip persists vectors across instances
// 5. Test f16 quantization: search accuracy within acceptable bounds vs brute-force

// tests/stage0_hybrid_search_test.rs
// 1. Test search_hybrid fuses BM25 + vector results via RRF
// 2. Test documents appearing in both result sets get higher RRF score
// 3. Test text-only results and vector-only results both contribute
// 4. Test k parameter limits final output count

// tests/stage0_end_to_end_test.rs
// 1. Full GraphQL flow: create node with @search field, query with allofterms, verify results
// 2. Full GraphQL flow: create node with @vector field, query with nearVector, verify results
// 3. Full GraphQL flow: hybrid search query returns fused results
```

---

## Stage 01: Fuzzy Matching

### Why

FTS5 has no built-in fuzzy matching. Tantivy's `FuzzyTermQuery` supports Levenshtein distance-based matching (distance 0-2), enabling "did you mean?" style queries where minor typos still return results.

### Existing code being replaced

1. **`src/bridge/sqlite_resolver.rs`** — `search_text_bm25()` method (no fuzzy parameter exists currently)
2. **`src/bridge/sqlite_resolver.rs`** — Filter handling in `apply_text_filter()` (no "fuzzy" filter type)

### New code to write

1. **`src/storage/tantivy_search.rs`** — Modify `search_bm25()`:
   - Add `fuzzy_distance: Option<u8>` parameter
   - Import `tantivy::query::FuzzyTermQuery`
   - When `fuzzy_distance` is `Some(d)`, use `FuzzyTermQuery::new(term, d, true)` instead of `TermQuery`
   - Applies to both AND and OR clause branches

2. **`src/bridge/sqlite_resolver.rs`** — Modify `search_text_bm25()`:
   - Add `fuzzy_distance: Option<u8>` parameter, pass through to `SearchEngine`

3. **`src/bridge/sqlite_resolver.rs`** — Modify `apply_text_filter()`:
   - Add "fuzzy" to recognized filter types
   - Handle `fuzzy: { terms: "...", distance: N }` filter object
   - Pass distance to `search_text_bm25()`

4. **`src/engine/schema.rs`** — No schema changes needed (fuzzy is a query-time parameter, not a schema directive)

### Tests

```rust
// tests/fuzzy_search_test.rs
// 1. Index "database" — fuzzy search for "databse" (distance=1) returns match
// 2. Index "database" — fuzzy search for "dtbase" (distance=2) returns match
// 3. Index "database" — exact search (distance=0) does NOT match "databse"
// 4. Fuzzy with AND semantics: index "machine learning", query "machne lerning" (distance=1) returns match
// 5. Fuzzy with OR semantics: index "database", query "databse OR computr" returns match for "database"
// 6. Full GraphQL: create Product with name "database", query with fuzzy filter { terms: "databse", distance: 1 }
```

---

## Stage 02: Phrase Queries

### Why

FTS5 supports basic phrase matching via quoted strings in the MATCH clause, but Tantivy's `PhraseQuery` provides explicit programmatic control with configurable slop for proximity matching. This enables exact phrase matching and near-proximity queries ("find documents where 'machine' appears within 2 words of 'learning'").

### Existing code being replaced

1. **`src/storage/tantivy_search.rs`** — `search_bm25()` (no phrase support currently, even after Stage 01)

### New code to write

1. **`src/storage/tantivy_search.rs`** — Modify `search_bm25()`:
   - Add `phrase_slop: Option<u32>` parameter
   - Import `tantivy::query::PhraseQuery`
   - Detect quoted queries: if `query_text` starts and ends with `"`, extract inner text
   - For phrase queries: tokenize inner text, create `PhraseQuery::new(terms)`, optionally set slop
   - For non-phrase queries: existing term/fuzzy logic

2. **`src/bridge/sqlite_resolver.rs`** — Modify `search_text_bm25()`:
   - Add `phrase_slop: Option<u32>` parameter, pass through to `SearchEngine`

3. **`src/bridge/sqlite_resolver.rs`** — Modify filter handling:
   - Recognize quoted strings in filter values as phrase queries
   - Support `phrase: { terms: "...", slop: N }` filter syntax

### Tests

```rust
// tests/phrase_search_test.rs
// 1. Index "the quick brown fox" — phrase query "quick brown" returns match
// 2. Index "the quick brown fox" — phrase query "quick fox" does NOT match (non-adjacent)
// 3. Index "the quick brown fox" — phrase query "quick fox" with slop=1 returns match
// 4. Index "the quick brown fox" — phrase query "brown quick" does NOT match (wrong order)
// 5. Phrase query with stemming strategy: "running fast" matches "run fast" with fulltext
// 6. Full GraphQL: create Article with content "machine learning algorithms", phrase query "machine learning" matches
// 7. Full GraphQL: phrase query "learning machine" does NOT match (wrong order)
// 8. Phrase query combined with fuzzy: "machne lerning" with distance=1 matches "machine learning"
```

---

## Stage 03: Field Boosting

### Why

BM25 scoring treats all fields equally. Field boosting allows specifying that a match in the `title` field is worth 3x more than a match in the `description` field, dramatically improving search relevance. This is impossible with FTS5's flat `bm25()` function. Tantivy's `BoostQuery` wrapper enables this.

### Existing code being replaced

1. **`src/bridge/sqlite_resolver.rs`** — `search_text_bm25()` (single-field search only)

### New code to write

1. **`src/storage/tantivy_search.rs`** — Add:
   - `FieldBoost` struct: `{ field: String, boost: f32 }`
   - Import `tantivy::query::BoostQuery`
   - New method `search_bm25_multi()`:
     - Takes `fields: &[FieldBoost]` instead of single field
     - For each field: builds field filter + content query, wraps with `BoostQuery::new(query, boost)`
     - Combines all field queries as SHOULD clauses in a top-level `BooleanQuery`
     - Supports fuzzy, phrase, AND/OR for each field
   - Keep existing `search_bm25()` as a single-field convenience wrapper

2. **`src/bridge/sqlite_resolver.rs`** — Add:
   - `search_text_bm25_multi()` method that wraps `SearchEngine::search_bm25_multi()`
   - Wire into GraphQL filter handling when multiple search fields with boosts are specified

3. **`src/engine/schema.rs`** — Add:
   - Parse `@search(by: [...], boost: N)` directive parameter
   - Store boost values in `TypeMetadata::search_fields` (change from `Vec<String>` to `Vec<SearchFieldConfig>` with field name + boost)

### Tests

```rust
// tests/field_boost_test.rs
// 1. Create nodes with title "Rust Programming" and description "A book about the Rust language"
//    Search for "Rust" with title boost=3.0, description boost=1.0
//    Verify title match scores higher than description match
// 2. Same documents, boost title=1.0, description=3.0
//    Verify description match scores higher
// 3. Multi-field search with no boost (all 1.0) returns same scores as single-field
// 4. Multi-field combined with fuzzy matching works correctly
// 5. Multi-field combined with phrase queries works correctly
// 6. Full GraphQL: schema with @search(by: [term], boost: 2.0) on title field
```

---

## Stage 04: RRF Weighting

### Why

The current hybrid search uses equal weights (50/50) for BM25 and vector contributions. Configurable RRF weighting (alpha parameter) allows tuning: `alpha=0.7` favors vector results, `alpha=0.3` favors text results. This is critical for different use cases — semantic search benefits from higher vector weight, keyword search from higher text weight.

### Existing code being replaced

1. **`src/bridge/sqlite_resolver.rs`** — `search_hybrid()` (hardcoded 50/50 weighting, equal `1/(60+rank)` for both)

### New code to write

1. **`src/bridge/sqlite_resolver.rs`** — Modify `search_hybrid()`:
   - Add `alpha: Option<f32>` parameter (default 0.5 when None)
   - `text_weight = 1.0 - alpha`, `vector_weight = alpha`
   - RRF formula: `score += text_weight / (60 + rank + 1)` for text results
   - RRF formula: `score += vector_weight / (60 + rank + 1)` for vector results

2. **`src/bridge/sqlite_resolver.rs`** — Modify `scan_nodes()` and aggregate query paths:
   - Accept `rrf_alpha` parameter from GraphQL query arguments
   - Pass through to `search_hybrid()`

3. **`src/engine/schema.rs`** — Parse `rrfAlpha` query argument on root query fields

### Tests

```rust
// tests/hybrid_search_test.rs
// 1. Hybrid search with alpha=0.0 (all text) — vector results contribute nothing
// 2. Hybrid search with alpha=1.0 (all vector) — text results contribute nothing
// 3. Hybrid search with alpha=0.5 (balanced) — equal contribution
// 4. Hybrid search with alpha=0.7 — vector results ranked higher than text for same document
// 5. Verify alpha=0.0 produces same results as pure text search
// 6. Verify alpha=1.0 produces same results as pure vector search
// 7. Full GraphQL: query with nearVector and rrfAlpha parameter
```

---

## Stage 05: Highlighting Specification

### Why

Search highlighting (returning matched text fragments with `<em>` tags) improves UX by showing users exactly why a result matched. This stage defines the specification and API contract. Tantivy's `SnippetGenerator` provides this capability.

### Existing code being replaced

None — this is a design spec only, no existing code to replace.

### New code to write

1. **Design document** — Define:
   - GraphQL `@search(highlight: true)` directive
   - Return type extension: search results include `highlights: [{ field, snippet, fragments }]`
   - Snippet configuration: max snippet length, fragment count, tag names

2. **`src/storage/tantivy_search.rs`** — Add:
   - `highlight()` method using Tantivy's `SnippetGenerator`
   - Takes `db_name, query_text, field, strategy, doc_text, max_chars`
   - Returns highlighted snippet string with `<b>` tags

### Tests

```rust
// tests/highlight_test.rs
// 1. Index "the quick brown fox jumps over the lazy dog"
//    Highlight query "quick brown" → returns snippet with <b>quick brown</b>
// 2. Highlight with max_chars limit truncates snippet
// 3. Highlight returns empty string when query doesn't match document
// 4. Highlight works with stemming strategy (query "running" highlights "run")
```

---

## Stage 06: BM25 Stats

### Why

Proper BM25 scoring requires corpus-level statistics: document frequency (how many documents contain a term), average document length, and total document count. Currently these are implicit inside the search engine. Exposing them enables debugging search quality, building custom ranking functions, and providing search UI metadata ("1,234 results found").

### Existing code being replaced

1. **`src/storage/tantivy_search.rs`** — Search stats are internal to Tantivy, not exposed

### New code to write

1. **`src/storage/tantivy_search.rs`** — Add methods:
   - `get_doc_count(db_name, field) -> u64` — number of indexed documents for a field
   - `get_avg_doc_length(db_name, field) -> f64` — average term count per document
   - `get_term_doc_frequency(db_name, field, term) -> u64` — how many docs contain a term
   - Use Tantivy's `Searcher::doc_freq()` and segment readers

2. **`src/bridge/sqlite_resolver.rs`** — Add:
   - Wire stats into GraphQL `aggregate{Type}` query to return search metadata

3. **`src/engine/schema.rs`** — Add:
   - `searchStats` field on aggregate query type

### Tests

```rust
// tests/index_stats_test.rs
// 1. Index 10 documents — get_doc_count returns 10
// 2. Index documents with varying term counts — get_avg_doc_length returns correct average
// 3. Index "apple" in 3 of 10 docs — get_term_doc_frequency("apple") returns 3
// 4. After remove_document — doc count decreases
// 5. Empty index — all stats return 0
```

---

## Stage 07: Tantivy Commits / Durability

### Why

By default Tantivy buffers writes in memory. If the process crashes before a commit, indexed data is lost. This stage ensures every write operation commits immediately to disk, making Tantivy's durability match SQLite's WAL guarantees. In the VardaDB_redb repo, this was added alongside redb durability; here we only need the Tantivy side.

### Existing code being replaced

1. **`src/storage/tantivy_search.rs`** — `index_document()` and `remove_document()` may buffer without committing (depends on Stage 0 implementation)

### New code to write

1. **`src/storage/tantivy_search.rs`** — Ensure:
   - `index_document()` calls `writer.commit()` after `add_document()`
   - `remove_document()` calls `writer.commit()` after `delete_term()`
   - Both operations block until data is persisted to the Tantivy index directory
   - Add `flush_deletes()` helper that commits pending deletes before any add

2. **`src/storage/backend.rs`** — Modify `flush()`:
   - Call `self.search_engine.commit_all()` before vector engine save
   - Ensure ordering: tantivy commit → usearch save → sqlite WAL checkpoint

3. **`src/storage/backend.rs`** — Crash safety:
   - Register shutdown hook to call `commit_all()` and `save_all()` on SIGINT/SIGTERM

### Tests

```rust
// tests/tantivy_durability_test.rs
// 1. Index document, commit, kill thread, create new SearchEngine — document is searchable
// 2. Index document WITHOUT commit, create new SearchEngine — document is NOT searchable
// 3. Index then remove then commit — document removed after reload
// 4. Rapid index/remove/index cycle — final state is correct after commit
// 5. Commit with 1000 documents — all searchable after reload
```

---

## Stage 08: Trigram Index

### Why

Substring/contains queries (`WHERE text LIKE '%substring%'`) require full table scans. A trigram index breaks text into 3-character overlapping sequences, enabling efficient `contains` queries via Tantivy. The current codebase handles `contains` via `json_extract() LIKE` table scans (sqlite_backend.rs:896-943). Tantivy's tokenizers can be extended with a trigram tokenizer for efficient substring matching.

### Existing code being replaced

1. **`src/storage/sqlite_backend.rs`** (lines 896-943): `filter_by_field_contains()` — SQL LIKE table scan
2. **`src/bridge/sqlite_resolver.rs`** — `contains` filter handling via SQL pushdown

### New code to write

1. **`src/storage/tantivy_search.rs`** — Add:
   - Register a `trigram_tokenizer` in `get_or_create()`: splits text into overlapping 3-char tokens
   - Add `trigram_content` field to Tantivy schema (indexed with trigram tokenizer)
   - `index_document()` also indexes into `trigram_content` when `@search(by: [trigram])` is declared
   - New method `search_contains(db_name, field, substring, k) -> Vec<(u64, f64)>` — tokenizes substring into trigrams, searches

2. **`src/bridge/sqlite_resolver.rs`** — Modify:
   - `write_term_index()` — also index into trigram field when strategy is "trigram"
   - Filter handling: `contains` filter on `@search(by: [trigram])` fields delegates to `search_contains()`
   - Falls back to SQL LIKE for non-indexed fields

3. **`src/engine/schema.rs`** — Modify:
   - Parse `@search(by: [trigram])` in the `by` parameter
   - Store trigram-indexed fields separately in `TypeMetadata`

### Tests

```rust
// tests/trigram_test.rs
// 1. Index "Hello World" with trigram — search_contains("llo") returns match
// 2. Index "Hello World" — search_contains("xyz") returns no match
// 3. Index "database" — search_contains("tab") returns match (from "database")
// 4. Trigram index coexists with term/fulltext indexes on same field
// 5. Update document — old trigrams removed, new ones indexed
// 6. Delete document — trigrams removed from index
// 7. Full GraphQL: @search(by: [trigram]) field with contains filter
```

---

## Stage 09: Faceted Search

### Why

Faceted search enables category/brand/price-range filtering with counts (e.g., "Electronics (45), Books (23), Clothing (12)"). This is essential for e-commerce and catalog UIs. FTS5 has no faceting capability. Tantivy's `FacetCollector` provides hierarchical facet counting natively.

### Existing code being replaced

None — this is a new feature with no existing VardaDB equivalent.

### New code to write

1. **`src/storage/tantivy_search.rs`** — Add:
   - Add `facet_field` to Tantivy schema: `sb.add_facet_field("facet_values", FacetOptions::default())`
   - Import `tantivy::collector::{FacetCollector, FacetCounts}` and `tantivy::schema::{Facet, FacetOptions}`
   - `index_facet(db_name, uid, field, value)` — stores facet as `/field_name/value` path
   - `remove_facet(db_name, uid, field)` — removes facet documents
   - `get_facet_counts(db_name, field) -> Vec<(String, u64)>` — returns value counts for a field

2. **`src/bridge/sqlite_resolver.rs`** — Modify:
   - `create_node_internal()` — after writing data, call `index_facet()` for `@facet` fields
   - `update_node()` — re-index facets when facet fields change
   - `delete_node()` — call `remove_facet()` for all facet fields
   - Add `facets(field: "...")` query parameter to root queries

3. **`src/engine/schema.rs`** — Modify:
   - Add `facet_fields: Vec<String>` to `TypeMetadata`
   - Parse `@facet` directive on fields
   - Generate `facets` field on aggregate query type

### Tests

```rust
// tests/facet_test.rs
// 1. Create 5 Products with category "Electronics", 3 with "Books", 2 with "Clothing"
//    get_facet_counts("category") → [("Electronics", 5), ("Books", 3), ("Clothing", 2)]
// 2. Add product — count increments
// 3. Delete product — count decrements
// 4. Update product category — old category decrements, new category increments
// 5. Facet on field with no values → empty result
// 6. Full GraphQL: query { aggregateProduct { facets { field, value, count } } }
```

---

## Stage 10: Geo-Spatial

### Why

Location-aware queries ("find all restaurants within 5km of this point") require geospatial indexing. This is implemented using geohash encoding, which maps lat/lng to sortable string prefixes. Nearby points share geohash prefixes, enabling efficient range scans. The `geo` crate is already a dependency.

### Existing code being replaced

None — this is a new feature. Current `GeoPoint` scalar exists but has no spatial query support.

### New code to write

1. **`src/storage/geohash.rs`** — New file (from VardaDB_redb `src/storage/geohash.rs`):
   - `encode(lat, lng, precision) -> String` — encode coordinates to geohash
   - `decode(geohash) -> (lat, lng)` — decode geohash to center coordinates
   - `neighbors(geohash) -> Vec<String>` — return 8 adjacent geohash cells
   - `precision_for_radius(radius_meters) -> usize` — determine geohash precision for a radius
   - `haversine_distance(lat1, lng1, lat2, lng2) -> f64` — great-circle distance in meters
   - `expand_search(lat, lng, radius_meters) -> Vec<String>` — return all geohash prefixes covering a circular area

2. **`src/storage/codec.rs`** — Add:
   - `encode_geo_index(geohash, uid) -> Vec<u8>` — new key prefix for geohash index
   - `decode_geo_index(key) -> (String, u64)` — extract geohash and uid

3. **`src/storage/mod.rs`** — Add `pub mod geohash;`

4. **`src/bridge/sqlite_resolver.rs`** — Add:
   - `write_geo_index(uid, field, lat, lng)` — encode geohash, write to KV store
   - `search_near(lat, lng, radius_meters, field) -> Vec<(u64, f64)>` — expand search area, scan geohash prefixes, filter by haversine distance
   - Wire `near(lat, lng, radius)` filter into query handling

5. **`src/engine/schema.rs`** — Add:
   - Parse `@geo` directive on GeoPoint fields
   - Add `near(lat: Float!, lng: Float!, radius: Float!)` filter argument on geo fields

### Tests

```rust
// tests/geohash_test.rs
// 1. encode(37.7749, -122.4194, 6) → "9q8yyz"
// 2. decode("9q8yyz") → coordinates within 1km of (37.7749, -122.4194)
// 3. neighbors("9q8yyz") returns 8 adjacent geohashes
// 4. precision_for_radius(1000) → reasonable precision (4-5)
// 5. haversine_distance(0, 0, 0, 1) ≈ 111km
// 6. expand_search covers all geohashes within radius

// tests/geo_search_test.rs
// 1. Create 3 nodes in SF, 2 in NYC — search_near(SF, 50km) returns 3
// 2. Search near SF with 10km radius — returns only nearby nodes
// 3. Search with very small radius (1m) — returns only exact matches
// 4. Update location — old geo index removed, new one created
// 5. Delete node — geo index cleaned up
```

---

## Stage 11: Resolve List HNSW

### Why

When querying a relation field (e.g., `user { documents(nearVector: [...]) }`), the current code loads all related nodes and computes cosine similarity in-memory with a brute-force dot product. With many related nodes this is O(N*d). Using the HNSW index for this lookup is O(log N) and dramatically faster for large relation sets.

### Existing code being replaced

1. **`src/bridge/sqlite_resolver.rs`** — `resolve_list()` method: when `nearVector` is provided, it loads all related UIDs, then for each UID resolves the `embedding` field and computes brute-force cosine similarity (dot product / norm product).

### New code to write

1. **`src/bridge/sqlite_resolver.rs`** — Modify `resolve_list()`:
   - When `nearVector` is provided:
     1. Load the set of related UIDs (from edge index)
     2. Convert query vector to `Vec<f32>`
     3. Call `self.storage.vector_engine.search(db_name, &vec_f32, related_count * 2)` to over-fetch
     4. Filter HNSW results to only those in the related UID set
     5. Use the filtered+sorted UIDs as the result ordering
   - Remove the brute-force cosine similarity loop entirely

2. No changes to `SearchEngine` or `VectorEngine` — this is purely a resolver optimization.

### Tests

```rust
// tests/resolve_list_hnsw_test.rs
// 1. Create User with 100 Documents (each with embeddings). Query user { documents(nearVector: [...], first: 10) }
//    Verify results are ordered by vector similarity
// 2. Verify results only include documents belonging to that user (not other users' docs)
// 3. Create User with no documents. Query with nearVector returns empty.
// 4. Compare HNSW-ordered results with brute-force ordering — same top-10 (order may differ slightly due to ANN approximation)
// 5. Test with filter + nearVector: documents must match both filter and vector ordering
// 6. Performance test: 1000 related docs, nearVector query completes in <100ms
```

---

## Execution Order

Stages must be executed in order — each builds on the previous:

```
Stage 0  (Foundation)     ← MUST be first, everything depends on this
Stage 01 (Fuzzy)           ← builds on Stage 0
Stage 02 (Phrase)          ← builds on Stage 01
Stage 03 (Boosting)        ← builds on Stage 02
Stage 04 (RRF Weighting)   ← builds on Stage 0 (independent of 01-03)
Stage 05 (Highlighting)    ← builds on Stage 0
Stage 06 (BM25 Stats)      ← builds on Stage 0
Stage 07 (Durability)      ← builds on Stage 0
Stage 08 (Trigram)         ← builds on Stage 0
Stage 09 (Faceted)         ← builds on Stage 0
Stage 10 (Geo)             ← builds on Stage 0
Stage 11 (Resolve List)    ← builds on Stage 0 + VectorEngine
```

Stages 01-11 can be parallelized after Stage 0 is complete, except:
- Stage 02 requires Stage 01
- Stage 03 requires Stage 02

## Dependency Summary

| After Stage 0 | Can start in parallel |
|---|---|
| 01-Fuzzy, 04-RRF, 05-Highlighting, 06-Stats, 07-Durability, 08-Trigram, 09-Facet, 10-Geo, 11-ResolveList | all at once |
| After 01 | 02-Phrase |
| After 02 | 03-Boosting |
