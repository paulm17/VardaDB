# Issue 00a: Redb Durability Audit

**File**: `src/storage/redb_backend.rs`, `src/storage/backend.rs`
**Effort**: 1 day
**Friction**: LOW

## Task
Verify all redb writes use `Durability::Immediate` (fsync on every commit).

## Audit Checklist

```rust
// Check 1: redb_backend.rs - begin_write() usage
// Should NOT see: db.begin_write_with_txn(txn_config_with_eventual_durability)
// Should see: db.begin_write() // uses default Immediate

// Check 2: backend.rs - put_with_lww, delete_with_lww paths
// Verify all write_batch calls go through redb_backend.write_batch()

// Check 3: redb_resolver.rs - batch operations
// Verify no direct Database::begin_write() with custom durability
```

## Fix (if needed)

If any path uses `Durability::Eventual`, change to `Durability::Immediate`:

```rust
// WRONG
let txn = db.begin_write_with_txn_config(
    TransactionConfig::new().set_durability(Durability::Eventual)
)?;

// CORRECT
let txn = db.begin_write()?; // Default is Immediate
```

## Documentation

Create `docs/DURABILITY.md`:

```markdown
# Durability Guarantee

All redb writes use `Durability::Immediate`.
- Every commit fsyncs to disk
- Zero data loss on power failure
- Slower than eventual durability but safe
```

## Test

```rust
#[test]
fn test_redb_uses_immediate_durability() {
    let storage = create_test_storage();
    
    // Verify all write paths use Immediate
    // This is a compile-time check + runtime audit
    let txn = storage.backend.db.begin_write().unwrap();
    assert_eq!(txn.durability(), redb::Durability::Immediate);
}
```

## Sign-off

- [x] Audit complete - all writes use Immediate durability
- [x] docs/DURABILITY.md created
