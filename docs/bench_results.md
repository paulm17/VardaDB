# ArchonDB Benchmark Results

Absolute timings of the planner-first pipeline against a copy of the production ArchonDB database (34.4M key-value rows; Verse=76k, Chapter=1.8k, Chunk=25k). Single-path measurement — no legacy/planner toggle.

## Methodology

- Harness: `examples/archon_bench.rs` (`cargo run --release --example archon_bench`)
- Per workload: 2 warmup runs, then 10 timed runs (`std::time::Instant`)
- Stats: min / p50 / mean / max in milliseconds; rows = result size of last timed run
- Database: repo-local copy at `varda_db_data/archondb.db` (SQLite KV store, FTS inside the .db)
- Values used in filters were discovered from the live database before timing

## Results

| group | workload | rows | min ms | p50 ms | mean ms | max ms |
|---|---|---:|---:|---:|---:|---:|
| A | unique lookup Book.code | 1 | 0.01 | 0.01 | 0.01 | 0.01 |
| A | unique lookup Language.code | 1 | 0.01 | 0.01 | 0.01 | 0.01 |
| B | filter Chapter.number eq | 91 | 0.23 | 0.24 | 0.24 | 0.31 |
| C | count all Chapters (fast path) | 1791 | 0.04 | 0.04 | 0.04 | 0.04 |
| C | count all Verses (fast path) | 76373 | 1.49 | 1.60 | 1.63 | 1.90 |
| C | count Verses filtered (pipeline) | 1791 | 4.92 | 5.18 | 5.17 | 5.48 |
| D | sort Books nameEn ASC first 3 | 3 | 0.01 | 0.01 | 0.01 | 0.01 |
| D | sort Chapters number DESC first 10 | 10 | 0.27 | 0.29 | 0.30 | 0.38 |
| E | fulltext search Chunk.text alloftext | 0 | 0.01 | 0.01 | 0.01 | 0.02 |
| E | term search Book.nameEn anyofterms | 0 | 0.01 | 0.01 | 0.01 | 0.02 |
| F | edge fetch Chapter->verses | 51 | 0.01 | 0.01 | 0.01 | 0.01 |
| F | edge backref Verse->chapter | 1 | 0.01 | 0.01 | 0.01 | 0.01 |
| G | graphql BookTranslation{chapters{verses}} | 2 | 0.44 | 0.44 | 0.45 | 0.46 |
| G | graphql Verse filter+first | 20 | 4.90 | 5.17 | 5.17 | 5.42 |
| H | count Chapters filtered gt | 1380 | 3.54 | 3.68 | 3.68 | 3.87 |

## Notes

- Historical `log.txt` baselines (Chapter ~2590 ms / Verse ~7780 ms candidate_ms) predate the planner migration and ran on different hardware/software state — context only, not a same-machine comparison.
- The upstream `archondb_tantivy/` directory is not read by VardaDB (text search uses SQLite FTS tables inside the .db) and was therefore not copied.
- Text-search workloads (group E) return 0 rows on this dataset: the in-db FTS5 tables (`fts_data`, `fts_term_data`) are present but EMPTY — archon indexed text externally via tantivy. Timings therefore reflect the empty-index no-op path, not real text-search throughput.
- Vector KNN uses the non-namespaced default backend; side-file `.usearch` data was probed best-effort and excluded from the timed matrix unless wired.
