mod common;

use jobs::{Job, JobStore, Queue};
use std::sync::Arc;
use common::MemoryKvStore;

#[test]
fn test_priority_ordering() {
    let kv = MemoryKvStore::new();
    let store = Arc::new(JobStore::new(Arc::new(kv)));
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
fn test_persistence_in_memory() {
    // With MemoryKvStore, data persists across queue operations within the same process.
    // This test validates the store retains data correctly.
    let kv = MemoryKvStore::new();
    let kv_clone = kv.clone(); // Clone shares the same Arc<Mutex<HashMap>>
    
    {
        let store = Arc::new(JobStore::new(Arc::new(kv)));
        let queue = Queue::new("persist".into(), store.clone());
        
        let j = Job::new(100, "persist".into(), b"data".to_vec());
        queue.push(j).unwrap();
    }
    
    // Reopen with same underlying store (simulates persistence)
    {
        let store = Arc::new(JobStore::new(Arc::new(kv_clone)));
        let queue = Queue::new("persist".into(), store.clone());
        
        // Should find job
        let pop = queue.pop().unwrap();
        assert!(pop.is_some());
        let job = pop.unwrap();
        assert_eq!(job.id, 100);
        assert_eq!(job.payload, b"data");
    }
}
