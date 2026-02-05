use jobs::{JobStore, Queue};
use std::sync::Arc;
use fjall::{Database, KeyspaceCreateOptions};

pub struct TestContext {
    pub _temp_dir: tempfile::TempDir, // Keep alive to prevent deletion
    pub store: Arc<JobStore>,
    pub queue: Queue,
    pub _db: Arc<Database>,
}

pub fn setup() -> TestContext {
    let _temp_dir = tempfile::tempdir().unwrap();
    let db = Database::builder(_temp_dir.path()).open().unwrap();
    let keyspace = db.keyspace("test_jobs", || KeyspaceCreateOptions::default()).unwrap();
    let store = Arc::new(JobStore::new(Arc::new(keyspace)));
    let queue = Queue::new("default".into(), store.clone());
    
    TestContext {
        _temp_dir,
        store,
        queue,
        _db: Arc::new(db), // Database is Arc internally too, but for struct
    }
}
