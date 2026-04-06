# Durability Guarantee

All redb writes use `Durability::Immediate`.

- Every commit fsyncs to disk
- Zero data loss on power failure
- Slower than eventual durability but safe

## Implementation

All write transactions are created using `db.begin_write()` which defaults to `Durability::Immediate`:

```rust
// In redb_backend.rs
let write_txn = self.db.begin_write()?; // Default is Immediate
```

No code paths use `Durability::Eventual` or `begin_write_with_txn_config()` with custom durability settings.

## Audit Checklist

- `redb_backend.rs:46` - `create_table`: uses `begin_write()`
- `redb_backend.rs:71` - `drop_table`: uses `begin_write()`
- `redb_backend.rs:95` - `write_batch`: uses `begin_write()`
- `redb_backend.rs:391` - `insert`: uses `begin_write()`
- `redb_backend.rs:410` - `remove`: uses `begin_write()`
- `redb_backend.rs:448` - `upsert_lww`: uses `begin_write()`
- `redb_backend.rs:478` - `delete_lww`: uses `begin_write()`

All paths confirmed safe as of the initial audit.