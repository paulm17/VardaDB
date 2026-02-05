use jobs::Job;
use common::setup;

mod common;

#[test]
fn test_push_batch() {
    let ctx = setup();
    
    let mut jobs = Vec::new();
    for i in 0..100 {
        jobs.push(Job::new(1000 + i, "default".into(), vec![]));
    }
    
    // Push Batch
    ctx.queue.push_batch(jobs).unwrap();
    
    // Verify Persistence (sample check)
    let j1 = ctx.store.get_job(1000).unwrap().unwrap();
    assert_eq!(j1.id, 1000);
    
    let j99 = ctx.store.get_job(1099).unwrap().unwrap();
    assert_eq!(j99.id, 1099);
    
    // Verify count? 
    // We can scan ready index to verify count if we want, or rely on samples.
    // Scan up to 101 items (limit)
    // Verify count? 
    // We rely on popping all items below. 
    // Wait, scan_next_ready_job returns Option<(Id, Key)>. It creates iterator but breaks on first val.
    // We don't have a "scan all" exposed yet easily.
    // Let's rely on popping.
    
    // Pop all 100
    for _ in 0..100 {
        assert!(ctx.queue.pop().unwrap().is_some());
    }
    assert!(ctx.queue.pop().unwrap().is_none());
}
