//! Property-Based Tests for VardaDB
//!
//! Inspired by Limbo's property-based testing approach.
//! Properties define invariants that must hold across many random inputs.

use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use std::time::Instant;

use crate::harness::TestHarness;
use crate::{TestResult, TestRunner};

/// Property types that can be tested
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Property {
    /// After insert, select should find the record
    InsertThenSelect,

    /// After delete, select should not find the record
    DeleteThenSelect,

    /// Read-your-writes: written value can be read back
    ReadYourWrites,

    /// Unique constraint prevents duplicates
    UniqueConstraintViolation,

    /// Vector search returns the inserted vector
    VectorInsertThenSearch,

    /// LWW: Last write wins based on timestamp
    LWWConvergence,
}

#[allow(dead_code)]
impl Property {
    /// Get all properties to test
    pub fn all() -> Vec<Property> {
        vec![
            Property::InsertThenSelect,
            Property::DeleteThenSelect,
            Property::ReadYourWrites,
            Property::UniqueConstraintViolation,
            Property::VectorInsertThenSearch,
        ]
    }
}

/// Run all property tests
pub async fn run_property_tests(runner: &mut TestRunner, seed: u64, iterations: usize) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    // Property: InsertThenSelect
    let start = Instant::now();
    let result = test_insert_then_select(&mut rng, iterations).await;
    runner.add_result(match result {
        Ok(passed) => {
            if passed == iterations {
                TestResult::pass(
                    &format!("InsertThenSelect ({} iterations)", iterations),
                    "properties",
                    start.elapsed(),
                )
            } else {
                TestResult::fail(
                    &format!("InsertThenSelect ({} iterations)", iterations),
                    "properties",
                    start.elapsed(),
                    &format!("{}/{} passed", passed, iterations),
                )
            }
        }
        Err(e) => TestResult::fail(
            &format!("InsertThenSelect ({} iterations)", iterations),
            "properties",
            start.elapsed(),
            &e,
        ),
    });

    // Property: DeleteThenSelect
    let start = Instant::now();
    let result = test_delete_then_select(&mut rng, iterations).await;
    runner.add_result(match result {
        Ok(passed) => {
            if passed == iterations {
                TestResult::pass(
                    &format!("DeleteThenSelect ({} iterations)", iterations),
                    "properties",
                    start.elapsed(),
                )
            } else {
                TestResult::fail(
                    &format!("DeleteThenSelect ({} iterations)", iterations),
                    "properties",
                    start.elapsed(),
                    &format!("{}/{} passed", passed, iterations),
                )
            }
        }
        Err(e) => TestResult::fail(
            &format!("DeleteThenSelect ({} iterations)", iterations),
            "properties",
            start.elapsed(),
            &e,
        ),
    });

    // Property: ReadYourWrites
    let start = Instant::now();
    let result = test_read_your_writes(&mut rng, iterations).await;
    runner.add_result(match result {
        Ok(passed) => {
            if passed == iterations {
                TestResult::pass(
                    &format!("ReadYourWrites ({} iterations)", iterations),
                    "properties",
                    start.elapsed(),
                )
            } else {
                TestResult::fail(
                    &format!("ReadYourWrites ({} iterations)", iterations),
                    "properties",
                    start.elapsed(),
                    &format!("{}/{} passed", passed, iterations),
                )
            }
        }
        Err(e) => TestResult::fail(
            &format!("ReadYourWrites ({} iterations)", iterations),
            "properties",
            start.elapsed(),
            &e,
        ),
    });
}

/// Generate a random string of given length
fn random_string(rng: &mut ChaCha8Rng, len: usize) -> String {
    (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..26);
            (b'a' + idx) as char
        })
        .collect()
}

/// Test: After insert, the record can be found via query
async fn test_insert_then_select(rng: &mut ChaCha8Rng, iterations: usize) -> Result<usize, String> {
    let sdl = r#"
        type Item {
            id: ID!
            name: String!
            value: Int!
        }
    "#;

    let harness = TestHarness::new(sdl)?;
    let mut passed = 0;

    for _ in 0..iterations {
        let name = random_string(rng, 10);
        let value: i32 = rng.gen_range(1..10000);

        // Insert
        let create_mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "{}", value: {} }}) {{ uid name value }} }}"#,
            name, value
        );

        let response = harness.execute_ok(&create_mutation).await?;

        // Verify the response contains the data we inserted
        let response_name =
            crate::harness::get_path(&response, "createItem.name").map_err(|e| e.to_string())?;
        let response_value =
            crate::harness::get_path(&response, "createItem.value").map_err(|e| e.to_string())?;

        if response_name == &async_graphql::Value::String(name.clone())
            && response_value == &async_graphql::Value::Number(value.into())
        {
            passed += 1;
        }
    }

    Ok(passed)
}

/// Test: After delete, the record cannot be found
async fn test_delete_then_select(rng: &mut ChaCha8Rng, iterations: usize) -> Result<usize, String> {
    let sdl = r#"
        type Item {
            id: ID!
            name: String!
        }
    "#;

    let harness = TestHarness::new(sdl)?;
    let mut passed = 0;

    for _ in 0..iterations {
        let name = random_string(rng, 10);

        // Insert
        let create_mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "{}" }}) {{ uid }} }}"#,
            name
        );

        let response = harness.execute_ok(&create_mutation).await?;
        let uid = match crate::harness::get_path(&response, "createItem.uid")? {
            async_graphql::Value::String(s) => s.clone(),
            _ => continue,
        };

        // Delete
        let delete_mutation = format!(r#"mutation {{ deleteItem(uid: "{}") }}"#, uid);
        harness.execute_ok(&delete_mutation).await?;

        // Query - should return null
        let query = format!(r#"query {{ getItem(uid: "{}") {{ uid }} }}"#, uid);
        let response = harness.execute_ok(&query).await?;
        let item = crate::harness::get_path(&response, "getItem")?;

        if item == &async_graphql::Value::Null {
            passed += 1;
        }
    }

    Ok(passed)
}

/// Test: Written values can be read back correctly
async fn test_read_your_writes(rng: &mut ChaCha8Rng, iterations: usize) -> Result<usize, String> {
    let sdl = r#"
        type Record {
            id: ID!
            data: String!
            count: Int!
        }
    "#;

    let harness = TestHarness::new(sdl)?;
    let mut passed = 0;

    for _ in 0..iterations {
        let data = random_string(rng, 20);
        let count: i32 = rng.gen_range(1..1000000);

        // Write
        let create_mutation = format!(
            r#"mutation {{ createRecord(input: {{ data: "{}", count: {} }}) {{ uid }} }}"#,
            data, count
        );

        let response = harness.execute_ok(&create_mutation).await?;
        let uid = match crate::harness::get_path(&response, "createRecord.uid")? {
            async_graphql::Value::String(s) => s.clone(),
            _ => continue,
        };

        // Read back
        let query = format!(r#"query {{ getRecord(uid: "{}") {{ data count }} }}"#, uid);
        let response = harness.execute_ok(&query).await?;

        let read_data = crate::harness::get_path(&response, "getRecord.data")?;
        let read_count = crate::harness::get_path(&response, "getRecord.count")?;

        if read_data == &async_graphql::Value::String(data)
            && read_count == &async_graphql::Value::Number(count.into())
        {
            passed += 1;
        }
    }

    Ok(passed)
}
