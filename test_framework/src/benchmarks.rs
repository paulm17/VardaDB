//! Performance Benchmarks
//!
//! Measures CRUD operation latency and throughput.
//! Inspired by TrailBase's benchmarking methodology.

use std::time::{Duration, Instant};

use crate::harness::TestHarness;
use crate::{TestResult, TestRunner};

/// Benchmark configuration
pub struct BenchmarkConfig {
    /// Number of warmup iterations
    pub warmup_iterations: usize,
    /// Number of measurement iterations
    pub iterations: usize,
    /// Batch sizes for bulk operations
    pub batch_sizes: Vec<usize>,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            warmup_iterations: 10,
            iterations: 100,
            batch_sizes: vec![10, 100, 1000],
        }
    }
}

/// Benchmark result with statistics
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BenchmarkResult {
    pub name: String,
    pub iterations: usize,
    pub total_duration: Duration,
    pub mean: Duration,
    pub median: Duration,
    pub p90: Duration,
    pub p99: Duration,
    pub throughput: f64, // operations per second
}

impl BenchmarkResult {
    pub fn from_samples(name: &str, samples: &[Duration]) -> Self {
        let mut sorted = samples.to_vec();
        sorted.sort();

        let n = sorted.len();
        let total: Duration = sorted.iter().sum();
        let mean = total / n as u32;

        let median = if n % 2 == 0 {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2
        } else {
            sorted[n / 2]
        };

        let p90_idx = (n as f64 * 0.90) as usize;
        let p99_idx = (n as f64 * 0.99) as usize;

        let p90 = sorted[p90_idx.min(n - 1)];
        let p99 = sorted[p99_idx.min(n - 1)];

        let throughput = n as f64 / total.as_secs_f64();

        Self {
            name: name.to_string(),
            iterations: n,
            total_duration: total,
            mean,
            median,
            p90,
            p99,
            throughput,
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "p50={:.2}ms, p90={:.2}ms, p99={:.2}ms, throughput={:.0}/s",
            self.median.as_secs_f64() * 1000.0,
            self.p90.as_secs_f64() * 1000.0,
            self.p99.as_secs_f64() * 1000.0,
            self.throughput
        )
    }
}

/// Run all benchmarks
pub async fn run_benchmarks(runner: &mut TestRunner, _seed: u64) {
    let config = BenchmarkConfig::default();

    // Benchmark 1: Single Insert
    let start = Instant::now();
    let result = benchmark_single_insert(&config).await;
    match result {
        Ok(bench) => {
            runner.add_result(TestResult::pass(
                &format!("insert_single: {}", bench.summary()),
                "benchmarks",
                start.elapsed(),
            ));
        }
        Err(e) => {
            runner.add_result(TestResult::fail(
                "insert_single",
                "benchmarks",
                start.elapsed(),
                &e,
            ));
        }
    }

    // Benchmark 2: Read by ID
    let start = Instant::now();
    let result = benchmark_read_by_id(&config).await;
    match result {
        Ok(bench) => {
            runner.add_result(TestResult::pass(
                &format!("read_by_id: {}", bench.summary()),
                "benchmarks",
                start.elapsed(),
            ));
        }
        Err(e) => {
            runner.add_result(TestResult::fail(
                "read_by_id",
                "benchmarks",
                start.elapsed(),
                &e,
            ));
        }
    }

    // Benchmark 3: Update
    let start = Instant::now();
    let result = benchmark_update(&config).await;
    match result {
        Ok(bench) => {
            runner.add_result(TestResult::pass(
                &format!("update: {}", bench.summary()),
                "benchmarks",
                start.elapsed(),
            ));
        }
        Err(e) => {
            runner.add_result(TestResult::fail(
                "update",
                "benchmarks",
                start.elapsed(),
                &e,
            ));
        }
    }

    // Benchmark 4: Delete
    let start = Instant::now();
    let result = benchmark_delete(&config).await;
    match result {
        Ok(bench) => {
            runner.add_result(TestResult::pass(
                &format!("delete: {}", bench.summary()),
                "benchmarks",
                start.elapsed(),
            ));
        }
        Err(e) => {
            runner.add_result(TestResult::fail(
                "delete",
                "benchmarks",
                start.elapsed(),
                &e,
            ));
        }
    }

    // Benchmark 5: Bulk Insert
    for batch_size in &config.batch_sizes {
        let start = Instant::now();
        let result = benchmark_bulk_insert(*batch_size).await;
        match result {
            Ok(bench) => {
                runner.add_result(TestResult::pass(
                    &format!("bulk_insert_{}: {}", batch_size, bench.summary()),
                    "benchmarks",
                    start.elapsed(),
                ));
            }
            Err(e) => {
                runner.add_result(TestResult::fail(
                    &format!("bulk_insert_{}", batch_size),
                    "benchmarks",
                    start.elapsed(),
                    &e,
                ));
            }
        }
    }
}

async fn benchmark_single_insert(config: &BenchmarkConfig) -> Result<BenchmarkResult, String> {
    let sdl = r#"
        type Item {
            id: ID!
            name: String!
            value: Int!
        }
    "#;

    let harness = TestHarness::new(sdl)?;
    let mut samples = Vec::new();

    // Warmup
    for i in 0..config.warmup_iterations {
        let mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "Warmup{}", value: {} }}) {{ id }} }}"#,
            i, i
        );
        harness.execute_ok(&mutation).await?;
    }

    // Measure
    for i in 0..config.iterations {
        let mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "Test{}", value: {} }}) {{ id }} }}"#,
            i, i
        );

        let start = Instant::now();
        harness.execute_ok(&mutation).await?;
        samples.push(start.elapsed());
    }

    Ok(BenchmarkResult::from_samples("insert_single", &samples))
}

async fn benchmark_read_by_id(config: &BenchmarkConfig) -> Result<BenchmarkResult, String> {
    let sdl = r#"
        type Item {
            id: ID!
            name: String!
        }
    "#;

    let harness = TestHarness::new(sdl)?;

    // Create items to read
    let mut uids = Vec::new();
    for i in 0..config.iterations {
        let mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "Item{}" }}) {{ uid }} }}"#,
            i
        );
        let response = harness.execute_ok(&mutation).await?;
        if let async_graphql::Value::Object(obj) = &response {
            if let Some(async_graphql::Value::Object(create_item)) =
                obj.get(&async_graphql::Name::new("createItem"))
            {
                if let Some(async_graphql::Value::String(uid)) =
                    create_item.get(&async_graphql::Name::new("uid"))
                {
                    uids.push(uid.clone());
                }
            }
        }
    }

    // Measure reads
    let mut samples = Vec::new();
    for uid in &uids {
        let query = format!(r#"query {{ getItem(uid: "{}") {{ uid name }} }}"#, uid);

        let start = Instant::now();
        harness.execute_ok(&query).await?;
        samples.push(start.elapsed());
    }

    Ok(BenchmarkResult::from_samples("read_by_uid", &samples))
}

async fn benchmark_update(config: &BenchmarkConfig) -> Result<BenchmarkResult, String> {
    let sdl = r#"
        type Item {
            id: ID!
            name: String!
            value: Int!
        }
    "#;

    let harness = TestHarness::new(sdl)?;

    // Create an item to update
    let response = harness
        .execute_ok(
            r#"
        mutation { createItem(input: { name: "UpdateTest", value: 0 }) { uid } }
    "#,
        )
        .await?;

    let uid = match &response {
        async_graphql::Value::Object(obj) => {
            match obj.get(&async_graphql::Name::new("createItem")) {
                Some(async_graphql::Value::Object(create_item)) => {
                    match create_item.get(&async_graphql::Name::new("uid")) {
                        Some(async_graphql::Value::String(uid)) => uid.clone(),
                        _ => return Err("No UID found".to_string()),
                    }
                }
                _ => return Err("No createItem found".to_string()),
            }
        }
        _ => return Err("Invalid response".to_string()),
    };

    // Measure updates
    let mut samples = Vec::new();
    for i in 0..config.iterations {
        let mutation = format!(
            r#"mutation {{ updateItem(uid: "{}", input: {{ value: {} }}) }}"#,
            uid, i
        );

        let start = Instant::now();
        harness.execute_ok(&mutation).await?;
        samples.push(start.elapsed());
    }

    Ok(BenchmarkResult::from_samples("update", &samples))
}

async fn benchmark_delete(config: &BenchmarkConfig) -> Result<BenchmarkResult, String> {
    let sdl = r#"
        type Item {
            id: ID!
            name: String!
        }
    "#;

    let harness = TestHarness::new(sdl)?;

    // Create items to delete
    let mut uids = Vec::new();
    for i in 0..config.iterations {
        let mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "Delete{}" }}) {{ uid }} }}"#,
            i
        );
        let response = harness.execute_ok(&mutation).await?;
        if let async_graphql::Value::Object(obj) = &response {
            if let Some(async_graphql::Value::Object(create_item)) =
                obj.get(&async_graphql::Name::new("createItem"))
            {
                if let Some(async_graphql::Value::String(uid)) =
                    create_item.get(&async_graphql::Name::new("uid"))
                {
                    uids.push(uid.clone());
                }
            }
        }
    }

    // Measure deletes
    let mut samples = Vec::new();
    for uid in &uids {
        let mutation = format!(r#"mutation {{ deleteItem(uid: "{}") }}"#, uid);

        let start = Instant::now();
        harness.execute_ok(&mutation).await?;
        samples.push(start.elapsed());
    }

    Ok(BenchmarkResult::from_samples("delete", &samples))
}

async fn benchmark_bulk_insert(batch_size: usize) -> Result<BenchmarkResult, String> {
    let sdl = r#"
        type Item {
            id: ID!
            name: String!
            value: Int!
        }
    "#;

    let harness = TestHarness::new(sdl)?;

    let start = Instant::now();

    for i in 0..batch_size {
        let mutation = format!(
            r#"mutation {{ createItem(input: {{ name: "Bulk{}", value: {} }}) {{ uid }} }}"#,
            i, i
        );
        harness.execute_ok(&mutation).await?;
    }

    let total_duration = start.elapsed();
    let throughput = batch_size as f64 / total_duration.as_secs_f64();

    Ok(BenchmarkResult {
        name: format!("bulk_insert_{}", batch_size),
        iterations: batch_size,
        total_duration,
        mean: total_duration / batch_size as u32,
        median: total_duration / batch_size as u32,
        p90: total_duration / batch_size as u32,
        p99: total_duration / batch_size as u32,
        throughput,
    })
}
