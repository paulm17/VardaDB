# Option 2: Type-Specific Key Encoding - Implementation Plan

---

## Problem Summary

**Key encoding:** `[0x01][UID:8][field_name]`

The problem: All types share the same key prefix. When querying `Verse.number = 1`:
- Index `idx_verse_number` returns ALL records with `number` field across Verse, Chapter, TokenMorphology, etc.
- SQLite must scan 379,914 rows and filter by JSON value to get only Verse records
- Result: ~2,000 Verse records returned from ~380,000 scanned

**Timings:**
| Operation | Current | With Caching (warm) |
|-----------|---------|---------------------|
| `filter_by_field_value` (number) | ~4,200ms | ~0ms |
| `filter_by_field_value` (chunkType) | ~2,400ms | ~0ms |
| Initial page load | ~20s | ~6-8s |
| Verse navigation (warm) | ~2s | ~2s |

---

## Solution: Type-Specific Key Encoding

### New Key Encoding

**Proposed:** `[0x01][UID:8][TypeID:2 bytes LE][field_name]`

Where `[TypeID:2 bytes LE]` is a 2-byte little-endian type ID.

### Why This Works

1. **Type-specific prefix scan**: Query for `Verse.number` becomes a prefix scan on `[0x01][UID:8][VerseTypeID][number]`
2. **Indexes become selective**: The partial indexes we created now filter by type
3. **50-100x fewer rows scanned**: Instead of 380,000 rows, we scan only Verse records (~7,000)

### Expected New Timings

| Operation | Current | With Option 2 |
|-----------|---------|----------------|
| `filter_by_field_value` (number) cold | ~4,200ms | ~50-100ms |
| `filter_by_field_value` (chunkType) cold | ~2,400ms | ~50-100ms |
| Initial page load | ~20s | **~2-3s** |
| Verse navigation | ~2s | **~0.1-0.2s** |

---

## Verification Phase (COMPLETE ✅) - SEE ./verification.md for more details

1. **Verify type context flow:**
   - [x] `type_name` is available at line 1519 where `filter_by_field_value` is called
   - [x] Can pass it to `filter_by_field_value_typed(type_id, ...)` method

2. **Verify type_id coordination (CHOSEN: Schema-driven):**
   - [x] VardaDB loads TOML at startup via `toml::from_str` (confirmed in config.rs line 116)
   - [x] Fast-ingest reads same file before ingest (confirmed in main.rs line 21-25)
   - [x] Both build identical `type_id_for_name` maps from `type_ids.toml`

3. **Verify key encoding:**
   - [x] Round-trip test written and confirmed in `verify/src/main.rs`
   - [x] Byte layout confirmed: `key[0]=prefix, key[1..9]=UID, key[9..11]=TypeID, key[11..]=field`

4. **CRITICAL: SQLite blob binding verification:**
   - [x] rusqlite correctly binds `&[u8]` as BLOB (confirmed at sqlite_backend.rs lines 692-697)
   - [x] Binding `&type_id.to_le_bytes()[..]` as BLOB parameter will work correctly

5. **Verify index creation SQL for typed indexes:**
   - [x] BUG confirmed: `type_id` interpolated as decimal integer — will not match blob column
   - [x] Fix: `format!("X'{:04x}'", type_id.swap_bytes())`
   - [x] Explanation: type_id stored LE `[0x03, 0x00]`; SQLite `X'HHHH'` is BE convention → `X'0300'`
   - [x] Comment required at call site: "X'HHHH' is BE by SQLite convention; type_id is stored LE, so swap_bytes() is needed"

6. **CRITICAL: Audit encode_data_prefix callers:**
   - [x] `encode_data_prefix_typed` is NOT needed
   - [x] Untyped prefix `[0x01][UID:8]` is a prefix of typed keys; prefix scan works
   - [x] Only field extraction at slicers breaks; fixing slicers is sufficient

7. **CRITICAL: Audit all key slicers after position 9:**
   - [x] 8 locations in sqlite_resolver.rs: `key[9..]` → `key[11..]`
   - [x] 3 locations in sqlite_backend.rs: `substr(key, 10)` → `substr(key, 12)`

8. **Verify encode_type_index_key usage:**
   - [x] Type index `[0x03][TypeName][0x00][UID]` is separate key space — does NOT need to change
   - [x] Used for candidate generation only; orthogonal to typed data keys

---

## Timeline (Master Sequence)

**Phase 0:** Verification Phase ✅ DONE

**VardaDB Changes (Phases 1-6):**

| Phase | Task |
|-------|------|
| **Phase 1** | Create `type_ids.toml` AND add loading to VardaDB (engine/schema.rs loads TOML at startup) |
| **Phase 2** | Add `encode_data_key_typed(uid, type_id, predicate)` to codec.rs |
| **Phase 3** | Add `filter_by_field_value_typed(type_id, field, op, target)` to sqlite_backend.rs |
| **Phase 4** | Update ALL key slicers: `key[9..]` → `key[11..]` and `substr(key, 10)` → `substr(key, 12)` (8 in resolver + 3 in backend) |
| **Phase 5** | Update resolver to pass type_id to typed filter |
| **Phase 6** | Test VardaDB changes in isolation |

**Fast-Ingest Changes (Phases 7-10):**

| Phase | Task |
|-------|------|
| **Phase 7** | Add `encode_data_key_typed` to fast-ingest codec.rs (must match VardaDB exactly) |
| **Phase 8** | Update fast-ingest schema.rs to read type_ids.toml, add `type_id` field to `IngestFile` |
| **Phase 9** | Update fast-ingest ingest.rs to use typed encoding when writing |
| **Phase 10** | Update fast-ingest index.rs with correct hex literal format (`swap_bytes()`) |

**Cutover & Verify (Phases 11-12):**

| Phase | Task |
|-------|------|
| **Phase 11** | Re-run fast-ingest to regenerate data with new keys |
| **Phase 12** | Hard cutover test (no mixed format period allowed) |
| **Phase 13** | Benchmark and verify success criteria |

---

## Files to Change (Ordered by Timeline)

### VardaDB - Phase 1

#### `type_ids.toml` (NEW FILE)

```toml
[type_ids]
Verse = 1
Chapter = 2
Chunk = 3
TokenMorphology = 4
# ... etc
```

#### `VardaDB/src/engine/schema.rs`

Load `type_ids.toml` at startup and build `type_id_for_name` map.

---

### VardaDB - Phase 2

#### `VardaDB/src/storage/codec.rs`

Add `encode_data_key_typed`:

```rust
/// Data key with type: [0x01][UID:8][TypeID:2][field_name]
pub fn encode_data_key_typed(uid: u64, type_id: u16, predicate: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 8 + 2 + predicate.len());
    buf.push(0x01);
    buf.write_u64::<BigEndian>(uid).unwrap();
    buf.write_u16::<LittleEndian>(type_id).unwrap();
    buf.extend_from_slice(predicate.as_bytes());
    buf
}
```

---

### VardaDB - Phase 3

#### `VardaDB/src/storage/sqlite_backend.rs`

Add `filter_by_field_value_typed` with new key structure:

```rust
pub fn filter_by_field_value_typed(
    &self,
    type_id: u16,
    field_name: &str,
    op: &str,
    target: rusqlite::types::Value,
) -> Vec<u64> {
    // Check cache first
    let cache_key = FilterCacheKey::new(field_name, op, &target);
    {
        let mut cache = self.filter_cache.lock().unwrap();
        if let Some(cached) = cache.get(&cache_key) {
            return cached.clone();
        }
    }

    // Field now starts at byte offset 11 (key[11..]), so substr(key, 12) in SQL (1-indexed)
    let sql = format!(
        "SELECT substr(key, 2, 8) FROM \"{}\" \
         WHERE substr(key, 1, 1) = ?1 \
         AND substr(key, 10, 2) = ?2 \
         AND length(key) = ?3 \
         AND substr(key, 12) = ?4 \
         AND json_extract(CAST(substr(value, 19) AS TEXT), '$') {} ?5",
        self.name, op
    );

    // ... rest of implementation
}
```

---

### VardaDB - Phase 4

#### `VardaDB/src/bridge/sqlite_resolver.rs`

Update 8 key slicer locations:
- Lines 59, 84, 2914, 2957: `key[9..]` → `key[11..]`

#### `VardaDB/src/storage/sqlite_backend.rs`

Update 3 SQL substr locations:
- Lines 683, 748, 810: `substr(key, 10)` → `substr(key, 12)`

---

### VardaDB - Phase 5

#### `VardaDB/src/bridge/sqlite_resolver.rs`

Update resolver to pass type_id:

```rust
// In get_candidates method:
if let Some(type_id) = type_id_for_name(&node_type) {
    results = table.filter_by_field_value_typed(type_id, field, op, target);
} else {
    // Fallback to old untyped query for unknown types
    results = table.filter_by_field_value(field, op, target);
}
```

---

### Fast-Ingest - Phase 7 - TELL THE USER WHEN YOU ARE ALMOST HERE!  AT THE END OF THE VARDADB PROCESS.

#### `fast-ingest/src/codec.rs`

Add `encode_data_key_typed` (must match VardaDB exactly):

```rust
/// Data key with type: [0x01][UID:8 BE][TypeID:2 LE][field_name]
pub fn encode_data_key_typed(uid: u64, type_id: u16, predicate: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 8 + 2 + predicate.len());
    buf.push(0x01);
    buf.write_u64::<BigEndian>(uid).unwrap();
    buf.write_u16::<LittleEndian>(type_id).unwrap();
    buf.extend_from_slice(predicate.as_bytes());
    buf
}
```

---

### Fast-Ingest - Phase 8

#### `fast-ingest/src/schema.rs`

Add `type_id` field to `IngestFile`:

```rust
struct IngestFile {
    // ... existing fields
    type_id: u16,  // NEW: read from type_ids.toml
}
```

#### `type_ids.toml` (same file as VardaDB)

Fast-ingest reads this before ingest to get type IDs.

---

### Fast-Ingest - Phase 9

#### `fast-ingest/src/ingest.rs`

Use typed key encoding when writing:

```rust
let key = Codec::encode_data_key_typed(uid, file_def.type_id, graphql_name);
```

---

### Fast-Ingest - Phase 10

#### `fast-ingest/src/index.rs`

Create typed indexes with correct hex literal format:

```rust
fn create_sql_typed(&self) -> String {
    // X'HHHH' is BE byte order; type_id is LE stored, so swap_bytes() is needed
    let type_id_hex = format!("X'{:04x}'", self.type_id.swap_bytes());
    format!(
        "CREATE INDEX IF NOT EXISTS {} ON {} (
            json_extract(CAST(substr(value, 19) AS TEXT), '$.{}')
        ) WHERE substr(key, 1, 1) = 0x01 AND substr(key, 10, 2) = {} AND length(key) = {};",
        self.index_name(),
        MAIN_TABLE,
        self.field_name,
        type_id_hex,  // hex literal, not decimal
        self.key_length
    )
}
```

---

## Data Migration

For existing archondb.db with old key format:
- Re-run fast-ingest to regenerate all data with new keys (Phase 11)
- No live migration possible - hard cutover required

---

## Key Byte Layout

```
key[0]         = prefix byte (0x01)
key[1..9]      = UID (8 bytes, big-endian)
key[9..11]     = TypeID (2 bytes, little-endian) ← NEW
key[11..]      = field_name                    ← was key[9..]
```

**Correct offsets:**
- Rust: `key[11..]` (0-indexed, field starts at byte 11)
- SQLite: `substr(key, 12)` (1-indexed, same byte 11)

---

## Tests

Each test verifies a specific requirement. All must pass before cutover.

### VardaDB Tests

**Test 1: `encode_data_key_typed` encoding matches spec**
```rust
// uid=123, type_id=1, field="number"
// Expected bytes: [0x01][00_00_00_00_00_00_00_7B][01_00]["number"]
let key = encode_data_key_typed(123, 1, "number");
assert_eq!(key[0], 0x01);                    // prefix
assert_eq!(&key[1..9], &[0,0,0,0,0,0,0,0x7B]); // UID BE (8 bytes for u64)
assert_eq!(&key[9..11], &[0x01, 0x00]);     // type_id LE (1 = 0x0001)
assert_eq!(&key[11..], "number".as_bytes());  // field
```

**Test 2: `filter_by_field_value_typed` round-trip**
```rust
// 1. Write record with typed key
// 2. Read back with filter_by_field_value_typed
// 3. Assert returns correct UID
// 4. Assert wrong type_id returns empty
```

**Test 3: Key slicers updated correctly**
```rust
// After slicer changes:
// key[11..] should extract field name correctly
// Verify with known key from verify/src/main.rs output
```

**Test 4: Type IDs loaded from `type_ids.toml`**
```rust
// Verify type_id_for_name("Verse") == configured value
// Verify matches fast-ingest's type_ids.toml
```

### Fast-Ingest Tests

**Test 5: `encode_data_key_typed` produces identical bytes to VardaDB**
```rust
// Same inputs → Same output (byte-for-byte match)
// Test with uid=123, type_id=1, field="number"
assert_eq!(
    fast_ingest::encode_data_key_typed(123, 1, "number"),
    vardadb::encode_data_key_typed(123, 1, "number")
);
```

**Test 6: Fast-ingest and VardaDB load same `type_ids.toml`**
```rust
// Both systems should produce identical type_id_for_name maps
```

### Integration Tests

**Test 7: End-to-end round-trip**
```rust
// 1. fast-ingest writes record with typed key
// 2. VardaDB reads via filter_by_field_value_typed
// 3. Assert UID recovered correctly
```

**Test 8: Wrong type_id returns empty**
```rust
// Write with type_id=1, query with type_id=2 → empty result
```

---

## Cutover Process

1. Delete old archondb.db - The user will do this.  DO NOT DELETE IT.
2. Run fast-ingest (generates new db with typed keys + creates indexes)
3. Deploy VardaDB with typed key support
4. Done — no migration, no mixed format

---

## Success Criteria

- [ ] All 8 tests pass
- [ ] Initial page load under 3 seconds
- [ ] Verse navigation under 200ms
