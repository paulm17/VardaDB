use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use usearch::ffi::{IndexOptions, MetricKind, ScalarKind};
use usearch::Index;

struct DbVectorIndex {
    index: Index,
}

pub struct VectorEngine {
    indexes: DashMap<String, Arc<Mutex<DbVectorIndex>>>,
    base_path: PathBuf,
}

fn dims_file(base_path: &Path, db_name: &str) -> PathBuf {
    base_path.join(format!("{}_vectors.dims", db_name))
}

fn index_file(base_path: &Path, db_name: &str) -> PathBuf {
    base_path.join(format!("{}_vectors.usearch", db_name))
}

fn load_dims(base_path: &Path, db_name: &str) -> Option<usize> {
    let path = dims_file(base_path, db_name);
    let s = std::fs::read_to_string(&path).ok()?;
    s.trim().parse().ok()
}

fn save_dims(base_path: &Path, db_name: &str, dims: usize) -> std::io::Result<()> {
    let path = dims_file(base_path, db_name);
    std::fs::write(&path, dims.to_string())
}

fn create_index(dims: usize) -> anyhow::Result<Index> {
    let options = IndexOptions {
        dimensions: dims,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F16,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 64,
        multi: false,
    };
    Index::new(&options).map_err(|e| anyhow::anyhow!("usearch create: {}", e))
}

impl VectorEngine {
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            indexes: DashMap::new(),
            base_path: base_path.as_ref().to_path_buf(),
        }
    }

    fn get_or_create(
        &self,
        db_name: &str,
        dims: usize,
    ) -> anyhow::Result<Arc<Mutex<DbVectorIndex>>> {
        match self.indexes.entry(db_name.to_string()) {
            Entry::Occupied(e) => Ok(e.get().clone()),
            Entry::Vacant(e) => {
                let idx = create_index(dims)?;
                let idx_path = index_file(&self.base_path, db_name);
                let path_str = idx_path.to_string_lossy().to_string();
                if idx_path.exists() {
                    idx.load(&path_str)
                        .map_err(|e| anyhow::anyhow!("usearch load: {}", e))?;
                } else {
                    idx.reserve(1000)
                        .map_err(|e| anyhow::anyhow!("usearch reserve: {}", e))?;
                }
                let arc = Arc::new(Mutex::new(DbVectorIndex { index: idx }));
                e.insert(arc.clone());
                save_dims(&self.base_path, db_name, dims)?;
                Ok(arc)
            }
        }
    }

    pub fn add_vector(&self, db_name: &str, uid: u64, vector: &[f32]) -> anyhow::Result<()> {
        let dims = vector.len();
        if dims == 0 {
            return Ok(());
        }

        let stored_dims = load_dims(&self.base_path, db_name);
        let effective_dims = stored_dims.unwrap_or(dims);

        if let Some(stored) = stored_dims {
            if dims != stored {
                return Err(anyhow::anyhow!(
                    "Vector dimension mismatch: expected {}, got {}",
                    stored,
                    dims
                ));
            }
        }

        let entry = self.get_or_create(db_name, effective_dims)?;
        let guard = entry.lock();

        guard
            .index
            .add(uid, vector)
            .map_err(|e| anyhow::anyhow!("usearch add: {}", e))?;
        Ok(())
    }

    pub fn remove_vector(&self, db_name: &str, uid: u64) -> anyhow::Result<()> {
        let stored_dims = load_dims(&self.base_path, db_name);
        let dims = match stored_dims {
            Some(d) => d,
            None => return Ok(()),
        };

        let entry = self.get_or_create(db_name, dims)?;
        let guard = entry.lock();
        let _ = guard.index.remove(uid);
        Ok(())
    }

    pub fn search(&self, db_name: &str, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        let stored_dims = match load_dims(&self.base_path, db_name) {
            Some(d) => d,
            None => return vec![],
        };

        let entry = match self.get_or_create(db_name, stored_dims) {
            Ok(e) => e,
            Err(_) => return vec![],
        };

        let guard = entry.lock();

        let results = match guard.index.search(query, k) {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        let mut out = Vec::new();
        for i in 0..results.keys.len() {
            out.push((results.keys[i], results.distances[i]));
        }
        out
    }

    pub fn save(&self, db_name: &str) -> anyhow::Result<()> {
        if let Some(entry) = self.indexes.get(db_name) {
            let guard = entry.value().lock();
            let idx_path = index_file(&self.base_path, db_name);
            let path_str = idx_path.to_string_lossy().to_string();
            guard
                .index
                .save(&path_str)
                .map_err(|e| anyhow::anyhow!("usearch save: {}", e))?;
        }
        Ok(())
    }

    pub fn save_all(&self) -> anyhow::Result<()> {
        for entry in self.indexes.iter() {
            let guard = entry.value().lock();
            let idx_path = index_file(&self.base_path, entry.key());
            let path_str = idx_path.to_string_lossy().to_string();
            guard
                .index
                .save(&path_str)
                .map_err(|e| anyhow::anyhow!("usearch save: {}", e))?;
        }
        Ok(())
    }
}
