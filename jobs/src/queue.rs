//! Queue implementation for VardaJobs.
//! Handles high-level push/pop/ack operations using JobStore.

use crate::storage::{JobStore, KvStore};
use crate::types::{Job, JobId};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::str::FromStr;

/// Dyn-safe trait for pushing jobs into a queue.
/// Used by sub-crates (e.g., auth) that don't know the concrete KvStore type.
pub trait JobEnqueuer: Send + Sync {
    fn push_job(&self, job: Job) -> Result<(), String>;
}


/// Helper to get current time in ms.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

pub struct Queue<S: KvStore > {
    queue_name: String,
    store: Arc<JobStore<S>>,
    pop_lock: Mutex<()>, 
    
    /// Maximum number of active jobs allowed for this queue.
    concurrency_limit: AtomicUsize,
}

impl<S: KvStore + 'static> JobEnqueuer for Queue<S> {
    fn push_job(&self, job: Job) -> Result<(), String> {
        self.push(job)
    }
}

impl<S: KvStore> Queue<S> {
    pub fn new(queue_name: String, store: Arc<JobStore<S>>) -> Self {
        Self {
            queue_name,
            store,
            pop_lock: Mutex::new(()),
            concurrency_limit: AtomicUsize::new(100),
        }
    }
    
    pub fn set_concurrency_limit(&self, limit: usize) {
        self.concurrency_limit.store(limit, Ordering::Relaxed);
    }

    /// Push a job into the queue.
    pub fn push(&self, mut job: Job) -> Result<(), String> {
        // Enforce queue name consistency
        if job.queue != self.queue_name {
            return Err(format!("Job queue '{}' does not match queue '{}'", job.queue, self.queue_name));
        }

        // Default run_at to now if 0
        let now = now_ms();
        if job.run_at == 0 {
            job.run_at = now;
        }

        // Determine Location: Ready or Scheduled
        if job.run_at <= now {
            job.location = crate::types::JobLocation::Ready { queue: self.queue_name.clone() };
        } else {
            job.location = crate::types::JobLocation::Scheduled { queue: self.queue_name.clone() };
        }

        self.store.put_job(&job)
    }

    /// Push multiple jobs efficiently.
    pub fn push_batch(&self, mut jobs: Vec<Job>) -> Result<(), String> {
        let now = now_ms();
        
        for job in &mut jobs {
             // Enforce queue name
            if job.queue != self.queue_name {
                 return Err(format!("Job queue '{}' does not match queue '{}'", job.queue, self.queue_name));
            }
            // Default run_at
            if job.run_at == 0 {
                job.run_at = now;
            }
            // Determine Location
            if job.run_at <= now {
                job.location = crate::types::JobLocation::Ready { queue: self.queue_name.clone() };
            } else {
                job.location = crate::types::JobLocation::Scheduled { queue: self.queue_name.clone() };
            }
        }
        
        self.store.put_batch(&jobs)
    }

    /// Pop the highest priority job that is ready to run.
    pub fn pop(&self) -> Result<Option<Job>, String> {
        // Check Concurrency Limit
        let limit = self.concurrency_limit.load(Ordering::Relaxed);
        if limit > 0 {
            let active = self.store.count_active_jobs(&self.queue_name)?;
            if active >= limit {
                return Ok(None);
            }
        }
    
        let _lock = self.pop_lock.lock().map_err(|_| "Poisoned lock")?;
        
        let now = now_ms();
        
        // 1. Promote Scheduled Jobs
        // Moves jobs from Sched Index to Ready Index if run_at <= now
        // We limit to 50 to avoid stalling pop if massive backlog (though we should process them eventually).
        // Since we are in a loop, we might want to do more? sidekiq poll interval is 5-15s.
        // Here we poll on pop.
        let _promoted = self.store.promote_scheduled(&self.queue_name, now, 50)?;
        
        // 2. Scan Ready Index for highest priority job
        // Scan ignores `run_at` > now? No, Ready Index only has run_at <= now (mostly).
        // Actually scan_next_ready_job doesn't check time anymore for validity, 
        // because we assume Ready Index contains only ready jobs.
        // Except... if we promoted a job, it's run_at <= now.
        // What if someone manually inserted a future job into Ready?
        // Let's assume Ready Index implies "Eligible to Run from Scheduling Perspective".
        
        let candidate = self.store.scan_next_ready_job(&self.queue_name)?;
        
        if let Some((job_id, ready_key)) = candidate {
            let worker_id = 0; // TODO: Real worker IDs
            
            if let Some(mut job) = self.store.get_job(job_id)? {
                self.store.move_to_active(job_id, &ready_key, &mut job, worker_id)?;
                return Ok(Some(job));
            }
        }

        Ok(None)
    }

    /// Acknowledge a job completion.
    /// Removes it from Active.
    pub fn ack(&self, job_id: JobId) -> Result<(), String> {
        // Remove from Active Index.
        // Delete Job Data? 
        // Sidekiq deletes on success. FlashQ deletes on success.
        // If we want history, we move to Archive.
        // Let's delete for now to save space, matching standard Redis-backed queues.
        
        self.store.delete_job(job_id)
    }
    /// Fail a job.
    /// If retries remain, schedules it for future execution.
    /// If max retries reached, moves it to DLQ.
    pub fn fail_job(&self, mut job: Job, error: String) -> Result<(), String> {
        // 1. Remove from Active Index (since it failed)
        self.store.remove_active_index(job.id, &job.queue)?;
        
        job.last_error = Some(error);
        
        // 2. Check Retries
        if job.attempt < job.retry.max_attempts {
            // RETRY
            // Backoff: (attempt^4) + 15 + jitter
            // attempt is 1-indexed. Sidekiq uses retry_count (0-indexed).
            // Let's use attempt as base.
            let count = job.attempt as u64;
            let base_delay = count.pow(4) + 15;
             // Simple jitter: use id % 30
            let jitter = job.id % 30;
            
            let delay_seconds = base_delay + jitter;
            let delay_ms = delay_seconds * 1000;
            
            let now = now_ms();
            job.run_at = now + delay_ms;
            
            job.location = crate::types::JobLocation::Scheduled { queue: self.queue_name.clone() };
        } else {
            // DLQ
            job.location = crate::types::JobLocation::Dlq { failed_at: now_ms() };
        }
        
        // 3. Save Job (Update Data + Add Index for new location)
        self.store.put_job(&job)
    }
    
    /// Register a new durable cron schedule.
    pub fn register_cron(&self, name: String, expression: String, queue: String, payload: Vec<u8>) -> Result<(), String> {
        let schedule = cron::Schedule::from_str(&expression).map_err(|e| e.to_string())?;
        
        let now = chrono::Utc::now();
        let next = schedule.after(&now).next();
        let next_run = next.map(|dt| dt.timestamp_millis() as u64).unwrap_or(0);
        
        let cron = crate::types::CronSchedule {
            name,
            expression,
            queue,
            payload,
            next_run,
            last_run: None,
            enabled: true,
        };
        
        self.store.put_cron(&cron)
    }

    /// Check and trigger all due cron schedules.
    /// Returns number of jobs triggered.
    pub fn trigger_crons(&self) -> Result<usize, String> {
        let crons = self.store.get_crons()?;
        let now_ms = now_ms();
        let mut triggered = 0;
        
        for mut cron in crons {
            if !cron.enabled || cron.next_run == 0 || cron.next_run > now_ms {
                continue;
            }
            
            // Trigger!
            // Generate ID: timestamp + simple hash of name to avoid collisions in same ms
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            cron.name.hash(&mut hasher);
            let hash = hasher.finish();
            let job_id = now_ms + (hash % 100000); 
            
            let job = Job::new(job_id, cron.queue.clone(), cron.payload.clone());
            self.push(job)?;
            
            triggered += 1;
            
            // Calculate next run
            let schedule = cron::Schedule::from_str(&cron.expression).map_err(|e| e.to_string())?;
            let now_dt = chrono::Utc::now();
            let next = schedule.after(&now_dt).next();
            
            cron.last_run = Some(now_ms);
            cron.next_run = next.map(|dt| dt.timestamp_millis() as u64).unwrap_or(0);
            
            self.store.put_cron(&cron)?;
        }
        Ok(triggered)
    }

    /// Explicitly delay a job for a duration.
    /// Moves it to Scheduled state.
    pub fn delay(&self, job_id: JobId, duration: std::time::Duration) -> Result<(), String> {
        // 1. Remove from Active Index if it was active (or Ready)
        // Actually, we need to know where it is.
        // If it's Active, we are "snoozing" it? 
        // Or is this for a fresh job?
        // Usually called by a worker to "wait".
        // Let's assume it's currently Active or we just fetch and move it.
        
        let mut job = self.store.get_job(job_id)?.ok_or("Job not found")?;
        
        // Remove current index (Active or Ready)
        match job.location {
            crate::types::JobLocation::Active { .. } => {
                self.store.remove_active_index(job.id, &job.queue)?;
            },
            crate::types::JobLocation::Ready { .. } => {
                // Remove Ready Key - logic handled by delete_job
            },
            _ => {}
        }
        
        // Actually, `store.delete_job` reads the job from disk to find keys.
        // So we can just call delete_job(job_id).
        self.store.delete_job(job_id)?;
        
        // Update Job
        let now = now_ms();
        job.run_at = now + duration.as_millis() as u64;
        job.location = crate::types::JobLocation::Scheduled { queue: self.queue_name.clone() };
        job.attempt += 1; // Treat delay as an attempt? Or not? 
        // Trigger.dev "wait" might be a separate state?
        // For now, treat as "Scheduled" retry/step. 
        // Let's NOT increment attempt if it's explicit "wait" requested by user logic?
        // But here we are just delaying. Keep attempt same? 
        // Sidekiq "perform_in" is new job.
        // Worker calling "retry_in" is retry.
        // Let's leave attempt as is.
        
        // Save
        self.store.put_job(&job)
    }
}
