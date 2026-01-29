use std::sync::Mutex;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// A thread-safe Bounded LRU Cache for GraphQL Query Results.
/// 
/// Principles:
/// 1. Bounded: Fixed capacity (e.g. 100) to prevent OOM.
/// 2. LRU: Evicts least accessed items effectively.
/// 3. Invalidation: Clears entries based on tags (Type names).
pub struct QueryCache {
    inner: Mutex<LruCache<u64, String>>, // Key: Hash(Query+Vars), Value: JSON Response
    // For invalidation, we might need a secondary index: TypeName -> Vec<Key>
    // Or simplified: Global clear on mutation / Type-based clear by iterating (slow?).
    // Given the LRU size is small (100), iteration is fine.
}

impl QueryCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(NonZeroUsize::new(capacity).unwrap())),
        }
    }

    pub fn get(&self, query: &str, vars: &str) -> Option<String> {
        let key = Self::hash_query(query, vars);
        let mut lock = self.inner.lock().unwrap();
        lock.get(&key).cloned() // Return Clone of String (cheap enough for JSON)
    }

    pub fn put(&self, query: &str, vars: &str, result: String) {
        let key = Self::hash_query(query, vars);
        let mut lock = self.inner.lock().unwrap();
        lock.put(key, result);
    }

    pub fn invalidate(&self) {
        // Simple strategy: Clear All on Mutation.
        // This guarantees consistency without complex dependency tracking.
        // For a Demo/Local DB, this is acceptable.
        let mut lock = self.inner.lock().unwrap();
        lock.clear();
    }

    fn hash_query(query: &str, vars: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        vars.hash(&mut hasher);
        hasher.finish()
    }
}
