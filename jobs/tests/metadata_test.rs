use common::setup;
use jobs::Job;
use std::collections::HashMap;

mod common;

#[test]
fn test_update_metadata() {
    let ctx = setup();

    // 1. Enqueue Job
    let job = Job::new(1, "default".into(), vec![]);
    ctx.queue.push(job).unwrap();

    // 2. Update Metadata
    let mut updates = HashMap::new();
    updates.insert("user_id".to_string(), "123".to_string());
    updates.insert("req_id".to_string(), "abc".to_string());

    ctx.store.update_job_meta(1, updates).unwrap();

    // 3. Verify
    let saved = ctx.store.get_job(1).unwrap().unwrap();
    assert_eq!(saved.meta.get("user_id").map(|s| s.as_str()), Some("123"));
    assert_eq!(saved.meta.get("req_id").map(|s| s.as_str()), Some("abc"));

    // 4. Update Again (Append/Overwrite)
    let mut updates2 = HashMap::new();
    updates2.insert("user_id".to_string(), "456".to_string()); // Overwrite
    updates2.insert("trace_id".to_string(), "xyz".to_string()); // New

    ctx.store.update_job_meta(1, updates2).unwrap();

    let saved2 = ctx.store.get_job(1).unwrap().unwrap();
    assert_eq!(saved2.meta.get("user_id").map(|s| s.as_str()), Some("456"));
    assert_eq!(saved2.meta.get("req_id").map(|s| s.as_str()), Some("abc")); // Persists
    assert_eq!(saved2.meta.get("trace_id").map(|s| s.as_str()), Some("xyz"));
}

#[test]
fn test_job_logging() {
    let ctx = setup();

    // 1. Append Logs (Job doesn't need to exist to log, but logically it should)
    // Our implementation doesn't check existence, just writes key.

    ctx.store
        .append_log(10, "Started processing".into())
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    ctx.store.append_log(10, "Step 1 complete".into()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    ctx.store.append_log(10, "Finished".into()).unwrap();

    // 2. Fetch Logs
    let logs = ctx.store.get_logs(10).unwrap();
    assert_eq!(logs.len(), 3);

    assert_eq!(logs[0].1, "Started processing");
    assert_eq!(logs[1].1, "Step 1 complete");
    assert_eq!(logs[2].1, "Finished");

    // Verify Timestamps increasing
    assert!(logs[0].0 < logs[1].0);
    assert!(logs[1].0 < logs[2].0);
}
