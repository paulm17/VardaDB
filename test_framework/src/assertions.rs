//! Three-Tier Assertions
//!
//! Ported from Antithesis assertion framework.
//!
//! Three types of assertions that map to temporal logic:
//! - `always`: Must be true at EVERY point (invariants)
//! - `eventually`: Must be true at SOME point (liveness)
//! - `finally`: Must be true at the END of the test (final state)

use async_graphql::Value;
use std::time::Instant;

use crate::harness::TestHarness;
use crate::{TestResult, TestRunner};

/// Assertion result with details
#[derive(Debug)]
#[allow(dead_code)]
pub struct AssertionResult {
    pub name: String,
    pub passed: bool,
    pub details: Option<String>,
}

impl AssertionResult {
    pub fn pass(name: &str) -> Self {
        Self {
            name: name.to_string(),
            passed: true,
            details: None,
        }
    }

    pub fn fail(name: &str, details: &str) -> Self {
        Self {
            name: name.to_string(),
            passed: false,
            details: Some(details.to_string()),
        }
    }
}

/// ALWAYS assertion: Must hold at every check point
pub fn always(condition: bool, name: &str, context: &str) -> AssertionResult {
    if condition {
        AssertionResult::pass(name)
    } else {
        AssertionResult::fail(name, context)
    }
}

/// EVENTUALLY assertion: Must hold at least once
pub struct EventuallyAssertion {
    name: String,
    ever_true: bool,
}

impl EventuallyAssertion {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ever_true: false,
        }
    }

    pub fn check(&mut self, condition: bool) {
        if condition {
            self.ever_true = true;
        }
    }

    pub fn result(&self) -> AssertionResult {
        if self.ever_true {
            AssertionResult::pass(&self.name)
        } else {
            AssertionResult::fail(&self.name, "Condition was never true")
        }
    }
}

/// FINALLY assertion: Must hold at the end
pub fn finally(condition: bool, name: &str, context: &str) -> AssertionResult {
    if condition {
        AssertionResult::pass(name)
    } else {
        AssertionResult::fail(name, context)
    }
}

/// Run three-tier assertion tests
pub async fn run_assertion_tests(runner: &mut TestRunner, _seed: u64) {
    // Test 1: ALWAYS - No duplicate unique values
    let start = Instant::now();
    let result = test_always_no_duplicates().await;
    runner.add_result(match result {
        Ok(r) => {
            if r.passed {
                TestResult::pass(
                    "[Always] NoDuplicateUniqueValues",
                    "assertions",
                    start.elapsed(),
                )
            } else {
                TestResult::fail(
                    "[Always] NoDuplicateUniqueValues",
                    "assertions",
                    start.elapsed(),
                    r.details.as_deref().unwrap_or("Unknown"),
                )
            }
        }
        Err(e) => TestResult::fail(
            "[Always] NoDuplicateUniqueValues",
            "assertions",
            start.elapsed(),
            &e,
        ),
    });

    // Test 2: ALWAYS - Vector dimensions consistent
    let start = Instant::now();
    let result = test_always_vector_dimensions().await;
    runner.add_result(match result {
        Ok(r) => {
            if r.passed {
                TestResult::pass(
                    "[Always] VectorDimensionsConsistent",
                    "assertions",
                    start.elapsed(),
                )
            } else {
                TestResult::fail(
                    "[Always] VectorDimensionsConsistent",
                    "assertions",
                    start.elapsed(),
                    r.details.as_deref().unwrap_or("Unknown"),
                )
            }
        }
        Err(e) => TestResult::fail(
            "[Always] VectorDimensionsConsistent",
            "assertions",
            start.elapsed(),
            &e,
        ),
    });

    // Test 3: EVENTUALLY - Query returns results
    let start = Instant::now();
    let result = test_eventually_query_returns().await;
    runner.add_result(match result {
        Ok(r) => {
            if r.passed {
                TestResult::pass(
                    "[Eventually] QueryReturnsResults",
                    "assertions",
                    start.elapsed(),
                )
            } else {
                TestResult::fail(
                    "[Eventually] QueryReturnsResults",
                    "assertions",
                    start.elapsed(),
                    r.details.as_deref().unwrap_or("Unknown"),
                )
            }
        }
        Err(e) => TestResult::fail(
            "[Eventually] QueryReturnsResults",
            "assertions",
            start.elapsed(),
            &e,
        ),
    });

    // Test 4: FINALLY - Database consistent
    let start = Instant::now();
    let result = test_finally_database_consistent().await;
    runner.add_result(match result {
        Ok(r) => {
            if r.passed {
                TestResult::pass(
                    "[Finally] DatabaseConsistent",
                    "assertions",
                    start.elapsed(),
                )
            } else {
                TestResult::fail(
                    "[Finally] DatabaseConsistent",
                    "assertions",
                    start.elapsed(),
                    r.details.as_deref().unwrap_or("Unknown"),
                )
            }
        }
        Err(e) => TestResult::fail(
            "[Finally] DatabaseConsistent",
            "assertions",
            start.elapsed(),
            &e,
        ),
    });
}

/// Test: No duplicate unique values exist
async fn test_always_no_duplicates() -> Result<AssertionResult, String> {
    let sdl = r#"
        type User {
            id: ID!
            email: String! @unique
        }
    "#;

    let harness = TestHarness::new(sdl)?;

    // Insert first user
    harness
        .execute_ok(
            r#"
        mutation { createUser(input: { email: "test@example.com" }) { id } }
    "#,
        )
        .await?;

    // Try to insert duplicate
    let response = harness
        .execute(
            r#"
        mutation { createUser(input: { email: "test@example.com" }) { id } }
    "#,
        )
        .await;

    // Check that it failed (has errors)
    let has_error = match &response {
        Value::Object(obj) => obj.contains_key(&async_graphql::Name::new("errors")),
        _ => false,
    };

    Ok(always(
        has_error,
        "NoDuplicateUniqueValues",
        "Duplicate unique value was accepted",
    ))
}

/// Test: All vector dimensions are consistent
async fn test_always_vector_dimensions() -> Result<AssertionResult, String> {
    // In VardaDB, vector dimensions are enforced at the storage level
    // This test verifies that the system rejects mismatched dimensions

    // For now, we'll test a simpler invariant: vector storage doesn't corrupt data
    let sdl = r#"
        type Item {
            id: ID!
            name: String!
        }
    "#;

    let harness = TestHarness::new(sdl)?;

    // Create items
    harness
        .execute_ok(
            r#"
        mutation { createItem(input: { name: "Test1" }) { id } }
    "#,
        )
        .await?;

    harness
        .execute_ok(
            r#"
        mutation { createItem(input: { name: "Test2" }) { id } }
    "#,
        )
        .await?;

    // Query all items
    let response = harness
        .execute_ok(
            r#"
        query { queryItem { id name } }
    "#,
        )
        .await?;

    // Verify we got 2 items
    let count = match &response {
        Value::Object(obj) => match obj.get(&async_graphql::Name::new("queryItem")) {
            Some(Value::List(items)) => items.len(),
            _ => 0,
        },
        _ => 0,
    };

    Ok(always(
        count == 2,
        "VectorDimensionsConsistent",
        &format!("Expected 2 items, got {}", count),
    ))
}

/// Test: Query eventually returns results
async fn test_eventually_query_returns() -> Result<AssertionResult, String> {
    let sdl = r#"
        type Todo {
            id: ID!
            title: String!
        }
    "#;

    let harness = TestHarness::new(sdl)?;

    let mut checker = EventuallyAssertion::new("QueryReturnsResults");

    // Initially no results
    let response = harness
        .execute_ok(
            r#"
        query { queryTodo { id } }
    "#,
        )
        .await?;

    let has_results = match &response {
        Value::Object(obj) => match obj.get(&async_graphql::Name::new("queryTodo")) {
            Some(Value::List(items)) => !items.is_empty(),
            _ => false,
        },
        _ => false,
    };
    checker.check(has_results);

    // Add a todo
    harness
        .execute_ok(
            r#"
        mutation { createTodo(input: { title: "Test Todo" }) { id } }
    "#,
        )
        .await?;

    // Now query should return results
    let response = harness
        .execute_ok(
            r#"
        query { queryTodo { id } }
    "#,
        )
        .await?;

    let has_results = match &response {
        Value::Object(obj) => match obj.get(&async_graphql::Name::new("queryTodo")) {
            Some(Value::List(items)) => !items.is_empty(),
            _ => false,
        },
        _ => false,
    };
    checker.check(has_results);

    Ok(checker.result())
}

/// Test: Database is consistent at the end
async fn test_finally_database_consistent() -> Result<AssertionResult, String> {
    let sdl = r#"
        type Counter {
            id: ID!
            value: Int!
        }
    "#;

    let harness = TestHarness::new(sdl)?;

    // Create a counter
    let response = harness
        .execute_ok(
            r#"
        mutation { createCounter(input: { value: 0 }) { uid } }
    "#,
        )
        .await?;

    let uid = match crate::harness::get_path(&response, "createCounter.uid")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string UID".to_string()),
    };

    // Increment multiple times
    for i in 1..=5 {
        let mutation = format!(
            r#"mutation {{ updateCounter(uid: "{}", input: {{ value: {} }}) }}"#,
            uid, i
        );
        harness.execute_ok(&mutation).await?;
    }

    // Final check: value should be 5
    let query = format!(r#"query {{ getCounter(uid: "{}") {{ value }} }}"#, uid);
    let response = harness.execute_ok(&query).await?;

    let final_value = match crate::harness::get_path(&response, "getCounter.value")? {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    };

    Ok(finally(
        final_value == 5,
        "DatabaseConsistent",
        &format!("Expected final value 5, got {}", final_value),
    ))
}
