# VardaDB Implementation Plan

## Infrastructure First (Data Safety)

Do these 4 issues FIRST before any features. Without these, you **will lose data**.

| Issue | Task | Effort | File | Status |
|-------|------|--------|------|--------|
| 00a | Redb durability audit | 1 day | `src/storage/redb_backend.rs` | ✅ Done |
| 00b | Tantivy commit-on-write | 1 day | `src/storage/tantivy_search.rs` | ✅ Done |
| 00c | Backup/Restore | 3 days | Admin endpoints | ✅ Done |
| 00d | Vector crash safety | 2 weeks | `src/storage/backend.rs` | ✅ Done |

**Total: 2.5 weeks**

## Features (Do After Infrastructure)

| Issue | Task | Effort | File | Status |
|-------|------|--------|------|--------|
| 01 | Fuzzy matching | 1 week | `src/storage/tantivy_search.rs` | ✅ Done |
| 02 | Phrase queries | 1-2 weeks | `src/storage/tantivy_search.rs` | ✅ Done |
| 03 | Field boosting | 1-2 weeks | `src/storage/tantivy_search.rs` | ✅ Done |
| 04 | RRF weighting | 1 week | `src/bridge/redb_resolver.rs` | ✅ Done |
| 05 | Highlighting | 1 week | `src/storage/tantivy_search.rs` | ✅ Done |
| 06 | BM25 stats | 1 week | `src/storage/tantivy_search.rs` | ✅ Done |
| 07 | Tantivy batching | 1-2 weeks | `src/storage/tantivy_search.rs` | ✅ Done |
| 08 | Trigram index | 1 week | `src/storage/codec.rs` | ✅ Done |
| 09 | Faceted search | 2-3 weeks | `src/storage/tantivy_search.rs` | ✅ Done |
| 10 | Geo spatial index | 3-4 weeks | `src/storage/codec.rs` | ✅ Done |
| 11 | resolve_list HNSW | 1-2 weeks | `src/bridge/redb_resolver.rs` | |
| 12 | Embedding generation | 5-6 weeks | `src/embedding/mod.rs` | |

**Note**: Issue 12 (was vector persistence) is now `00d`. Embedding generation moves to position 12.

**Total: 19-26 weeks for features**

## Recommended Order

1. **Week 1**: Infrastructure (00a, 00b, 00c)
2. **Weeks 2-3**: 00d (vector crash safety)
3. **Weeks 4+**: Features in order (01-12)

## Dependencies

- 00a, 00b, 00c: Independent
- 00d: Touches storage layer, do after 00a
- 01-06: Independent (all Tantivy search)
- 07: Touches same file as 01-06, coordinate
- 08, 09, 10: Independent
- 11: Needs vector engine working (depends on 00d)
- 12: Do last (touches everything)
