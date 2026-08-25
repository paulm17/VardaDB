# RAG Completion — Post-Build Manual Steps

All code changes are complete and every workspace compiles (`cargo check` clean; no tests were executed per instruction). This file documents the steps that must be run manually, in order.

## What changed

| Project | Change |
|---|---|
| `_archon_old/rust/nlp/fast-ingest` | Now writes VardaDB-native indexes directly inside the SQLite DB: `fts_data` / `fts_term_data` (FTS5, identical DDL to VardaDB) and `vec_data` (sqlite-vec). Tantivy/usearch sidecars are opt-in only via `ARCHON_EXTERNAL_INDEX=1`. FTS/vector writes happen inside the same transaction as the KV rows, fixing resume-replay index drift. |
| `VardaDB` | Configurable vector dims; scored text/vector/hybrid search plumbed end-to-end; `_score` + `_snippet()` virtual fields; phrase queries (quoted spans); prefix matching (`term*`); per-predicate `boost`; multi-text-predicate weighted-RRF fusion; real trigram table (`fts_trigram_data`, `strategy:"trigram"` override); `aggregate{Type}(filter, groupBy, limit)` facet root fields; HNSW doc/code drift removed. |
| `archon` | All 16 resolver call sites updated to current VardaDB signatures; BM25 scores now fused into arm ranking instead of discarded; chapter-arm weight 1.4→0.9 and limits reduced to stop chapter flooding. |

## Step 1 — Configure vector dimensions BEFORE first startup

The live corpus uses `mxbai-embed-large-v1` (1024 dims), but `vec_data` defaults to `float[384]`.

Pick ONE before starting VardaDB / running the ingester against a fresh DB:

- config.toml:
  ```toml
  [search]
  vector_dims = 1024
  ```
- or env: `VARDADB_VECTOR_DIMS=1024`

If a DB already has a `vec_data` table, its existing DDL wins (dims are introspected and adopted). To change dims on an existing DB, drop the table first:
```sql
DROP TABLE IF EXISTS vec_data;
```
then restart with the desired config.

## Step 2 — Re-ingest the corpus

```bash
cd /Volumes/Data/Users/paul/development/src/github/_archon_old/rust/nlp
cargo build -p archon-fast-ingest
# orchestrator output must already exist under output/INGEST/
./target/debug/archon-fast-ingest   # reads config.toml db_path
```

Verify after ingest (against `archondb.db`):
```sql
SELECT COUNT(*) FROM fts_data;        -- > 0 (fulltext/porter)
SELECT COUNT(*) FROM fts_term_data;   -- > 0 (exact terms)
SELECT COUNT(*) FROM vec_data;        -- > 0, embeddings must be 1024-dim
```

Legacy external Tantivy/usearch sidecars are still available with
`ARCHON_EXTERNAL_INDEX=1 ./target/debug/archon-fast-ingest` but are no longer
needed by the app.

## Step 3 — Point archon at the re-ingested DB

archon's embedded VardaDB reads `/Volumes/Data/Users/paul/development/src/github/archon/db_data/archondb.db`.
Copy the freshly ingested DB there (server stopped):

```bash
cp /Volumes/Data/Users/paul/development/src/github/archon/db_data/archondb.db{,.bak}
cp <ingest-output>/archondb.db /Volumes/Data/Users/paul/development/src/github/archon/db_data/archondb.db
```

## Step 4 — Validation gates (before merging)

```bash
cd /Volumes/Data/Users/paul/development/src/github/VardaDB
cargo test --workspace          # full suite incl. new unit tests
cargo bench                     # criterion vs docs/bench_baselines.md
```

Bench gate: any access-path median regressing **>30%** fails review
(see `docs/bench_baselines.md`).

## Step 5 — Brain smoke test

```bash
cd /Volumes/Data/Users/paul/development/src/github/archon
cargo run            # starts embedded VardaDB :8000 + brain :9000
# then run the brain-test suite as before (previously 0 passed / 3 failed)
```

Expected improvements: text/entity/syntax/lemma arms return non-empty results
(previously all zero due to empty FTS tables), keyword assertions
(`sleep/wake/afraid/perish/rebuked/sea/feared`) hit because Text-arm results are
no longer drowned by Chapter-arm floods, and vector hits match scope (1024-dim
mxbai vectors in `vec_data`).

## New GraphQL surface (for reference)

- `_score: Float` on every type — bm25 (text), `1/(1+distance)` similarity (vector), RRF score (hybrid).
- `_snippet(before:, after:, ellipsis:, tokens:)` — lazy FTS5 `snippet()`.
- Phrases: `"sea of galilee"` inside any of `allofterms/anyofterms/alloftext/anyoftext`.
- Prefix: trailing `*` on bare terms (`rebuk*`).
- Per-predicate extras inside filter condition objects: `"boost": <number>`,
  `"strategy": "term"|"fulltext"|"trigram"` (both stripped before residual evaluation).
- Multiple text predicates on one type fuse by weighted RRF (k=60).
- Facets: `aggregateVerseContent(filter: {...}, groupBy: "book", limit: 10) { value count }`.

Known follow-ups intentionally NOT done here (per scope): C3 embedding-cache perf work (~90 embeds/query in `reasoning.rs`), I2 embed cache, sequential arm execution.
