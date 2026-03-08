use common::setup;
use jobs::{Job, JobLocation};

mod common;

#[test]
fn test_scheduled_promotion() {
    let ctx = setup();
    let now = jobs::queue::now_ms();

    // 1. Scheduled Job (Ready in past/now)
    let mut job1 = Job::new(1, "default".into(), vec![]);
    job1.run_at = now - 1000; // Past
    ctx.queue.push(job1).unwrap();

    // 2. Scheduled Job (Future)
    let mut job2 = Job::new(2, "default".into(), vec![]);
    job2.run_at = now + 10_000; // Future
    ctx.queue.push(job2).unwrap();

    // Before Pop: Check Locations
    let saved_j1 = ctx.store.get_job(1).unwrap().unwrap();
    // It depends on how we pushed. Push logic checks `run_at <= now`.
    // So job1 should be Ready immediately if we push with past time.
    // Wait, let's verify `push` logic again.
    // If run_at <= now, it goes to Ready.
    // So to test PROMOTION, we must insert it as Scheduled manually OR push it when it IS future, then wait?
    // Or we force it.

    // Let's force job1 to be Scheduled but with past time (simulating time passed).
    // We can't easily force via push public API if it auto-resolves.
    // But `JobStore` has `put_job`.
    // Let's manually overwrite job1 to be Scheduled.

    let mut job1_sched = saved_j1.clone();
    job1_sched.location = JobLocation::Scheduled {
        queue: "default".into(),
    };
    ctx.store.put_job(&job1_sched).unwrap();

    // Verify it is indeed Scheduled
    let check_j1 = ctx.store.get_job(1).unwrap().unwrap();
    matches!(check_j1.location, JobLocation::Scheduled { .. });

    // Now Pop. Pop triggers promote_scheduled.
    // It should promote job1 (run_at < now) but NOT job2 (run_at > now).

    let pop_result = ctx.queue.pop().unwrap();

    // Should get Job 1
    assert!(pop_result.is_some());
    let popped = pop_result.unwrap();
    assert_eq!(popped.id, 1);

    // Job 2 should still be Scheduled
    let saved_j2 = ctx.store.get_job(2).unwrap().unwrap();
    match saved_j2.location {
        JobLocation::Scheduled { .. } => {}
        _ => panic!("Job 2 should remain Scheduled"),
    }

    // Pop again should be None (Job 2 is future)
    let pop_result_2 = ctx.queue.pop().unwrap();
    assert!(pop_result_2.is_none());
}

#[test]
fn test_scheduled_ordering() {
    let ctx = setup();
    let now = jobs::queue::now_ms();

    // Push 3 jobs to be scheduled
    // J1: Now + 5s
    // J2: Now + 2s
    // J3: Now + 10s

    let mut j1 = Job::new(1, "default".into(), vec![]);
    j1.run_at = now + 5000;
    let mut j2 = Job::new(2, "default".into(), vec![]);
    j2.run_at = now + 2000;
    let mut j3 = Job::new(3, "default".into(), vec![]);
    j3.run_at = now + 10000;

    ctx.queue.push(j1).unwrap();
    ctx.queue.push(j2).unwrap();
    ctx.queue.push(j3).unwrap();

    // Verify all are Scheduled
    // NOTE: We cannot easily verify ORDER in Scheduled Index without `scan` API exposed.
    // But `promote_scheduled` relies on it.

    // Simualte Time Travel: Advance "now" in our check (we pass `now` to promote).
    // We can't change system time, but we can call `promote_scheduled` manually with future time.

    // Promote up to Now + 3s. Should only pick J2.
    let promoted_count = ctx
        .store
        .promote_scheduled("default", now + 3000, 10)
        .unwrap();
    assert_eq!(promoted_count, 1);

    // J2 should be Ready. J1, J3 Scheduled.
    match ctx.store.get_job(2).unwrap().unwrap().location {
        JobLocation::Ready { .. } => {}
        _ => panic!("J2 should be Ready"),
    }
    match ctx.store.get_job(1).unwrap().unwrap().location {
        JobLocation::Scheduled { .. } => {}
        _ => panic!("J1 should be Scheduled"),
    }
}
