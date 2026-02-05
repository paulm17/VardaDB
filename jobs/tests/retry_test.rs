use jobs::{Job, JobLocation, RetryConfig};
use common::setup;

mod common;

#[test]
fn test_retry_backoff() {
    let ctx = setup();
    let now = jobs::queue::now_ms();
    
    // 1. Enqueue Job
    let mut job = Job::new(1, "default".into(), vec![]);
    // Configure Retry: max 3
    job.retry = RetryConfig {
        max_attempts: 3,
        ..RetryConfig::default()
    };
    ctx.queue.push(job).unwrap();
    
    // 2. Pop (Move to Active, attempt 1)
    let popped = ctx.queue.pop().unwrap().expect("Should pop job");
    assert_eq!(popped.attempt, 1);
    
    // 3. Fail (Retry 1)
    ctx.queue.fail_job(popped, "Oops".into()).unwrap();
    
    // 4. Verify Location -> Scheduled
    let saved = ctx.store.get_job(1).unwrap().unwrap();
    match saved.location {
        JobLocation::Scheduled { .. } => {},
        _ => panic!("Job should be Scheduled"),
    }
    
    // 5. Verify RunAt (Backoff)
    // attempt=1. delay = 1^4 + 15 + jitter(1%30=1) = 17s.
    // expected delay is ~17s
    let run_at_diff = saved.run_at - now;
    // Allow slight timing diffs, but should be around 17000
    // System time might advance slightly between `now` capture and `fail_job`.
    assert!(run_at_diff >= 16_000 && run_at_diff <= 18_000, "Delay {} not close to 17000", run_at_diff);
}

#[test]
fn test_max_retries_dlq() {
    let ctx = setup();
    
    // 1. Enqueue Job with Max Attempts = 1
    let mut job = Job::new(2, "default".into(), vec![]);
    job.retry = RetryConfig {
        max_attempts: 1,
        ..RetryConfig::default()
    };
    ctx.queue.push(job).unwrap();
    
    // 2. Pop (Active, attempt 1)
    let popped = ctx.queue.pop().unwrap().expect("Should pop job");
    assert_eq!(popped.attempt, 1); // Current attempt is 1. Max is 1.
    
    // 3. Fail
    // Attempt (1) is NOT < Max Attempts (1). So it should FAIL to DLQ.
    // Wait. If max=1, and we are on attempt 1.
    // Logic: `if job.attempt < job.retry.max_attempts`
    // 1 < 1 is FALSE.
    // So it goes to DLQ. Correct.
    
    ctx.queue.fail_job(popped, "Fatal".into()).unwrap();
    
    // 4. Verify DLQ
    let saved = ctx.store.get_job(2).unwrap().unwrap();
    match saved.location {
        JobLocation::Dlq { .. } => {},
        _ => panic!("Job should be in DLQ. Location: {:?}", saved.location),
    }
}

#[test]
fn test_retry_increases_delay() {
    let ctx = setup();
    let now = jobs::queue::now_ms();
    
    // Manually insert job at attempt 3
    let mut job = Job::new(3, "default".into(), vec![]);
    job.attempt = 3;
    job.location = JobLocation::Active { started_at: now, worker_id: 1 };
    // Put directly to store (bypass queue push which resets?) No, push doesn't reset attempt.
    // But push overwrites location.
    // Let's manually put.
    ctx.store.put_job(&job).unwrap();
    
    // Fail it
    ctx.queue.fail_job(job, "Fail".into()).unwrap();
    
    // Verify delay
    // attempt 3. 3^4 = 81. + 15 + jitter. ~96s.
    let saved = ctx.store.get_job(3).unwrap().unwrap();
    let diff = saved.run_at - now;
    assert!(diff > 90_000, "Delay {} should be > 90s", diff);
}
