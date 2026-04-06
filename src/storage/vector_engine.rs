/// usearch-backed HNSW vector search engine for VardaDB.
///
/// Replaces the `sqlite-vec` extension and the stubbed vector worker.
///
/// Key design decisions:
/// * **f16 quantisation** – `ScalarKind::F16` halves vector storage vs f32
///   with minimal accuracy loss. Input is always accepted as `&[f32]`.
/// * **Per-database index** – one usearch file per logical database at
///   `{base_path}/{db_name}_vectors.usearch`.
/// * **Lazy dimension detection** – dimensions are inferred from the first
///   vector inserted and persisted in `{db_name}_vectors.dims` so the index
///   can be reloaded across restarts.
/// * **Thread safety** – each per-db index is wrapped in a `parking_lot::Mutex`.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::Mutex;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

struct DbVectorIndex {
    index: Index,
    dims: usize,
}

/// Thread-safe, multi-database HNSW vector engine backed by usearch.
pub struct VectorEngine {
    base_path: PathBuf,
    /// `db_name` → locked index handle.
    /// The `Option` supports lazy initialisation: `None` until the first
    /// vector is inserted (dimensions become known only then).
    indexes: DashMap<String, Arc<Mutex<Option<DbVectorIndex>>>>,
}

impl VectorEngine {
    pub fn new(base_path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            base_path: base_path.to_path_buf(),
            indexes: DashMap::new(),
        })
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn index_path(&self, db_name: &str) -> PathBuf {
        self.base_path.join(format!("{}_vectors.usearch", db_name))
    }

    fn dims_path(&self, db_name: &str) -> PathBuf {
        self.base_path.join(format!("{}_vectors.dims", db_name))
    }

    /// Return the slot for `db_name`, creating an empty `None` slot if absent.
    fn slot(&self, db_name: &str) -> Arc<Mutex<Option<DbVectorIndex>>> {
        if let Some(slot) = self.indexes.get(db_name) {
            return Arc::clone(slot.value());
        }
        let slot = Arc::new(Mutex::new(None));
        self.indexes.insert(db_name.to_string(), Arc::clone(&slot));
        slot
    }

    /// Build a new usearch `Index` for the given number of dimensions.
    /// Uses cosine similarity and f16 quantisation.
    fn create_index(dims: usize) -> anyhow::Result<Index> {
        let options = IndexOptions {
            dimensions: dims,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F16, // halves storage, Phase 4 requirement
            connectivity: 16,
            expansion_add: 128,
            expansion_search: 64,
            ..Default::default()
        };
        Ok(Index::new(&options)?)
    }

    /// Load a previously saved index from disk. The `.dims` file must exist.
    fn try_load(&self, db_name: &str) -> anyhow::Result<Option<DbVectorIndex>> {
        let index_path = self.index_path(db_name);
        let dims_path = self.dims_path(db_name);

        if !index_path.exists() || !dims_path.exists() {
            return Ok(None);
        }

        let dims_bytes = std::fs::read(&dims_path)?;
        if dims_bytes.len() < 8 {
            return Ok(None);
        }
        let dims = u64::from_be_bytes(dims_bytes[..8].try_into().unwrap()) as usize;
        if dims == 0 {
            return Ok(None);
        }

        let index = Self::create_index(dims)?;
        index.load(index_path.to_str().unwrap_or(""))?;
        Ok(Some(DbVectorIndex { index, dims }))
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Add or replace a vector for `uid` in the given database.
    ///
    /// On first call the index is lazily created with
    /// `dims = vector.len()`. The dimensions are persisted to disk so the
    /// index can be reloaded after restart.
    pub fn add_vector(&self, db_name: &str, uid: u64, vector: &[f32]) -> anyhow::Result<()> {
        if vector.is_empty() {
            return Ok(());
        }
        let slot = self.slot(db_name);
        let mut guard = slot.lock();

        if guard.is_none() {
            // Attempt to load from disk first.
            let loaded = self.try_load(db_name)?;
            if let Some(db_idx) = loaded {
                *guard = Some(db_idx);
            } else {
                // First ever vector — initialise the index.
                let dims = vector.len();
                let index = Self::create_index(dims)?;
                index.reserve(1024)?;
                // Persist dimension metadata.
                let dims_path = self.dims_path(db_name);
                std::fs::write(&dims_path, (dims as u64).to_be_bytes())?;
                *guard = Some(DbVectorIndex { index, dims });
            }
        }

        let db_idx = guard.as_mut().unwrap();

        // Dimension mismatch: skip and warn rather than panic.
        if vector.len() != db_idx.dims {
            eprintln!(
                "VectorEngine: dimension mismatch for db '{}': expected {}, got {} — skipping uid {}",
                db_name, db_idx.dims, vector.len(), uid
            );
            return Ok(());
        }

        // Remove the old entry if present (usearch does not auto-dedup).
        if db_idx.index.contains(uid) {
            let _ = db_idx.index.remove(uid);
        }

        // Grow the index capacity if needed.
        let size = db_idx.index.size();
        let capacity = db_idx.index.capacity();
        if size + 1 >= capacity {
            db_idx.index.reserve(capacity + 1024)?;
        }

        db_idx.index.add(uid, vector)?;
        Ok(())
    }

    /// Remove the vector for `uid` from the given database.
    pub fn remove_vector(&self, db_name: &str, uid: u64) -> anyhow::Result<()> {
        let slot = self.slot(db_name);
        let guard = slot.lock();
        if let Some(db_idx) = guard.as_ref() {
            if db_idx.index.contains(uid) {
                db_idx.index.remove(uid)?;
            }
        }
        Ok(())
    }

    /// Approximate nearest-neighbour search.
    ///
    /// Returns up to `k` `(uid, cosine_distance)` pairs sorted by ascending
    /// distance (closest first).
    pub fn search(&self, db_name: &str, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        let slot = self.slot(db_name);
        let guard = slot.lock();

        let db_idx = match guard.as_ref() {
            Some(i) => i,
            None => return vec![],
        };

        if query.len() != db_idx.dims {
            eprintln!(
                "VectorEngine: query dimension mismatch for db '{}': expected {}, got {}",
                db_name,
                db_idx.dims,
                query.len()
            );
            return vec![];
        }

        let results = match db_idx.index.search(query, k) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("VectorEngine: search error: {}", e);
                return vec![];
            }
        };

        results
            .keys
            .into_iter()
            .zip(results.distances.into_iter())
            .collect()
    }

    /// Check if a vector exists in the index for the given uid.
    pub fn contains(&self, db_name: &str, uid: u64) -> bool {
        let slot = self.slot(db_name);
        let guard = slot.lock();
        if let Some(db_idx) = guard.as_ref() {
            db_idx.index.contains(uid)
        } else {
            false
        }
    }

    /// Persist the index for `db_name` to disk.
    pub fn save(&self, db_name: &str) -> anyhow::Result<()> {
        let slot = self.slot(db_name);
        let guard = slot.lock();
        if let Some(db_idx) = guard.as_ref() {
            let path = self.index_path(db_name);
            db_idx.index.save(path.to_str().unwrap_or(""))?;
        }
        Ok(())
    }

    /// Persist all open indexes to disk.
    pub fn save_all(&self) -> anyhow::Result<()> {
        for entry in self.indexes.iter() {
            let db_name = entry.key().as_str();
            let guard = entry.value().lock();
            if let Some(db_idx) = guard.as_ref() {
                let path = self.index_path(db_name);
                db_idx.index.save(path.to_str().unwrap_or(""))?;
            }
        }
        Ok(())
    }
}
