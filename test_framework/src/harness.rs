//! Test Harness - Fixtures, assertions, and utilities for testing VardaDB.
//!
//! This module provides:
//! - `TestHarness`: A reusable test environment with storage, schema, and resolver
//! - Custom assertions for GraphQL responses
//! - CRUD test implementations

use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use async_graphql::Value;

use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use vardadb::engine::schema::Schema;
use vardadb::realtime::bus::EventBus;

use crate::{TestRunner, TestResult};

/// A reusable test harness that provides storage, schema, and resolver.
#[allow(dead_code)]
pub struct TestHarness {
    pub storage: Arc<Storage>,
    pub schema: Schema,
    pub resolver: FjallResolver,
    pub event_bus: EventBus,
    _temp_dir: TempDir, // Kept alive to prevent cleanup
}

impl TestHarness {
    /// Create a new test harness with the given SDL schema.
    pub fn new(sdl: &str) -> Result<Self, String> {
        let temp_dir = TempDir::new().map_err(|e| e.to_string())?;
        let storage = Arc::new(
            Storage::new(temp_dir.path().to_str().unwrap(), Some(1))
                .map_err(|e| e.to_string())?
        );
        
        let event_bus = EventBus::new();
        let resolver = FjallResolver::with_bus(storage.clone(), event_bus.clone());
        let schema = Schema::load_with_resolver(sdl, resolver.clone())?;

        Ok(Self {
            storage,
            schema,
            resolver,
            event_bus,
            _temp_dir: temp_dir,
        })
    }

    /// Execute a GraphQL query/mutation and return the response as JSON Value.
    pub async fn execute(&self, query: &str) -> Value {
        let response = self.schema.execute(query).await;
        
        // Convert response to Value
        if !response.errors.is_empty() {
            // Return errors as a Value
            let errors: Vec<Value> = response.errors.iter()
                .map(|e| Value::String(e.message.clone()))
                .collect();
            return Value::Object({
                let mut map = async_graphql::indexmap::IndexMap::new();
                map.insert(async_graphql::Name::new("errors"), Value::List(errors));
                if let Some(data) = response.data.into_json().ok() {
                    map.insert(async_graphql::Name::new("data"), serde_json_to_value(data));
                }
                map
            });
        }

        response.data.into_json()
            .map(serde_json_to_value)
            .unwrap_or(Value::Null)
    }

    /// Execute a query and expect no errors.
    pub async fn execute_ok(&self, query: &str) -> Result<Value, String> {
        let response = self.schema.execute(query).await;
        
        if !response.errors.is_empty() {
            return Err(response.errors.iter()
                .map(|e| e.message.clone())
                .collect::<Vec<_>>()
                .join(", "));
        }

        response.data.into_json()
            .map(serde_json_to_value)
            .map_err(|e| e.to_string())
    }

    /// Execute a query and expect an error containing the given message.
    pub async fn execute_expect_error(&self, query: &str, expected_msg: &str) -> Result<(), String> {
        let response = self.schema.execute(query).await;
        
        if response.errors.is_empty() {
            return Err("Expected error but got success".to_string());
        }

        let has_expected = response.errors.iter()
            .any(|e| e.message.contains(expected_msg));

        if has_expected {
            Ok(())
        } else {
            Err(format!(
                "Expected error containing '{}', got: {:?}",
                expected_msg,
                response.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
            ))
        }
    }
}

/// Convert serde_json::Value to async_graphql::Value
fn serde_json_to_value(json: serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                Value::Number(async_graphql::Number::from_f64(f).unwrap_or(async_graphql::Number::from(0)))
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(arr) => {
            Value::List(arr.into_iter().map(serde_json_to_value).collect())
        }
        serde_json::Value::Object(obj) => {
            let map: async_graphql::indexmap::IndexMap<async_graphql::Name, Value> = obj
                .into_iter()
                .map(|(k, v)| (async_graphql::Name::new(&k), serde_json_to_value(v)))
                .collect();
            Value::Object(map)
        }
    }
}

// ============================================================================
// Assertions
// ============================================================================

/// Assert that a GraphQL response has no errors.
#[allow(dead_code)]
pub fn assert_no_errors(response: &Value) -> Result<(), String> {
    if let Value::Object(obj) = response {
        if let Some(Value::List(errors)) = obj.get(&async_graphql::Name::new("errors")) {
            if !errors.is_empty() {
                return Err(format!("Expected no errors, got: {:?}", errors));
            }
        }
    }
    Ok(())
}

/// Assert that response data at a path equals expected value.
#[allow(dead_code)]
pub fn assert_data_equals(response: &Value, path: &str, expected: &Value) -> Result<(), String> {
    let actual = get_path(response, path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!("At path '{}': expected {:?}, got {:?}", path, expected, actual))
    }
}

/// Get a value at a dot-separated path (e.g., "data.createUser.id")
pub fn get_path<'a>(value: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut current = value;
    for key in path.split('.') {
        match current {
            Value::Object(obj) => {
                current = obj.get(&async_graphql::Name::new(key))
                    .ok_or_else(|| format!("Path '{}' not found", key))?;
            }
            _ => return Err(format!("Cannot index into {:?} with key '{}'", current, key)),
        }
    }
    Ok(current)
}

// ============================================================================
// CRUD Tests
// ============================================================================

/// Run CRUD tests using the test harness.
pub async fn run_crud_tests(runner: &mut TestRunner, _seed: u64) {
    // Test 1: Create User
    let start = Instant::now();
    let result = test_create_user().await;
    runner.add_result(match result {
        Ok(_) => TestResult::pass("create_user", "crud", start.elapsed()),
        Err(e) => TestResult::fail("create_user", "crud", start.elapsed(), &e),
    });

    // Test 2: Read by ID
    let start = Instant::now();
    let result = test_read_by_id().await;
    runner.add_result(match result {
        Ok(_) => TestResult::pass("read_by_id", "crud", start.elapsed()),
        Err(e) => TestResult::fail("read_by_id", "crud", start.elapsed(), &e),
    });

    // Test 3: Update partial
    let start = Instant::now();
    let result = test_update_partial().await;
    runner.add_result(match result {
        Ok(_) => TestResult::pass("update_partial", "crud", start.elapsed()),
        Err(e) => TestResult::fail("update_partial", "crud", start.elapsed(), &e),
    });

    // Test 4: Delete
    let start = Instant::now();
    let result = test_delete().await;
    runner.add_result(match result {
        Ok(_) => TestResult::pass("delete", "crud", start.elapsed()),
        Err(e) => TestResult::fail("delete", "crud", start.elapsed(), &e),
    });

    // Test 5: Unique constraint violation
    let start = Instant::now();
    let result = test_unique_constraint().await;
    runner.add_result(match result {
        Ok(_) => TestResult::pass("unique_constraint", "crud", start.elapsed()),
        Err(e) => TestResult::fail("unique_constraint", "crud", start.elapsed(), &e),
    });
}

async fn test_create_user() -> Result<(), String> {
    let sdl = r#"
        type User {
            id: ID!
            name: String!
            email: String! @unique
        }
    "#;
    
    let harness = TestHarness::new(sdl)?;
    
    let response = harness.execute_ok(r#"
        mutation {
            createUser(input: { name: "Alice", email: "alice@example.com" }) {
                uid
                name
                email
            }
        }
    "#).await?;

    // Verify created user has a UID
    get_path(&response, "createUser.uid")?;
    Ok(())
}

async fn test_read_by_id() -> Result<(), String> {
    let sdl = r#"
        type User {
            id: ID!
            name: String!
        }
    "#;
    
    let harness = TestHarness::new(sdl)?;
    
    // Create user
    let create_response = harness.execute_ok(r#"
        mutation {
            createUser(input: { name: "Bob" }) {
                uid
            }
        }
    "#).await?;

    let uid = match get_path(&create_response, "createUser.uid")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string UID".to_string()),
    };

    // Read user by UID
    let query = format!(r#"
        query {{
            getUser(uid: "{}") {{
                uid
                name
            }}
        }}
    "#, uid);

    let read_response = harness.execute_ok(&query).await?;
    let name = get_path(&read_response, "getUser.name")?;
    
    if name != &Value::String("Bob".to_string()) {
        return Err(format!("Expected name 'Bob', got {:?}", name));
    }

    Ok(())
}

async fn test_update_partial() -> Result<(), String> {
    let sdl = r#"
        type User {
            id: ID!
            name: String!
            age: Int
        }
    "#;
    
    let harness = TestHarness::new(sdl)?;
    
    // Create user
    let create_response = harness.execute_ok(r#"
        mutation {
            createUser(input: { name: "Charlie", age: 25 }) {
                uid
            }
        }
    "#).await?;

    let uid = match get_path(&create_response, "createUser.uid")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string UID".to_string()),
    };

    // Update only age (VardaDB returns Boolean for update mutations)
    let mutation = format!(r#"
        mutation {{
            updateUser(uid: "{}", input: {{ age: 30 }})
        }}
    "#, uid);

    harness.execute_ok(&mutation).await?;
    
    // Query to verify the update worked
    let query = format!(r#"
        query {{
            getUser(uid: "{}") {{
                name
                age
            }}
        }}
    "#, uid);
    
    let read_response = harness.execute_ok(&query).await?;
    
    // Verify name unchanged, age updated
    let name = get_path(&read_response, "getUser.name")?;
    let age = get_path(&read_response, "getUser.age")?;
    
    if name != &Value::String("Charlie".to_string()) {
        return Err(format!("Name should be unchanged, got {:?}", name));
    }
    
    if age != &Value::Number(30.into()) {
        return Err(format!("Age should be 30, got {:?}", age));
    }

    Ok(())
}

async fn test_delete() -> Result<(), String> {
    let sdl = r#"
        type User {
            id: ID!
            name: String!
        }
    "#;
    
    let harness = TestHarness::new(sdl)?;
    
    // Create user
    let create_response = harness.execute_ok(r#"
        mutation {
            createUser(input: { name: "Dave" }) {
                uid
            }
        }
    "#).await?;

    let uid = match get_path(&create_response, "createUser.uid")? {
        Value::String(s) => s.clone(),
        _ => return Err("Expected string UID".to_string()),
    };

    // Delete user
    let mutation = format!(r#"
        mutation {{
            deleteUser(uid: "{}")
        }}
    "#, uid);

    harness.execute_ok(&mutation).await?;

    // Verify user is gone
    let query = format!(r#"
        query {{
            getUser(uid: "{}") {{
                uid
            }}
        }}
    "#, uid);

    let read_response = harness.execute_ok(&query).await?;
    let user = get_path(&read_response, "getUser")?;
    
    if user != &Value::Null {
        return Err(format!("Expected null after delete, got {:?}", user));
    }

    Ok(())
}

async fn test_unique_constraint() -> Result<(), String> {
    let sdl = r#"
        type User {
            id: ID!
            email: String! @unique
        }
    "#;
    
    let harness = TestHarness::new(sdl)?;
    
    // Create first user
    harness.execute_ok(r#"
        mutation {
            createUser(input: { email: "unique@example.com" }) {
                uid
            }
        }
    "#).await?;

    // Try to create second user with same email - should fail
    harness.execute_expect_error(
        r#"
            mutation {
                createUser(input: { email: "unique@example.com" }) {
                    uid
                }
            }
        "#,
        "unique" // Error message should contain "unique"
    ).await?;

    Ok(())
}
