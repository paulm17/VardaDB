use jobs::{Job, JobLocation};
use common::setup;

mod common;

#[test]
fn test_client_push() {
    let ctx = setup();
    
    // Test Basic Push
    let job = Job::new(1, "default".into(), vec![]);
    ctx.queue.push(job.clone()).unwrap();
    
    let saved_job = ctx.store.get_job(1).unwrap().unwrap();
    assert_eq!(saved_job.id, 1);
    assert_eq!(saved_job.queue, "default");
    // Verify Location
    match saved_job.location {
        JobLocation::Ready { queue } => assert_eq!(queue, "default"),
        _ => panic!("Job should be in Ready state"),
    }
}

#[test]
fn test_client_push_scheduled() {
    let ctx = setup();
    let now = jobs::queue::now_ms();
    
    // Test Scheduled Push (Future)
    let mut job = Job::new(2, "default".into(), vec![]);
    job.run_at = now + 10_000; // 10s in future
    ctx.queue.push(job).unwrap();
    
    let saved_job = ctx.store.get_job(2).unwrap().unwrap();
    // Verify Location
    match saved_job.location {
        JobLocation::Scheduled { queue } => assert_eq!(queue, "default"),
        _ => panic!("Job should be in Scheduled state"),
    }
}

#[test]
fn test_client_push_bulk() {
    let ctx = setup();
    
    // Simulate push_bulk (loop)
    for i in 0..100 {
        let job = Job::new(100 + i, "default".into(), vec![]);
        ctx.queue.push(job).unwrap();
    }
    
    // Verify count (naive scan for now as we don't have count API on specific queue easily without scan)
    // But we can check a few
    let saved_job = ctx.store.get_job(100).unwrap().unwrap();
    assert!(saved_job.id == 100);
    let saved_job_last = ctx.store.get_job(199).unwrap().unwrap();
    assert!(saved_job_last.id == 199);
}

#[test]
fn test_client_push_validation() {
    let ctx = setup();
    
    let job = Job::new(3, "other_queue".into(), vec![]);
    // Queue mismatch
    let res = ctx.queue.push(job);
    assert!(res.is_err());
    assert_eq!(res.err(), Some("Job queue 'other_queue' does not match queue 'default'".into()));
}
