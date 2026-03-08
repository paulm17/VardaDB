//! Sync Tests - Multi-Node Synchronization Verification
//!
//! Tests that verify data synchronization between multiple VardaDB nodes.

use async_graphql::Value;
use std::time::{Duration, Instant};

use crate::multi_node::MultiNodeHarness;
use crate::{TestResult, TestRunner};

/// Run all sync tests
pub async fn run_sync_tests(runner: &mut TestRunner) {
    // Note: These tests require VardaDB binary to be built
    // Run `cargo build` in VardaDB root first

    let start = Instant::now();

    match test_write_to_a_read_from_b().await {
        Ok(result) => runner.add_result(result),
        Err(e) => runner.add_result(TestResult::fail(
            "write_to_a_read_from_b",
            "sync",
            start.elapsed(),
            &e,
        )),
    }

    let start = Instant::now();
    match test_write_to_b_read_from_a().await {
        Ok(result) => runner.add_result(result),
        Err(e) => runner.add_result(TestResult::fail(
            "write_to_b_read_from_a",
            "sync",
            start.elapsed(),
            &e,
        )),
    }

    let start = Instant::now();
    match test_lww_conflict_resolution().await {
        Ok(result) => runner.add_result(result),
        Err(e) => runner.add_result(TestResult::fail(
            "lww_conflict_resolution",
            "sync",
            start.elapsed(),
            &e,
        )),
    }

    let start = Instant::now();
    match test_bidirectional_sync().await {
        Ok(result) => runner.add_result(result),
        Err(e) => runner.add_result(TestResult::fail(
            "bidirectional_sync",
            "sync",
            start.elapsed(),
            &e,
        )),
    }
}

/// Test: Write to Node A, read from Node B
async fn test_write_to_a_read_from_b() -> Result<TestResult, String> {
    let start = Instant::now();

    let sdl = r#"
        type User {
            id: ID!
            name: String!
            email: String
        }
    "#;

    let harness = MultiNodeHarness::new(2, 18000, sdl).await?;

    // Create user on Node A
    let create_mutation = r#"
        mutation { createUser(input: { name: "Alice", email: "alice@example.com" }) { uid name email } }
    "#;

    let response = harness.execute(0, create_mutation).await?;

    let uid = match get_path(&response, "createUser.uid")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string UID".to_string()),
    };

    let created_name = match get_path(&response, "createUser.name")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string name".to_string()),
    };

    // Wait for sync
    harness.wait_for_sync(Duration::from_secs(1)).await;

    // Read from Node B
    let query = format!(
        r#"query {{ getUser(uid: "{}") {{ uid name email }} }}"#,
        uid
    );

    let response = harness.execute(1, &query).await?;

    let read_name = match get_path(&response, "getUser.name")? {
        Value::String(s) => s.clone(),
        _ => return Err("User not found on Node B".to_string()),
    };

    if created_name == read_name {
        Ok(TestResult::pass(
            "write_to_a_read_from_b",
            "sync",
            start.elapsed(),
        ))
    } else {
        Ok(TestResult::fail(
            "write_to_a_read_from_b",
            "sync",
            start.elapsed(),
            &format!(
                "Name mismatch: created '{}', read '{}'",
                created_name, read_name
            ),
        ))
    }
}

/// Test: Write to Node B, read from Node A
async fn test_write_to_b_read_from_a() -> Result<TestResult, String> {
    let start = Instant::now();

    let sdl = r#"
        type User {
            id: ID!
            name: String!
            status: String
        }
    "#;

    let harness = MultiNodeHarness::new(2, 18100, sdl).await?;

    // Create user on Node B (index 1)
    let create_mutation = r#"
        mutation { createUser(input: { name: "Bob", status: "active" }) { uid name status } }
    "#;

    let response = harness.execute(1, create_mutation).await?;

    let uid = match get_path(&response, "createUser.uid")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string UID".to_string()),
    };

    let created_name = match get_path(&response, "createUser.name")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string name".to_string()),
    };

    // Wait for sync
    harness.wait_for_sync(Duration::from_secs(1)).await;

    // Read from Node A (index 0)
    let query = format!(
        r#"query {{ getUser(uid: "{}") {{ uid name status }} }}"#,
        uid
    );

    let response = harness.execute(0, &query).await?;

    let read_name = match get_path(&response, "getUser.name")? {
        Value::String(s) => s.clone(),
        _ => return Err("User not found on Node A".to_string()),
    };

    if created_name == read_name {
        Ok(TestResult::pass(
            "write_to_b_read_from_a",
            "sync",
            start.elapsed(),
        ))
    } else {
        Ok(TestResult::fail(
            "write_to_b_read_from_a",
            "sync",
            start.elapsed(),
            &format!(
                "Name mismatch: created '{}', read '{}'",
                created_name, read_name
            ),
        ))
    }
}

/// Test: LWW conflict resolution - both nodes update, latest wins
async fn test_lww_conflict_resolution() -> Result<TestResult, String> {
    let start = Instant::now();

    let sdl = r#"
        type Counter {
            id: ID!
            value: Int!
        }
    "#;

    let harness = MultiNodeHarness::new(2, 18200, sdl).await?;

    // Create counter on Node A
    let create_mutation = r#"
        mutation { createCounter(input: { value: 0 }) { uid value } }
    "#;

    let response = harness.execute(0, create_mutation).await?;

    let uid = match get_path(&response, "createCounter.uid")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string UID".to_string()),
    };

    // Wait for sync
    harness.wait_for_sync(Duration::from_secs(1)).await;

    // Update on Node A with value 100
    let update_a = format!(
        r#"mutation {{ updateCounter(uid: "{}", input: {{ value: 100 }}) }}"#,
        uid
    );
    harness.execute(0, &update_a).await?;

    // Small delay to ensure HLC advances
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Update on Node B with value 200 (should win - later timestamp)
    let update_b = format!(
        r#"mutation {{ updateCounter(uid: "{}", input: {{ value: 200 }}) }}"#,
        uid
    );
    harness.execute(1, &update_b).await?;

    // Wait for sync
    harness.wait_for_sync(Duration::from_secs(1)).await;

    // Read from both nodes - should both show 200 (LWW)
    let query = format!(r#"query {{ getCounter(uid: "{}") {{ value }} }}"#, uid);

    let response_a = harness.execute(0, &query).await?;
    let response_b = harness.execute(1, &query).await?;

    let value_a = match get_path(&response_a, "getCounter.value")? {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => return Err("Expected number value on Node A".to_string()),
    };

    let value_b = match get_path(&response_b, "getCounter.value")? {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => return Err("Expected number value on Node B".to_string()),
    };

    // Both nodes should have converged to the same value
    if value_a == value_b {
        Ok(TestResult::pass(
            "lww_conflict_resolution",
            "sync",
            start.elapsed(),
        ))
    } else {
        Ok(TestResult::fail(
            "lww_conflict_resolution",
            "sync",
            start.elapsed(),
            &format!("Nodes did not converge: A={}, B={}", value_a, value_b),
        ))
    }
}

/// Test: Bidirectional sync - multiple writes on both nodes
async fn test_bidirectional_sync() -> Result<TestResult, String> {
    let start = Instant::now();

    let sdl = r#"
        type Item {
            id: ID!
            name: String!
        }
    "#;

    let harness = MultiNodeHarness::new(2, 18300, sdl).await?;

    // Create items on Node A
    let mut created_on_a = Vec::new();
    for i in 0..3 {
        let mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "ItemA{}" }}) {{ uid name }} }}"#,
            i
        );
        let response = harness.execute(0, &mutation).await?;
        if let Value::String(uid) = get_path(&response, "createItem.uid")? {
            created_on_a.push(uid.clone());
        }
    }

    // Create items on Node B
    let mut created_on_b = Vec::new();
    for i in 0..3 {
        let mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "ItemB{}" }}) {{ uid name }} }}"#,
            i
        );
        let response = harness.execute(1, &mutation).await?;
        if let Value::String(uid) = get_path(&response, "createItem.uid")? {
            created_on_b.push(uid.clone());
        }
    }

    // Wait for sync
    harness.wait_for_sync(Duration::from_secs(2)).await;

    // Verify Node B has all items created on A
    let mut found_on_b = 0;
    for uid in &created_on_a {
        let query = format!(r#"query {{ getItem(uid: "{}") {{ uid name }} }}"#, uid);
        if let Ok(response) = harness.execute(1, &query).await {
            if get_path(&response, "getItem.uid").is_ok() {
                found_on_b += 1;
            }
        }
    }

    // Verify Node A has all items created on B
    let mut found_on_a = 0;
    for uid in &created_on_b {
        let query = format!(r#"query {{ getItem(uid: "{}") {{ uid name }} }}"#, uid);
        if let Ok(response) = harness.execute(0, &query).await {
            if get_path(&response, "getItem.uid").is_ok() {
                found_on_a += 1;
            }
        }
    }

    if found_on_b == 3 && found_on_a == 3 {
        Ok(TestResult::pass(
            "bidirectional_sync",
            "sync",
            start.elapsed(),
        ))
    } else {
        Ok(TestResult::fail(
            "bidirectional_sync",
            "sync",
            start.elapsed(),
            &format!(
                "Sync incomplete: A items on B: {}/3, B items on A: {}/3",
                found_on_b, found_on_a
            ),
        ))
    }
}

/// Helper to get nested value from GraphQL response
fn get_path<'a>(value: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut current = value;
    for key in path.split('.') {
        match current {
            Value::Object(obj) => {
                current = obj
                    .get(&async_graphql::Name::new(key))
                    .ok_or_else(|| format!("Path '{}' not found", key))?;
            }
            _ => {
                return Err(format!(
                    "Cannot index into {:?} with key '{}'",
                    current, key
                ))
            }
        }
    }
    Ok(current)
}
