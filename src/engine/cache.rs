use std::hash::{Hash, Hasher};
use std::time::Duration;

/// A thread-safe, lock-free, bounded cache for GraphQL query results.
///
/// Powered by `moka` — a concurrent cache with:
/// - Lock-free reads (no global Mutex contention)
/// - LRU eviction with size bounds
/// - TTL-based expiration
/// - Concurrent reads without blocking
///
/// This replaces the previous `Mutex<LruCache>` implementation which
/// required a global lock on every cache hit under concurrent GraphQL load.
pub struct QueryCache {
    inner: moka::sync::Cache<u64, String>,
}

impl QueryCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: moka::sync::Cache::builder()
                .max_capacity(capacity as u64)
                .time_to_live(Duration::from_secs(60)) // TTL: 1 minute
                .build(),
        }
    }

    pub fn get(&self, query: &str, vars: &str) -> Option<String> {
        let key = Self::hash_query(query, vars);
        self.inner.get(&key)
    }

    pub fn put(&self, query: &str, vars: &str, result: String) {
        let key = Self::hash_query(query, vars);
        self.inner.insert(key, result);
    }

    pub fn invalidate(&self) {
        self.inner.invalidate_all();
    }

    fn hash_query(query: &str, vars: &str) -> u64 {
        let mut hasher = ahash::AHasher::default();
        query.hash(&mut hasher);
        vars.hash(&mut hasher);
        hasher.finish()
    }
}
