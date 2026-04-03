# Verification Phase Results

## 1. Verify type context flow ✅ VERIFIED

**Finding:** `type_name` is available at line 1519 where `filter_by_field_value` is called.

```rust
// Line 1430: get_candidates signature
fn get_candidates(
    &self,
    type_name: &str,  // <-- AVAILABLE HERE
    filter: &std::collections::HashMap<String, Value>,
    ...
)
```

At line 1517-1519:
```rust
if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
    let sqlite_val = Self::json_to_sqlite_value(val);
    let uids = main_ks.filter_by_field_value(field, "=", sqlite_val);
```

**Conclusion:** `type_name` is in scope but NOT passed to `filter_by_field_value`. A new method `filter_by_field_value_typed(type_id, ...)` could be called with `type_name` converted to `type_id`.

---

## 2. Verify type_id coordination (Schema-driven) ✅ VERIFIED

**Finding:** Both VardaDB and fast-ingest can load TOML files natively. Schema-driven approach is feasible.

**Evidence:**
- `VardaDB/src/config.rs` line 116: `let config: VardaConfig = toml::from_str(&content)?;`
- `VardaDB/src/lib.rs` line 211-217: Loads schema from `{storage_path}/current_schema.graphql`
- Fast-ingest uses `nlp_lib::config` which reads `config.toml`
- `fast-ingest/src/main.rs` line 21-25: Already loads `db_path` from config

**Feasibility:** Creating a `type_ids.toml` file is straightforward:
```toml
[type_ids]
Verse = 1
Chapter = 2
Chunk = 3
TokenMorphology = 4
```

**Action needed:**
1. Create `type_ids.toml` alongside VardaDB config or in repo root
2. VardaDB loads via `toml::from_str` at startup
3. Fast-ingest loads same file before ingest (path relative to its config.toml location)
4. Both build `HashMap<&str, u16>` from file

**No new dependencies needed** - `toml` crate already in use by both systems.

---

## 3. Verify key encoding - ROUND-TRIP TEST ✅ VERIFIED

**Test:** Created `verify/src/main.rs` to inspect actual keys in archondb.db.

**Result:** Current encoding confirmed:
```
Key: [0x01][UID:8 BE][field_name]
      ↑      ↑          ↑
    pos 0  pos 1-8    pos 9+
```

**Sample from archondb.db:**
```
0100000000000000015F74797065 | 14 | [01] UID=0000000000000001 field="_type"
010000000000000001636F6465   | 13 | [01] UID=0000000000000001 field="code"
0100000000000000016E616D65   | 13 | [01] UID=0000000000000001 field="name"
```

**Formula verified:** `key_len = 1 + 8 + field_name.len()` ✅

**Proposed change:** Insert 2 bytes (type_id LE) between UID and field_name:
```
New: [0x01][UID:8 BE][TypeID:2 LE][field_name]
              ↑              ↑
           pos 1-8       pos 9-10
                        pos 11+ = field_name
```

**New field offset:** `key[12..]` instead of `key[9..]` for field extraction.

---

## 4. SQLite blob binding verification ✅ VERIFIED

**Test result:** rusqlite correctly binds `&[u8]` as BLOB.

The code at line 692-697 in sqlite_backend.rs:
```rust
rusqlite::params![
    &[data_prefix][..],  // BLOB - verified works
    expected_key_len as i64,
    field_bytes,         // BLOB - verified works  
    target,
],
```

**Conclusion:** Binding `&type_id.to_le_bytes()[..]` as a BLOB parameter will work correctly.

---

## 5. Verify index creation SQL for typed indexes ❌ BUG CONFIRMED

**Bug found in plan:** The plan's `create_sql_typed` interpolates `type_id` as decimal.

```rust
"WHERE substr(key, 10, 2) = {} AND length(key) = {}"
//                                        ^^^^^ BUG: decimal 3, not hex X'0300'
```

**Fix confirmed:** Use hex literal with `swap_bytes()`:
```rust
format!("X'{:04x}'", type_id.swap_bytes())  // type_id=3 → X'0300'
```

---

## 6. CRITICAL: Audit encode_data_prefix callers ✅ AUDITED

**Found 5 call sites in sqlite_resolver.rs:**

| Line | Usage | Risk |
|------|-------|------|
| 43 | `encode_data_prefix(min_uid)` for range scan | HIGH - scans typed keys with untyped prefix |
| 45 | `encode_data_prefix(max_uid)` for range bound | HIGH - same |
| 77 | `load_object_fields` prefix scan | HIGH - breaks typed keys |
| 2908 | `rebuild_order_index_for_field` | MEDIUM - uses 0x03 prefix, separate |
| 2942 | Delete data keys scan | HIGH - breaks typed keys |

**Key insight:** `encode_data_prefix` creates `[0x01][UID:8]` which is a prefix of both untyped AND typed keys. When typed keys are written, untyped prefix scans will find them BUT field extraction will be wrong.

**Action needed:** Create `encode_data_prefix_typed(uid, type_id)` returning `[0x01][UID:8][TypeID:2]`.

---

## 7. CRITICAL: Audit all key slicers after position 9 ✅ AUDITED

**Found multiple broken usages:**

### sqlite_resolver.rs
| Line | Code | Issue |
|------|------|-------|
| 59 | `std::str::from_utf8(&key[9..])` | Gets `[TypeID:2][field_name]` instead of `[field_name]` |
| 84 | `key[prefix.len()..]` where `prefix.len()=9` | Same issue - gives wrong slice |
| 2914 | `&key[data_prefix.len()..]` where `len()=9` | Same issue |
| 2957 | `&k[9..]` | Same issue |

### sqlite_backend.rs (filter functions)
| Line | Code | Issue |
|------|------|-------|
| 683 | `substr(key, 10) = ?3` | Gets `TypeID:2 + field_name` instead of `field_name` |
| 748 | `substr(key, 10) = ?3` | Same |
| 810 | `substr(key, 10) = ?3` | Same |

**All must be updated to account for 2-byte type_id at position 10-11.**

---

## 8. Verify encode_type_index_key usage ✅ BENIGN

**Finding:** `[0x03][TypeName][0x00][UID]` is a SEPARATE key space from `[0x01]` data keys.

Usage in sqlite_resolver.rs:
- Line 616, 671, 2103, 2262, 2932, 3310, 3965: Used for type membership queries
- Line 418, 3299, 3535: Used to scan all UIDs of a type (candidate generation)

**Conclusion:** The type index (0x03) does NOT need to change. It stores type name as string, not type_id. It's orthogonal to the typed data key change.

---

## Summary of Verification

| Item | Status | Action |
|------|--------|--------|
| 1. Type context flow | ✅ Verified | Implement `filter_by_field_value_typed(type_id, ...)` |
| 2. Type ID coordination | ✅ Verified | Create `type_ids.toml` file |
| 3. Key encoding round-trip | ✅ Verified | Confirmed byte layout, new offset key[12..] for fields |
| 4. SQLite blob binding | ✅ Verified | No change needed |
| 5. Index SQL hex literal | ❌ Bug confirmed | Fix with `swap_bytes()` |
| 6. encode_data_prefix callers | ✅ Audited | Create `encode_data_prefix_typed` |
| 7. Key slicers position 9+ | ✅ Audited | All 8 locations must be updated |
| 8. encode_type_index_key | ✅ Benign | No change needed |

**All verification items complete except item 5 (index SQL bug fix documented).**

---

## Implementation Order (Updated)

0. ~~Write round-trip test for key encoding~~ ✅ DONE
1. Create `type_ids.toml` with type ID mapping
2. Add `encode_data_prefix_typed(uid, type_id)` to codec.rs
3. Add `encode_data_key_typed(uid, type_id, predicate)` to codec.rs
4. Add `filter_by_field_value_typed(type_id, field, op, target)` to sqlite_backend.rs
5. Update ALL key slicers in sqlite_resolver.rs (8 locations)
6. Update ALL `substr(key, 10)` to `substr(key, 12)` in sqlite_backend.rs (3 locations)
7. Update fast-ingest codec.rs with typed encoding
8. Update fast-ingest schema.rs to read type_ids.toml
9. Update fast-ingest index.rs with correct hex literal format
10. Re-run fast-ingest to regenerate data
11. Benchmark
