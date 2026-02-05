use jobs::{Job, JobStore, Queue};
use std::sync::Arc;
use fjall::{KeyspaceCreateOptions}; 
// VardaDB uses `Storage::new`. We can manually create a temp Partition for unit testing.

#[test]
fn test_priority_ordering() {
    let path = tempfile::tempdir().unwrap();
    let db = fjall::Database::builder(path.path()).open().unwrap();
    let keyspace = db.keyspace("test_jobs", || KeyspaceCreateOptions::default()).unwrap();
    
    let store = Arc::new(JobStore::new(Arc::new(keyspace)));
    let queue = Queue::new("default".to_string(), store.clone());

    // Push 3 jobs
    // Job 1: Prio 0, Now
    // Job 2: Prio 10, Now (Should come first)
    // Job 3: Prio 0, Future (Should not come yet)

    let mut j1 = Job::new(1, "default".into(), vec![]);
    j1.priority = 0;
    queue.push(j1).unwrap();

    let mut j2 = Job::new(2, "default".into(), vec![]);
    j2.priority = 10;
    queue.push(j2).unwrap();

    let mut j3 = Job::new(3, "default".into(), vec![]);
    j3.run_at = jobs::queue::now_ms() + 10_000;
    queue.push(j3).unwrap();

    // Pop 1: Should be Job 2 (High Prio)
    let pop1 = queue.pop().unwrap();
    assert!(pop1.is_some());
    assert_eq!(pop1.unwrap().id, 2);

    // Pop 2: Should be Job 1 (Low Prio)
    let pop2 = queue.pop().unwrap();
    assert!(pop2.is_some());
    assert_eq!(pop2.unwrap().id, 1);

    // Pop 3: Should be None (Job 3 is future)
    let pop3 = queue.pop().unwrap();
    assert!(pop3.is_none());
}

#[test]
fn test_persistence() {
    let path = tempfile::tempdir().unwrap();
    let db_path = path.path().to_path_buf();
    
    {
        let db = fjall::Database::builder(&db_path).open().unwrap();
        let keyspace = db.keyspace("test_persist", || KeyspaceCreateOptions::default()).unwrap();
        let store = Arc::new(JobStore::new(Arc::new(keyspace)));
        let queue = Queue::new("persist".into(), store.clone());
        
        let j = Job::new(100, "persist".into(), b"data".to_vec());
        queue.push(j).unwrap();
        
        // Manual flush/sync handled by Drop usually, or explicit flush
        db.persist(fjall::PersistMode::SyncAll).unwrap();
    }
    
    // Reopen
    {
        let db = fjall::Database::builder(&db_path).open().unwrap();
        let keyspace = db.keyspace("test_persist", || KeyspaceCreateOptions::default()).unwrap();
        let store = Arc::new(JobStore::new(Arc::new(keyspace)));
        let queue = Queue::new("persist".into(), store.clone());
        
        // Should find job
        let pop = queue.pop().unwrap();
        assert!(pop.is_some());
        let job = pop.unwrap();
        assert_eq!(job.id, 100);
        assert_eq!(job.payload, b"data");
    }
}
