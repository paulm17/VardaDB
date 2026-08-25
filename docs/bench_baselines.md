# Planner Benchmark Baselines & Regression Rules

Criterion regression suite for the query-planner pipeline, implementing the
spec's §Benchmark And Regression Plan (`query_planner.md`). Workloads run
against the production-scale ArchonDB copy in `varda_db_data/archondb.db`
(34.4M KV rows; Verse=76,373 / Chapter=1,791 / Chunk=25,001).

## Usage

```sh
# capture / refresh the baseline
cargo bench --bench planner_bench -- --save-baseline main

# compare current tree against baseline (exit code reflects regressions)
cargo bench --bench planner_bench -- --baseline main

# single scenario
cargo bench --bench planner_bench -- 09_count/verses_filtered_pipeline
```

HTML reports land in `target/criterion/report/index.html`.

## Workload map (spec → scenario)

| Spec workload | Scenario | Notes |
|---|---|---|
| 1 full scan | `01_full_scan/verse_unfiltered` | 76k rows streamed through FullTypeScan |
| 2 unique get | `02_unique_get/{book,language}_by_code` | unique-index point lookup |
| 3 ordered scan+first | `03_ordered_scan_first/chapter_number_asc_first10` | ordered-index probe path |
| 4 nested relation LOW | `04_nested_relation_low/translation_chapters_number_eq` | legacy log.txt context: Chapter ~2590 ms |
| 5 nested relation MED | `05_nested_relation_wide/chapter_verses_number_eq` | wide parent set; log.txt Verse ~7780 ms |
| 6 text search | `06_text_search/chunk_text_alloftext` | FTS tables EMPTY in this dataset (archon indexed externally via tantivy) — measures empty-index no-op path only |
| 7 vector search | *(skipped)* | `Storage::search_vectors` is non-namespaced; archon vectors live in unreadable side-files |
| 8 edge fanout | `08_edge_fetch/chapter_to_verses` | resolve_list inverse-edge fetch (~42 rows avg) |
| 9 counts | `09_count/*` | prefix fast path vs filtered pipeline (`chapters_filtered_pipeline` uses `number ge 15`) |
| 10 aggregation | `10_aggregate/sum_chapter_number` | HashAggregateOperator over FullTypeScan |
| e2e | `11_graphql_e2e/nested_translation_chapters_verses` | GraphQL surface incl. dynamic-schema overhead |

Values used in filters are discovered from the live database at setup so every
scenario hits representative data.

**Dataset gotcha**: chapter numbers repeat per translation (each of the 91
translations numbers its chapters from 1), so high thresholds like
`number gt 1380` match nothing and hit the planner's ordered-index empty-probe
path (~5 µs) rather than the scan+residual pipeline. Filtered-count scenarios
deliberately use small thresholds to exercise real row evaluation.

## Regression rules (from spec §Benchmark And Regression Plan)

1. **Any scenario** whose median regresses **>30%** vs the saved baseline
   fails the review gate.
2. **Cold-path scenarios** (first scenario in each group, OS page-cache
   dependent: `verse_unfiltered`, `chapter_to_verses`) tolerate up to **2×**
   jitter before counting as regressions.
3. Parity suites (`cargo test`) always gate independently of bench numbers —
   a correctness regression blocks regardless of timing.
4. Historical pre-planner baselines (log.txt: Chapter ~2590 ms / Verse
   ~7780 ms candidate time) are context only — different hardware/run, not
   comparable to these absolute numbers.

## Reference numbers (initial capture, release profile)

See `docs/bench_results.md` for the one-shot absolute-timing table captured
with `examples/archon_bench.rs`; criterion medians from this suite are the
authoritative regression reference going forward.
