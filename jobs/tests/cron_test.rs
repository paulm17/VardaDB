use common::setup;
use std::thread;
use std::time::Duration;

mod common;

#[test]
fn test_cron_registration() {
    let ctx = setup();

    // Register a cron for every 1 second
    // Trigger.dev uses 6-part cron? or standard 5? `cron` crate supports quartz (seconds included).
    // "* * * * * *" is 6 parts: sec, min, hour, dom, mon, dow.
    // "0/1 * * * * *" should be every second.
    // quartz: sec min hour dom mon dow year(opt)

    let expr = "0/1 * * * * * *"; // Run every second

    ctx.queue
        .register_cron(
            "test-cron".to_string(),
            expr.to_string(),
            "default".to_string(),
            vec![],
        )
        .unwrap();

    // Verify stored
    let crons = ctx.store.get_crons().unwrap();
    assert_eq!(crons.len(), 1);
    assert_eq!(crons[0].name, "test-cron");
    assert!(crons[0].next_run > 0);
}

#[test]
fn test_cron_trigger() {
    let ctx = setup();

    // Register cron
    let expr = "0/1 * * * * * *"; // Every second
    ctx.queue
        .register_cron(
            "trigger-test".to_string(),
            expr.to_string(),
            "default".to_string(),
            vec![],
        )
        .unwrap();

    // Check initial state
    let crons = ctx.store.get_crons().unwrap();
    let initial_next = crons[0].next_run;

    // Wait for next second + buffer
    thread::sleep(Duration::from_millis(1100));

    // Trigger
    let count = ctx.queue.trigger_crons().unwrap();
    assert_eq!(count, 1, "Should trigger 1 job");

    // Verify job enqueued
    let job = ctx.queue.pop().unwrap().expect("Should have a job");
    assert_eq!(job.queue, "default");

    // Verify next run updated
    let crons_after = ctx.store.get_crons().unwrap();
    assert!(crons_after[0].next_run > initial_next);
}
