//! Fjall storage layer for VardaJobs.

use crate::types::{Job, JobId};
use fjall::Keyspace;
use std::sync::Arc;

// --- Key Prefixes ---

// --- Key Prefixes ---

/// Prefix for Job Logs: `job:log:{job_id}:{timestamp}` -> Log Message
pub const JOB_LOG_PREFIX: &str = "job:log:";

/// Prefix for Primary Job Data: `job:data:{job_id}` -> Serialized Job
pub const JOB_DATA_PREFIX: &str = "job:data:";

impl Keys {
    pub fn log_entry(job_id: JobId, timestamp: u64) -> String {
        format!("{}{}:{:020}", JOB_LOG_PREFIX, job_id, timestamp)
    }
}

impl JobStore {
    /// Append a log entry for a specific job.
    pub fn append_log(&self, job_id: JobId, message: String) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let key = Keys::log_entry(job_id, now);
        self.keyspace.insert(key, message.into_bytes()).map_err(|e| e.to_string())
    }

    /// Retrieve logs for a job.
    pub fn get_logs(&self, job_id: JobId) -> Result<Vec<(u64, String)>, String> {
        let prefix = format!("{}{}:", JOB_LOG_PREFIX, job_id);
        let mut logs = Vec::new();
        
        for item in self.keyspace.prefix(&prefix) {
            let (key, val) = match item.into_inner() {
                Ok((k, v)) => (k, v),
                Err(_) => continue,
            };
            
            let key_str = std::str::from_utf8(&key).map_err(|_| "Invalid UTF-8")?;
            // Key: job:log:{id}:{timestamp}
            let parts: Vec<&str> = key_str.split(':').collect();
            if parts.len() < 4 { continue; }
            
            let ts: u64 = parts[parts.len()-1].parse().unwrap_or(0);
            let message = String::from_utf8(val.to_vec()).unwrap_or_default();
            
            logs.push((ts, message));
        }
        
        Ok(logs)
    }

    /// Update job metadata (partial update).
    pub fn update_job_meta(&self, job_id: JobId, meta: std::collections::HashMap<String, String>) -> Result<(), String> {
        if let Some(mut job) = self.get_job(job_id)? {
            job.meta.extend(meta);
            self.put_job(&job)?;
        }
        Ok(())
    }
}


/// Prefix for Ready Index: `job:ready:{queue}:{prio_key}:{run_at}:{job_id}`
pub const JOB_READY_PREFIX: &str = "job:ready:";

/// Prefix for Scheduled Index: `job:sched:{queue}:{run_at}:{prio_key}:{job_id}`
pub const JOB_SCHED_PREFIX: &str = "job:sched:";

/// Prefix for Active Index: `job:active:{job_id}` -> Worker Metadata
pub const JOB_ACTIVE_PREFIX: &str = "job:active:";

/// Prefix for Active Queue Index: `job:active_q:{queue}:{job_id}` -> Empty
pub const JOB_ACTIVE_QUEUE_PREFIX: &str = "job:active_q:";

/// Key encoding helper.
pub struct Keys;

impl Keys {
    pub fn data(job_id: JobId) -> String {
        format!("{}{}", JOB_DATA_PREFIX, job_id)
    }

    /// Ready Index: Priority First.
    /// `job:ready:{queue}:{priority_padded}:{run_at_padded}:{job_id}`
    pub fn ready_index(queue: &str, priority: i32, run_at: u64, job_id: JobId) -> String {
        let priority_sort_key = (i32::MAX - priority) as u32; 
        format!("{}{}:{:010}:{:020}:{}", 
            JOB_READY_PREFIX, 
            queue, 
            priority_sort_key,
            run_at, 
            job_id
        )
    }

    /// Scheduled Index: Time First.
    /// `job:sched:{queue}:{run_at_padded}:{priority_padded}:{job_id}`
    pub fn scheduled_index(queue: &str, run_at: u64, priority: i32, job_id: JobId) -> String {
        let priority_sort_key = (i32::MAX - priority) as u32; 
        format!("{}{}:{:020}:{:010}:{}", 
            JOB_SCHED_PREFIX, 
            queue, 
            run_at,
            priority_sort_key, 
            job_id
        )
    }

    pub fn active_index(job_id: JobId) -> String {
        format!("{}{}", JOB_ACTIVE_PREFIX, job_id)
    }

    pub fn active_queue_index(queue: &str, job_id: JobId) -> String {
        format!("{}{}:{}", JOB_ACTIVE_QUEUE_PREFIX, queue, job_id)
    }
}

pub struct JobStore {
    keyspace: Arc<Keyspace>,
}

impl JobStore {
    pub fn new(keyspace: Arc<Keyspace>) -> Self {
        Self { keyspace }
    }

    /// Save a job to storage (Data + Index based on Location).
    pub fn put_job(&self, job: &Job) -> Result<(), String> {
        let data_key = Keys::data(job.id);
        
        // Determine Index Key based on Location
        let index_key = match &job.location {
            crate::types::JobLocation::Ready { queue } => {
                Some(Keys::ready_index(queue, job.priority, job.run_at, job.id))
            }
            crate::types::JobLocation::Scheduled { queue } => {
                Some(Keys::scheduled_index(queue, job.run_at, job.priority, job.id))
            }
            crate::types::JobLocation::Active { .. } => {
                Some(Keys::active_index(job.id))
            }
            _ => None, // Completed/Dlq might not have an index or handle differently
        };

        let payload = serde_json::to_vec(job).map_err(|e| e.to_string())?;

        // Sequential writes
        self.keyspace.insert(data_key, payload).map_err(|e| e.to_string())?;
        if let Some(key) = index_key {
            // Active index stores worker ID, others store empty?
            // For now put empty for all, `move_to_active` handles specific value.
            // If updating Active job via put_job, we overwrite worker ID with empty?
            // Ideally put_job is for Enqueueing. 
            // `move_to_active` is for transition.
            // Let's assume empty is fine or we check.
            self.keyspace.insert(key, &[]).map_err(|e| e.to_string())?;
        }
        
        Ok(())
    }

    /// Save multiple jobs.
    /// Note: Currently sequential (non-atomic) until fjall Batch API is clarified.
    pub fn put_batch(&self, jobs: &[Job]) -> Result<(), String> {
        for job in jobs {
            self.put_job(job)?;
        }
        Ok(())
    }

    /// Retrieve a job by ID.
    pub fn get_job(&self, job_id: JobId) -> Result<Option<Job>, String> {
        let key = Keys::data(job_id);
        match self.keyspace.get(key).map_err(|e| e.to_string())? {
            Some(bytes) => {
                let job = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                Ok(Some(job))
            }
            None => Ok(None),
        }
    }

    /// Delete a job completely (Data + Index). 
    pub fn delete_job(&self, job_id: JobId) -> Result<(), String> {
        // Read job to find its index keys
        if let Some(job) = self.get_job(job_id)? {
            let data_key = Keys::data(job_id);
            
            // Determine Index Key to delete
            let index_key = match &job.location {
                crate::types::JobLocation::Ready { queue } => {
                    Some(Keys::ready_index(queue, job.priority, job.run_at, job.id))
                }
                crate::types::JobLocation::Scheduled { queue } => {
                    Some(Keys::scheduled_index(queue, job.run_at, job.priority, job.id))
                }
                crate::types::JobLocation::Active { .. } => {
                    Some(Keys::active_index(job.id))
                }
                _ => None,
            };
            
            // Sequential deletes
            self.keyspace.remove(data_key).map_err(|e| e.to_string())?;
            if let Some(key) = index_key {
                self.keyspace.remove(key).map_err(|e| e.to_string())?;
            }
            
            // Special cleanup for Active jobs which have TWO indices
            if let crate::types::JobLocation::Active { .. } = job.location {
                let active_q_key = Keys::active_queue_index(&job.queue, job.id);
                self.keyspace.remove(active_q_key).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Scan for the next ready job in the Ready Index (Priority Sorted).
    /// Returns: Some((JobId, ReadyIndexKey))
    pub fn scan_next_ready_job(&self, queue: &str) -> Result<Option<(JobId, String)>, String> {
        let prefix = format!("{}{}:", JOB_READY_PREFIX, queue);
        
        // Scan lexicographically on Ready Index.
        // Key: job:ready:{queue}:{prio_key}:{run_at}:{id}
        // Smallest key = Highest Priority (due to prio_key inversion).
        // RunAt is secondary, but we assume all in Ready are valid to run?
        // Wait, what if we have future jobs in Ready?
        // We shouldn't. `push` should route future jobs to `Scheduled`.
        // `promote` moves them when ready.
        // So we just take the first one.
        
        let iter = self.keyspace.prefix(&prefix);
        
        for item in iter {
             let (key, _val) = match item.into_inner() {
                Ok((k, v)) => (k, v),
                Err(_) => continue,
            };
            let key_str = std::str::from_utf8(&key).map_err(|_| "Invalid UTF-8 key")?;
            
            let parts: Vec<&str> = key_str.split(':').collect();
            // Expected: ["job", "ready", "queue", "prio_key", "run_at", "id"]
            if parts.len() < 6 {
                continue; 
            }
            
            let job_id_str = parts[parts.len() - 1];
            let job_id: JobId = job_id_str.parse().unwrap_or(0);
            
            return Ok(Some((job_id, key_str.to_string())));
        }
        Ok(None)
    }

    /// Promote scheduled jobs that are now ready.
    /// Moves from Scheduled Index (Time Ordered) to Ready Index (Priority Ordered).
    pub fn promote_scheduled(&self, queue: &str, now: u64, limit: usize) -> Result<usize, String> {
        let prefix = format!("{}{}:", JOB_SCHED_PREFIX, queue);
        // Key: job:sched:{queue}:{run_at}:{prio}:{id}
        // Sorted by run_at.
        
        let mut promoted = 0;
        let iter = self.keyspace.prefix(&prefix);
        
        for item in iter {
            if promoted >= limit {
                break;
            }
             let (key, _val) = match item.into_inner() {
                Ok((k, v)) => (k, v),
                Err(_) => continue,
            };
            let key_str = std::str::from_utf8(&key).map_err(|_| "Invalid UTF-8 key")?;
             let parts: Vec<&str> = key_str.split(':').collect();
            // Expected: ["job", "sched", "queue", "run_at", "prio", "id"]
            if parts.len() < 6 {
                continue; 
            }
            
            let run_at_str = parts[3];
            let run_at: u64 = run_at_str.parse().unwrap_or(u64::MAX);
            
            // If run_at > now, we stop scanning (since sorted by time).
            if run_at > now {
                break;
            }

            let job_id_str = parts[parts.len() - 1];
            let job_id: JobId = job_id_str.parse().unwrap_or(0);
            
            // Move: Delete Sched Key, Insert Ready Key, Update Job Location
            // We need to read job to get priority (or parse from key, but updating Job is good practice).
            
            if let Some(mut job) = self.get_job(job_id)? {
                // Remove Sched Key
                self.keyspace.remove(key.clone()).map_err(|e| e.to_string())?;
                
                // Update Location
                job.location = crate::types::JobLocation::Ready { queue: queue.to_string() };
                
                // Insert Ready Key
                let ready_key = Keys::ready_index(queue, job.priority, job.run_at, job.id);
                self.keyspace.insert(&ready_key, &[]).map_err(|e| e.to_string())?;
                
                // Update Job Data
                let payload = serde_json::to_vec(&job).map_err(|e| e.to_string())?;
                let data_key = Keys::data(job.id);
                self.keyspace.insert(data_key, payload).map_err(|e| e.to_string())?;
                
                promoted += 1;
            } else {
                // Orphan key? Remove it.
                self.keyspace.remove(key.clone()).map_err(|e| e.to_string())?;
            }
        }
        Ok(promoted)
    }

    /// Atomically move a job from Ready to Active state.
    pub fn move_to_active(&self, job_id: JobId, ready_key: &str, job: &mut Job, worker_id: u64) -> Result<(), String> {
        let data_key = Keys::data(job_id);
        let active_key = Keys::active_index(job_id);
        
        // Update Job Data in memory
        job.attempt += 1;
        job.location = crate::types::JobLocation::Active { 
            started_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64, 
            worker_id 
        };
        
        let payload = serde_json::to_vec(job).map_err(|e| e.to_string())?;
        let active_val = format!("{}", worker_id).into_bytes(); 
        
        // Sequential writes
        // 1. Remove Ready Index
        self.keyspace.remove(ready_key).map_err(|e| e.to_string())?;
        
        // 2. Add Active Index
        self.keyspace.insert(active_key, active_val).map_err(|e| e.to_string())?;
        
        // 2b. Add Active Queue Index (for concurrency tracking)
        let active_q_key = Keys::active_queue_index(&job.queue, job.id);
        self.keyspace.insert(active_q_key, &[]).map_err(|e| e.to_string())?;
        
        // 3. Update Job Data
        self.keyspace.insert(data_key, payload).map_err(|e| e.to_string())?;
        
        Ok(())
    }
    pub fn remove_active_index(&self, job_id: JobId, queue: &str) -> Result<(), String> {
        let active_key = Keys::active_index(job_id);
        self.keyspace.remove(active_key).map_err(|e| e.to_string())?;
        
        let active_q_key = Keys::active_queue_index(queue, job_id);
        self.keyspace.remove(active_q_key).map_err(|e| e.to_string())
    }

    /// Count number of active jobs for a specific queue.
    pub fn count_active_jobs(&self, queue: &str) -> Result<usize, String> {
        let prefix = format!("{}{}:", JOB_ACTIVE_QUEUE_PREFIX, queue);
        let mut count = 0;
        for _ in self.keyspace.prefix(&prefix) {
            count += 1;
        }
        Ok(count)
    }
}

// --- Cron Storage ---

/// Prefix for Cron Schedules: `job:cron:{name}` -> Serialized CronSchedule
pub const JOB_CRON_PREFIX: &str = "job:cron:";

impl JobStore {
    pub fn put_cron(&self, cron: &crate::types::CronSchedule) -> Result<(), String> {
        let key = format!("{}{}", JOB_CRON_PREFIX, cron.name);
        let payload = serde_json::to_vec(cron).map_err(|e| e.to_string())?;
        self.keyspace.insert(key, payload).map_err(|e| e.to_string())
    }
    
    pub fn get_cron(&self, name: &str) -> Result<Option<crate::types::CronSchedule>, String> {
            let key = format!("{}{}", JOB_CRON_PREFIX, name);
            match self.keyspace.get(key).map_err(|e| e.to_string())? {
                Some(bytes) => {
                    let cron = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                    Ok(Some(cron))
                },
                None => Ok(None)
            }
    }
    
    pub fn get_crons(&self) -> Result<Vec<crate::types::CronSchedule>, String> {
        let prefix = JOB_CRON_PREFIX;
        let mut crons = Vec::new();
        
        for item in self.keyspace.prefix(prefix) {
                let (_key, val) = match item.into_inner() {
                Ok((k, v)) => (k, v),
                Err(_) => continue,
            };
            let cron: crate::types::CronSchedule = serde_json::from_slice(&val).map_err(|e| e.to_string())?;
            crons.push(cron);
        }
        Ok(crons)
    }
}

