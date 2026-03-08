use jobs::{JobStore, KvStore, Queue};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

/// In-memory KvStore for tests. Thread-safe via Mutex.
#[derive(Clone)]
pub struct MemoryKvStore {
    data: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
}

impl MemoryKvStore {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl KvStore for MemoryKvStore {
    fn kv_insert(&self, key: &[u8], value: &[u8]) -> Result<(), String> {
        self.data
            .lock()
            .unwrap()
            .insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        Ok(self.data.lock().unwrap().get(key).cloned())
    }

    fn kv_remove(&self, key: &[u8]) -> Result<(), String> {
        self.data.lock().unwrap().remove(key);
        Ok(())
    }

    fn kv_prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let data = self.data.lock().unwrap();
        let mut results: Vec<_> = data
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
}

pub struct TestContext {
    pub store: Arc<JobStore<MemoryKvStore>>,
    pub queue: Queue<MemoryKvStore>,
}

pub fn setup() -> TestContext {
    let kv = MemoryKvStore::new();
    let store = Arc::new(JobStore::new(Arc::new(kv)));
    let queue = Queue::new("default".into(), store.clone());

    TestContext { store, queue }
}
