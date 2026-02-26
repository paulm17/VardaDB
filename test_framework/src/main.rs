//! VardaDB Test Framework
//! 
//! A comprehensive testing framework for VardaDB covering:
//! - CRUD operations testing
//! - Property-based testing (InsertThenSelect, LWWConvergence, etc.)
//! - Deterministic simulation with fault injection
//! - Performance benchmarks
//! - Bank test pattern (ported from Antithesis)
//! - Stress composer pattern (ported from Antithesis)
//!
//! # Usage
//! ```bash
//! # Run all tests
//! cargo run -p varda-test-framework
//!
//! # Run with specific seed (reproducible)
//! cargo run -p varda-test-framework -- --seed 12345
//!
//! # Run specific category
//! cargo run -p varda-test-framework -- --category simulation
//!
//! # Run benchmarks only
//! cargo run -p varda-test-framework -- --benchmarks
//! ```

mod harness;
mod properties;
mod simulator;
mod faults;
mod assertions;
mod bank_test;
mod stress_composer;
mod benchmarks;
mod multi_node;
mod sync_tests;
mod blob_tests;

use clap::Parser;
use colored::*;
use std::time::Duration;
use chrono::Local;

#[derive(serde::Deserialize, Debug, Default, Clone)]
pub struct TestConfig {
    #[serde(default)]
    pub blob_tests: BlobTestConfig,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct BlobTestConfig {
    pub images_dir: String,
}

impl Default for BlobTestConfig {
    fn default() -> Self {
        Self {
            images_dir: "./images".to_string(),
        }
    }
}

/// VardaDB Test Framework CLI
#[derive(Parser, Debug)]
#[command(name = "varda_test")]
#[command(about = "Comprehensive testing framework for VardaDB")]
#[command(version = "0.1.0")]
struct Cli {
    /// Random seed for deterministic testing (default: random)
    #[arg(short, long)]
    seed: Option<u64>,

    /// Test category to run (crud, properties, simulation, bank, stress, sync, zenoh, benchmarks, all)
    #[arg(short, long, default_value = "all")]
    category: String,

    /// Number of iterations for property tests
    #[arg(short, long, default_value = "100")]
    iterations: usize,

    /// Number of simulation interactions
    #[arg(short = 'n', long, default_value = "1000")]
    sim_interactions: usize,

    /// Run benchmarks only
    #[arg(long)]
    benchmarks: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Load and replay a specific bug by seed
    #[arg(long)]
    replay: Option<u64>,
}

/// Test result for a single test
#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub category: String,
    pub passed: bool,
    pub duration: Duration,
    pub message: Option<String>,
}

impl TestResult {
    pub fn pass(name: &str, category: &str, duration: Duration) -> Self {
        Self {
            name: name.to_string(),
            category: category.to_string(),
            passed: true,
            duration,
            message: None,
        }
    }

    pub fn fail(name: &str, category: &str, duration: Duration, message: &str) -> Self {
        Self {
            name: name.to_string(),
            category: category.to_string(),
            passed: false,
            duration,
            message: Some(message.to_string()),
        }
    }
}

/// Test runner that collects results
pub struct TestRunner {
    results: Vec<TestResult>,
    seed: u64,
    verbose: bool,
}

impl TestRunner {
    pub fn new(seed: u64, verbose: bool) -> Self {
        Self {
            results: Vec::new(),
            seed,
            verbose,
        }
    }

    pub fn add_result(&mut self, result: TestResult) {
        if self.verbose {
            let status = if result.passed {
                "✓".green()
            } else {
                "✗".red()
            };
            let duration_str = format!("[{:.1}ms]", result.duration.as_secs_f64() * 1000.0);
            println!("  {} {} {}", status, result.name, duration_str.dimmed());
            
            if let Some(ref msg) = result.message {
                println!("    {} {}", "→".yellow(), msg);
            }
        }
        self.results.push(result);
    }

    pub fn passed(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }

    pub fn failed(&self) -> usize {
        self.results.iter().filter(|r| !r.passed).count()
    }

    pub fn total(&self) -> usize {
        self.results.len()
    }

    pub fn total_duration(&self) -> Duration {
        self.results.iter().map(|r| r.duration).sum()
    }

    pub fn print_summary(&self) {
        let border = "═".repeat(66);
        let passed = self.passed();
        let failed = self.failed();
        let total = self.total();
        let duration = self.total_duration();

        println!();
        println!("╔{}╗", border);
        println!("║{:^66}║", "SUMMARY".bold());
        println!("╠{}╣", border);
        println!("║  Seed:    {:>54} ║", self.seed);
        println!("║  Total:   {:>54} ║", total);
        println!("║  Passed:  {} ║", format!("{:>54}", passed).green());
        
        if failed > 0 {
            println!("║  Failed:  {} ║", format!("{:>54}", failed).red());
        } else {
            println!("║  Failed:  {:>54} ║", failed);
        }
        
        println!("║  Time:    {:>52.2}s ║", duration.as_secs_f64());
        println!("╚{}╝", border);

        // Print failed tests
        if failed > 0 {
            println!();
            println!("{}", "Failed Tests:".red().bold());
            for result in &self.results {
                if !result.passed {
                    println!("  {} [{}] {}", "✗".red(), result.category, result.name);
                    if let Some(ref msg) = result.message {
                        println!("    {}", msg.dimmed());
                    }
                }
            }
        }
    }
}

fn print_banner(seed: u64) {
    let border = "═".repeat(66);
    let now = Local::now().format("%Y-%m-%d %H:%M:%S");
    
    println!();
    println!("╔{}╗", border);
    println!("║{:^66}║", "VardaDB Test Framework".bold());
    println!("╠{}╣", border);
    println!("║  Seed:    {:>54} ║", seed);
    println!("║  Started: {:>54} ║", now);
    println!("╚{}╝", border);
    println!();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize tracing
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    }

    // Generate or use provided seed
    let seed = cli.seed.unwrap_or_else(|| {
        use rand::Rng;
        rand::thread_rng().gen()
    });

    print_banner(seed);

    let mut runner = TestRunner::new(seed, true); // Always verbose for now

    // Handle replay mode
    if let Some(replay_seed) = cli.replay {
        println!("{}", format!("▶ Replaying bug with seed {}...", replay_seed).cyan());
        // TODO: Load interaction plan from bugbase and replay
        println!("  {} Bug replay not yet implemented", "⚠".yellow());
        return Ok(());
    }

    // Run benchmarks only if requested
    if cli.benchmarks {
        println!("{}", "▶ Running Benchmarks...".cyan());
        benchmarks::run_benchmarks(&mut runner, seed).await;
        runner.print_summary();
        return Ok(());
    }

    // Run test categories
    let category = cli.category.to_lowercase();

    if category == "all" || category == "crud" {
        println!("{}", "▶ Running CRUD Tests...".cyan());
        harness::run_crud_tests(&mut runner, seed).await;
    }

    if category == "all" || category == "properties" {
        println!("{}", "▶ Running Property Tests...".cyan());
        properties::run_property_tests(&mut runner, seed, cli.iterations).await;
    }

    if category == "all" || category == "bank" {
        println!("{}", "▶ Running Bank Test (LWW Invariant)...".cyan());
        bank_test::run_bank_test(&mut runner, seed).await;
    }

    if category == "all" || category == "stress" {
        println!("{}", "▶ Running Stress Composer...".cyan());
        stress_composer::run_stress_test(&mut runner, seed).await;
    }

    if category == "all" || category == "simulation" {
        println!("{}", "▶ Running Deterministic Simulation...".cyan());
        simulator::run_simulation(&mut runner, seed, cli.sim_interactions).await;
    }

    if category == "all" || category == "assertions" {
        println!("{}", "▶ Running Three-Tier Assertions...".cyan());
        assertions::run_assertion_tests(&mut runner, seed).await;
    }

    if category == "all" || category == "sync" || category == "zenoh" {
        println!("{}", "▶ Running Multi-Node Sync Tests (Zenoh)...".cyan());
        sync_tests::run_sync_tests(&mut runner).await;
    }

    if category == "all" || category == "blob" {
        println!("{}", "▶ Running Blob Storage Integration Tests...".cyan());
        
        let config_str = std::fs::read_to_string("config.toml").unwrap_or_else(|_| "".to_string());
        let test_config: TestConfig = toml::from_str(&config_str).unwrap_or_default();
        
        blob_tests::run_blob_tests(&mut runner, test_config, seed).await;
    }

    runner.print_summary();

    // Exit with error code if any tests failed
    if runner.failed() > 0 {
        std::process::exit(1);
    }

    Ok(())
}
