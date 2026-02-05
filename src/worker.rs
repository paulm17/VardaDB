use std::sync::Arc;
use tokio::time::{sleep, Duration};
use crate::storage::backend::Storage;

/// Background worker that processes jobs from the "system_queue".
pub struct Worker {
    storage: Arc<Storage>,
    id: usize,
}

impl Worker {
    pub fn new(storage: Arc<Storage>, id: usize) -> Self {
        Self { storage, id }
    }

    /// Run the worker loop.
    pub async fn run(self) {
        println!("Worker #{} started for queue 'system_queue'", self.id);
        
        loop {
            // 1. Cron Trigger (Only Worker 0 does this to avoid contention/redundancy)
            // Ideally we use a distributed lock, but for single-node VardaDB, checks in `trigger_crons` are safe enough,
            // or just dedicating one worker is simpler.
            if self.id == 0 {
                if let Err(e) = self.storage.system_queue.trigger_crons() {
                    eprintln!("Worker #0: Cron Trigger Error: {}", e);
                }
            }

            // 2. Pop Job
            match self.storage.system_queue.pop() {
                Ok(Some(job)) => {
                    println!("Worker #{} processing Job {}", self.id, job.id);
                    
                    // EXECUTE JOB logic here
                    // For now, we just print payload
                    if let Ok(s) = std::str::from_utf8(&job.payload) {
                        println!("Worker #{}: Payload: {}", self.id, s);
                    } else {
                         println!("Worker #{}: Payload: <binary>", self.id);
                    }
                    
                    // Simulate work?
                    // sleep(Duration::from_millis(50)).await;
                    
                    // Ack
                    if let Err(e) = self.storage.system_queue.ack(job.id) {
                         eprintln!("Worker #{} failed to ack job {}: {}", self.id, job.id, e);
                    }
                },
                Ok(None) => {
                    // Queue empty or limit reached, sleep a bit
                    sleep(Duration::from_millis(1000)).await;
                },
                Err(e) => {
                    eprintln!("Worker #{} Pop Error: {}", self.id, e);
                    sleep(Duration::from_millis(1000)).await;
                }
            }
        }
    }
}
