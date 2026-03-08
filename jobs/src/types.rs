//! Core types for VardaJobs (durable execution).

use serde::{Deserialize, Serialize};

/// Unique identifier for a job.
pub type JobId = u64;

/// Where a job is currently located in the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobLocation {
    /// In the priority queue, ready for execution.
    Ready { queue: String },
    /// Scheduled for future execution.
    Scheduled { queue: String },
    /// Currently being processed by a worker.
    Active { started_at: u64, worker_id: u64 },
    /// Finished processing (successfully or failed).
    Completed { at: u64, success: bool },
    /// Dead Letter Queue (failed max retries).
    Dlq { failed_at: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff_factor: f32,
    pub min_timeout_ms: u64,
    pub max_timeout_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 25,
            backoff_factor: 2.0,
            min_timeout_ms: 1000,
            max_timeout_ms: 86400 * 1000, // 24 hours
        }
    }
}

/// The main Job structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub queue: String,

    /// Execution priority (higher is better).
    pub priority: i32,

    /// Scheduled run time (unix timestamp ms).
    pub run_at: u64,

    /// Payload data (JSON-like).
    pub payload: Vec<u8>,

    /// Current attempt number (1-indexed).
    pub attempt: u32,

    /// Configuration for retries.
    pub retry: RetryConfig,

    /// Job headers / Metadata.
    pub meta: std::collections::HashMap<String, String>,

    /// Last error message if failed.
    pub last_error: Option<String>,
    pub location: JobLocation,
}

impl Job {
    pub fn new(id: JobId, queue: String, payload: Vec<u8>) -> Self {
        Self {
            id,
            queue: queue.clone(),
            priority: 0,
            run_at: 0, // 0 means "now"
            payload,
            attempt: 0,
            retry: RetryConfig::default(),
            meta: std::collections::HashMap::new(),
            last_error: None,
            location: JobLocation::Ready { queue },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSchedule {
    pub name: String,
    pub expression: String,
    pub queue: String,
    pub payload: Vec<u8>,
    pub next_run: u64,
    pub last_run: Option<u64>,
    pub enabled: bool,
}
