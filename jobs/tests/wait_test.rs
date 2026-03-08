use common::setup;
use jobs::Job;
use std::thread;
use std::time::Duration;

mod common;

#[test]
fn test_explicit_delay() {
    let ctx = setup();

    // 1. Push a job and pop it (make it active)
    let job = Job::new(2000, "default".into(), vec![]);
    ctx.queue.push(job.clone()).unwrap();

    let popped = ctx.queue.pop().unwrap().expect("Should have job");
    assert_eq!(popped.id, 2000);

    // 2. Delay it by 1 second
    ctx.queue
        .delay(popped.id, Duration::from_millis(1000))
        .unwrap();

    // 3. Verify it is now Scheduled
    let stored = ctx.store.get_job(2000).unwrap().unwrap();
    match stored.location {
        jobs::JobLocation::Scheduled { .. } => {}
        _ => panic!("Job should be Scheduled, got {:?}", stored.location),
    }
    assert!(stored.run_at > jobs::queue::now_ms());

    // 4. Verify it is NOT available immediately
    assert!(ctx.queue.pop().unwrap().is_none());

    // 5. Wait
    thread::sleep(Duration::from_millis(1100));

    // 6. Verify it comes back
    let active_again = ctx
        .queue
        .pop()
        .unwrap()
        .expect("Job should return after delay");
    assert_eq!(active_again.id, 2000);
}
