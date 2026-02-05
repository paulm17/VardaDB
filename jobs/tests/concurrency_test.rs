use jobs::Job;
use common::setup;

mod common;

#[test]
fn test_concurrency_limit() {
    let ctx = setup();
    
    // Set limit to 1
    ctx.queue.set_concurrency_limit(1);
    
    // Push 2 jobs
    ctx.queue.push(Job::new(100, "default".into(), vec![])).unwrap();
    ctx.queue.push(Job::new(101, "default".into(), vec![])).unwrap();
    
    // Pop 1st (Active: 0 -> 1)
    let job1 = ctx.queue.pop().unwrap().expect("Should get job 1");
    assert_eq!(job1.id, 100);
    
    // Explicitly verify store count to silence unused warning and verify logic
    let active_count = ctx.store.count_active_jobs("default").unwrap();
    assert_eq!(active_count, 1, "Store should report 1 active job");
    
    // Pop 2nd (Active: 1 >= 1) -> Should be blocked
    let job2 = ctx.queue.pop().unwrap();
    assert!(job2.is_none(), "Should be blocked by concurrency limit");
    
    // Ack 1st (Active: 1 -> 0)
    ctx.queue.ack(job1.id).unwrap();
    
    // Pop 2nd (Active: 0 -> 1) -> Should work now
    let job2_succ = ctx.queue.pop().unwrap().expect("Should get job 2 now");
    assert_eq!(job2_succ.id, 101);
}
