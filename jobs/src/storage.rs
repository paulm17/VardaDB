use crate::types::{Job, JobId};
use std::sync::Arc;

// --- KV Store Trait ---
// Abstraction over storage backend (was Fjall Keyspace, now SQLite)

/// A key-value store trait that abstracts the underlying storage engine.
pub trait KvStore: Send + Sync {
    fn kv_insert(&self, key: &[u8], value: &[u8]) -> Result<(), String>;
    fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String>;
    fn kv_remove(&self, key: &[u8]) -> Result<(), String>;
    fn kv_prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)>;
}

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

impl<S: KvStore> JobStore<S> {
    /// Append a log entry for a specific job.
    pub fn append_log(&self, job_id: JobId, message: String) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let key = Keys::log_entry(job_id, now);
        self.keyspace
            .kv_insert(key.as_bytes(), &message.into_bytes())
    }

    /// Retrieve logs for a job.
    pub fn get_logs(&self, job_id: JobId) -> Result<Vec<(u64, String)>, String> {
        let prefix = format!("{}{}:", JOB_LOG_PREFIX, job_id);
        let mut logs = Vec::new();

        for (key, val) in self.keyspace.kv_prefix(prefix.as_bytes()) {
            let key_str = std::str::from_utf8(&key).map_err(|_| "Invalid UTF-8")?;
            let parts: Vec<&str> = key_str.split(':').collect();
            if parts.len() < 4 {
                continue;
            }

            let ts: u64 = parts[parts.len() - 1].parse().unwrap_or(0);
            let message = String::from_utf8(val).unwrap_or_default();

            logs.push((ts, message));
        }

        Ok(logs)
    }

    /// Update job metadata (partial update).
    pub fn update_job_meta(
        &self,
        job_id: JobId,
        meta: std::collections::HashMap<String, String>,
    ) -> Result<(), String> {
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

    pub fn ready_index(queue: &str, priority: i32, run_at: u64, job_id: JobId) -> String {
        let priority_sort_key = (i32::MAX - priority) as u32;
        format!(
            "{}{}:{:010}:{:020}:{}",
            JOB_READY_PREFIX, queue, priority_sort_key, run_at, job_id
        )
    }

    pub fn scheduled_index(queue: &str, run_at: u64, priority: i32, job_id: JobId) -> String {
        let priority_sort_key = (i32::MAX - priority) as u32;
        format!(
            "{}{}:{:020}:{:010}:{}",
            JOB_SCHED_PREFIX, queue, run_at, priority_sort_key, job_id
        )
    }

    pub fn active_index(job_id: JobId) -> String {
        format!("{}{}", JOB_ACTIVE_PREFIX, job_id)
    }

    pub fn active_queue_index(queue: &str, job_id: JobId) -> String {
        format!("{}{}:{}", JOB_ACTIVE_QUEUE_PREFIX, queue, job_id)
    }
}

pub struct JobStore<S: KvStore> {
    keyspace: Arc<S>,
}

impl<S: KvStore> JobStore<S> {
    pub fn new(keyspace: Arc<S>) -> Self {
        Self { keyspace }
    }

    /// Save a job to storage (Data + Index based on Location).
    pub fn put_job(&self, job: &Job) -> Result<(), String> {
        let data_key = Keys::data(job.id);

        let index_key = match &job.location {
            crate::types::JobLocation::Ready { queue } => {
                Some(Keys::ready_index(queue, job.priority, job.run_at, job.id))
            }
            crate::types::JobLocation::Scheduled { queue } => Some(Keys::scheduled_index(
                queue,
                job.run_at,
                job.priority,
                job.id,
            )),
            crate::types::JobLocation::Active { .. } => Some(Keys::active_index(job.id)),
            _ => None,
        };

        let payload = serde_json::to_vec(job).map_err(|e| e.to_string())?;

        self.keyspace.kv_insert(data_key.as_bytes(), &payload)?;
        if let Some(key) = index_key {
            self.keyspace.kv_insert(key.as_bytes(), &[])?;
        }

        Ok(())
    }

    /// Save multiple jobs.
    pub fn put_batch(&self, jobs: &[Job]) -> Result<(), String> {
        for job in jobs {
            self.put_job(job)?;
        }
        Ok(())
    }

    /// Retrieve a job by ID.
    pub fn get_job(&self, job_id: JobId) -> Result<Option<Job>, String> {
        let key = Keys::data(job_id);
        match self.keyspace.kv_get(key.as_bytes())? {
            Some(bytes) => {
                let job = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                Ok(Some(job))
            }
            None => Ok(None),
        }
    }

    /// Delete a job completely (Data + Index).
    pub fn delete_job(&self, job_id: JobId) -> Result<(), String> {
        if let Some(job) = self.get_job(job_id)? {
            let data_key = Keys::data(job_id);

            let index_key = match &job.location {
                crate::types::JobLocation::Ready { queue } => {
                    Some(Keys::ready_index(queue, job.priority, job.run_at, job.id))
                }
                crate::types::JobLocation::Scheduled { queue } => Some(Keys::scheduled_index(
                    queue,
                    job.run_at,
                    job.priority,
                    job.id,
                )),
                crate::types::JobLocation::Active { .. } => Some(Keys::active_index(job.id)),
                _ => None,
            };

            self.keyspace.kv_remove(data_key.as_bytes())?;
            if let Some(key) = index_key {
                self.keyspace.kv_remove(key.as_bytes())?;
            }

            if let crate::types::JobLocation::Active { .. } = job.location {
                let active_q_key = Keys::active_queue_index(&job.queue, job.id);
                self.keyspace.kv_remove(active_q_key.as_bytes())?;
            }
        }
        Ok(())
    }

    /// Scan for the next ready job in the Ready Index (Priority Sorted).
    pub fn scan_next_ready_job(&self, queue: &str) -> Result<Option<(JobId, String)>, String> {
        let prefix = format!("{}{}:", JOB_READY_PREFIX, queue);

        for (key, _val) in self.keyspace.kv_prefix(prefix.as_bytes()) {
            let key_str = std::str::from_utf8(&key).map_err(|_| "Invalid UTF-8 key")?;

            let parts: Vec<&str> = key_str.split(':').collect();
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
    pub fn promote_scheduled(&self, queue: &str, now: u64, limit: usize) -> Result<usize, String> {
        let prefix = format!("{}{}:", JOB_SCHED_PREFIX, queue);

        let mut promoted = 0;
        let items: Vec<(Vec<u8>, Vec<u8>)> = self.keyspace.kv_prefix(prefix.as_bytes());

        for (key, _val) in items {
            if promoted >= limit {
                break;
            }
            let key_str = std::str::from_utf8(&key).map_err(|_| "Invalid UTF-8 key")?;
            let parts: Vec<&str> = key_str.split(':').collect();
            if parts.len() < 6 {
                continue;
            }

            let run_at_str = parts[3];
            let run_at: u64 = run_at_str.parse().unwrap_or(u64::MAX);

            if run_at > now {
                break;
            }

            let job_id_str = parts[parts.len() - 1];
            let job_id: JobId = job_id_str.parse().unwrap_or(0);

            if let Some(mut job) = self.get_job(job_id)? {
                self.keyspace.kv_remove(&key)?;

                job.location = crate::types::JobLocation::Ready {
                    queue: queue.to_string(),
                };

                let ready_key = Keys::ready_index(queue, job.priority, job.run_at, job.id);
                self.keyspace.kv_insert(ready_key.as_bytes(), &[])?;

                let payload = serde_json::to_vec(&job).map_err(|e| e.to_string())?;
                let data_key = Keys::data(job.id);
                self.keyspace.kv_insert(data_key.as_bytes(), &payload)?;

                promoted += 1;
            } else {
                self.keyspace.kv_remove(&key)?;
            }
        }
        Ok(promoted)
    }

    /// Atomically move a job from Ready to Active state.
    pub fn move_to_active(
        &self,
        job_id: JobId,
        ready_key: &str,
        job: &mut Job,
        worker_id: u64,
    ) -> Result<(), String> {
        let data_key = Keys::data(job_id);
        let active_key = Keys::active_index(job_id);

        job.attempt += 1;
        job.location = crate::types::JobLocation::Active {
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            worker_id,
        };

        let payload = serde_json::to_vec(job).map_err(|e| e.to_string())?;
        let active_val = format!("{}", worker_id).into_bytes();

        self.keyspace.kv_remove(ready_key.as_bytes())?;
        self.keyspace
            .kv_insert(active_key.as_bytes(), &active_val)?;

        let active_q_key = Keys::active_queue_index(&job.queue, job.id);
        self.keyspace.kv_insert(active_q_key.as_bytes(), &[])?;

        self.keyspace.kv_insert(data_key.as_bytes(), &payload)?;

        Ok(())
    }

    pub fn remove_active_index(&self, job_id: JobId, queue: &str) -> Result<(), String> {
        let active_key = Keys::active_index(job_id);
        self.keyspace.kv_remove(active_key.as_bytes())?;

        let active_q_key = Keys::active_queue_index(queue, job_id);
        self.keyspace.kv_remove(active_q_key.as_bytes())
    }

    /// Count number of active jobs for a specific queue.
    pub fn count_active_jobs(&self, queue: &str) -> Result<usize, String> {
        let prefix = format!("{}{}:", JOB_ACTIVE_QUEUE_PREFIX, queue);
        Ok(self.keyspace.kv_prefix(prefix.as_bytes()).len())
    }
}

// --- Cron Storage ---

/// Prefix for Cron Schedules: `job:cron:{name}` -> Serialized CronSchedule
pub const JOB_CRON_PREFIX: &str = "job:cron:";

impl<S: KvStore> JobStore<S> {
    pub fn put_cron(&self, cron: &crate::types::CronSchedule) -> Result<(), String> {
        let key = format!("{}{}", JOB_CRON_PREFIX, cron.name);
        let payload = serde_json::to_vec(cron).map_err(|e| e.to_string())?;
        self.keyspace.kv_insert(key.as_bytes(), &payload)
    }

    pub fn get_cron(&self, name: &str) -> Result<Option<crate::types::CronSchedule>, String> {
        let key = format!("{}{}", JOB_CRON_PREFIX, name);
        match self.keyspace.kv_get(key.as_bytes())? {
            Some(bytes) => {
                let cron = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                Ok(Some(cron))
            }
            None => Ok(None),
        }
    }

    pub fn get_crons(&self) -> Result<Vec<crate::types::CronSchedule>, String> {
        let prefix = JOB_CRON_PREFIX;
        let mut crons = Vec::new();

        for (_key, val) in self.keyspace.kv_prefix(prefix.as_bytes()) {
            let cron: crate::types::CronSchedule =
                serde_json::from_slice(&val).map_err(|e| e.to_string())?;
            crons.push(cron);
        }
        Ok(crons)
    }
}
