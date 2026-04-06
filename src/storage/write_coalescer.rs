use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info};

use redb::ReadableTable;

use crate::storage::redb_backend::RedbBackend;

macro_rules! dbg_info {
    ($($arg:tt)*) => {
        if crate::debug_logging() {
            info!($($arg)*)
        }
    };
}

/// A write operation that can be coalesced.
#[derive(Debug)]
pub enum CoalescedWrite {
    /// Insert a key-value pair.
    Insert {
        table_name: String,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    /// LWW upsert (key, value, timestamp).
    UpsertLww {
        table_name: String,
        key: Vec<u8>,
        value: Vec<u8>,
        ts: Vec<u8>,
    },
    /// Remove a key.
    Remove { table_name: String, key: Vec<u8> },
}

/// Configuration for the write coalescer.
pub struct CoalescerConfig {
    /// Maximum time to accumulate writes before flushing (default: 2ms).
    pub flush_interval: Duration,
    /// Maximum number of writes to buffer before forcing a flush (default: 1000).
    pub max_buffer_size: usize,
}

impl Default for CoalescerConfig {
    fn default() -> Self {
        Self {
            flush_interval: Duration::from_millis(2),
            max_buffer_size: 1000,
        }
    }
}

/// A write coalescer that accumulates writes over a short window
/// and flushes them in a single redb write transaction.
///
/// This is critical for high-frequency fingerprint updates and small writes
/// that would otherwise each open their own write transaction.
///
/// Design based on the existing vector worker pattern: bounded SyncSender +
/// background thread.
pub struct WriteCoalescer {
    tx: std::sync::mpsc::SyncSender<CoalescedWrite>,
    /// Handle to the background thread (kept for join on shutdown).
    _handle: Option<std::thread::JoinHandle<()>>,
    /// Flag to signal shutdown.
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

impl WriteCoalescer {
    /// Create a new write coalescer that flushes to the given backend.
    pub fn new(backend: Arc<RedbBackend>, config: CoalescerConfig) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<CoalescedWrite>(config.max_buffer_size * 2);
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();

        let handle = std::thread::Builder::new()
            .name("write-coalescer".into())
            .spawn(move || {
                dbg_info!("WriteCoalescer: background thread started");
                Self::run_loop(rx, backend, config, shutdown_clone);
                dbg_info!("WriteCoalescer: background thread stopped");
            })
            .expect("Failed to spawn write coalescer thread");

        Self {
            tx,
            _handle: Some(handle),
            shutdown,
        }
    }

    /// Submit a write for coalescing. This is non-blocking if the buffer
    /// is not full, and will block briefly if the buffer is at capacity.
    pub fn submit(&self, write: CoalescedWrite) -> anyhow::Result<()> {
        self.tx
            .send(write)
            .map_err(|e| anyhow::anyhow!("WriteCoalescer channel closed: {}", e))
    }

    /// Signal the coalescer to flush and shut down.
    pub fn shutdown(&self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn run_loop(
        rx: std::sync::mpsc::Receiver<CoalescedWrite>,
        backend: Arc<RedbBackend>,
        config: CoalescerConfig,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let mut buffer: Vec<CoalescedWrite> = Vec::with_capacity(config.max_buffer_size);
        let mut last_flush = Instant::now();

        loop {
            // Try to receive with a timeout equal to the flush interval
            match rx.recv_timeout(config.flush_interval) {
                Ok(write) => {
                    buffer.push(write);
                    // Drain any additional pending writes without blocking
                    while buffer.len() < config.max_buffer_size {
                        match rx.try_recv() {
                            Ok(w) => buffer.push(w),
                            Err(_) => break,
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // No writes received — check if we need to flush or shutdown
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Channel closed — flush remaining and exit
                    if !buffer.is_empty() {
                        Self::flush_buffer(&backend, &mut buffer);
                    }
                    return;
                }
            }

            // Flush if buffer is at capacity or interval has elapsed
            let should_flush = !buffer.is_empty()
                && (buffer.len() >= config.max_buffer_size
                    || last_flush.elapsed() >= config.flush_interval);

            if should_flush {
                Self::flush_buffer(&backend, &mut buffer);
                last_flush = Instant::now();
            }

            // Check shutdown flag
            if shutdown.load(std::sync::atomic::Ordering::Acquire) {
                // Drain remaining writes
                while let Ok(w) = rx.try_recv() {
                    buffer.push(w);
                }
                if !buffer.is_empty() {
                    Self::flush_buffer(&backend, &mut buffer);
                }
                return;
            }
        }
    }

    fn flush_buffer(backend: &RedbBackend, buffer: &mut Vec<CoalescedWrite>) {
        let count = buffer.len();
        let start = Instant::now();

        let result = backend.write_batch(|txn| {
            for write in buffer.drain(..) {
                match write {
                    CoalescedWrite::Insert {
                        table_name,
                        key,
                        value,
                    } => {
                        let leaked: &'static str = Box::leak(table_name.into_boxed_str());
                        let table_def = redb::TableDefinition::<&[u8], &[u8]>::new(leaked);
                        let mut table = txn.open_table(table_def)?;
                        table.insert(key.as_slice(), value.as_slice())?;
                    }
                    CoalescedWrite::UpsertLww {
                        table_name,
                        key,
                        value,
                        ts,
                    } => {
                        let leaked: &'static str = Box::leak(table_name.into_boxed_str());
                        let table_def = redb::TableDefinition::<&[u8], &[u8]>::new(leaked);
                        let mut table = txn.open_table(table_def)?;

                        let should_write = match table.get(key.as_slice())? {
                            Some(existing) => {
                                let existing_bytes: &[u8] = existing.value();
                                if existing_bytes.len() >= 16 {
                                    ts.as_slice() > &existing_bytes[..16]
                                } else {
                                    true
                                }
                            }
                            None => true,
                        };

                        if should_write {
                            let mut combined = Vec::with_capacity(16 + value.len());
                            combined.extend_from_slice(&ts);
                            combined.extend_from_slice(&value);
                            table.insert(key.as_slice(), combined.as_slice())?;
                        }
                    }
                    CoalescedWrite::Remove { table_name, key } => {
                        let leaked: &'static str = Box::leak(table_name.into_boxed_str());
                        let table_def = redb::TableDefinition::<&[u8], &[u8]>::new(leaked);
                        let mut table = txn.open_table(table_def)?;
                        table.remove(key.as_slice())?;
                    }
                }
            }
            Ok(())
        });

        let elapsed = start.elapsed();
        match result {
            Ok(_) => {
                if elapsed.as_millis() > 10 && crate::debug_logging() {
                    eprintln!("[COALESCER] flushed {} writes in {:?}", count, elapsed);
                }
            }
            Err(e) => {
                error!(
                    write_count = count,
                    error = %e,
                    "WriteCoalescer: flush failed"
                );
            }
        }
    }
}

impl Drop for WriteCoalescer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_coalescer_basic() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("coalescer_test.redb");
        let backend = Arc::new(RedbBackend::new(&db_path).unwrap());

        // Create the table first
        backend.create_table("test").unwrap();

        let config = CoalescerConfig {
            flush_interval: Duration::from_millis(5),
            max_buffer_size: 100,
        };

        let coalescer = WriteCoalescer::new(backend.clone(), config);

        // Submit some writes
        for i in 0..10u32 {
            coalescer
                .submit(CoalescedWrite::Insert {
                    table_name: "test".to_string(),
                    key: format!("key_{}", i).into_bytes(),
                    value: format!("value_{}", i).into_bytes(),
                })
                .unwrap();
        }

        // Wait for flush
        std::thread::sleep(Duration::from_millis(50));

        // Verify writes were applied
        let table =
            crate::storage::redb_backend::RedbTable::new("test".to_string(), backend.clone());

        for i in 0..10u32 {
            let key = format!("key_{}", i);
            let val = table.get(key.as_bytes()).unwrap();
            assert_eq!(val, Some(format!("value_{}", i).into_bytes()));
        }

        coalescer.shutdown();
    }

    #[test]
    fn test_coalescer_remove() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("coalescer_remove_test.redb");
        let backend = Arc::new(RedbBackend::new(&db_path).unwrap());

        backend.create_table("test").unwrap();

        let table =
            crate::storage::redb_backend::RedbTable::new("test".to_string(), backend.clone());

        // Insert directly
        table.insert(b"k1", b"v1").unwrap();

        let config = CoalescerConfig {
            flush_interval: Duration::from_millis(5),
            max_buffer_size: 100,
        };

        let coalescer = WriteCoalescer::new(backend.clone(), config);

        coalescer
            .submit(CoalescedWrite::Remove {
                table_name: "test".to_string(),
                key: b"k1".to_vec(),
            })
            .unwrap();

        std::thread::sleep(Duration::from_millis(50));

        let val = table.get(b"k1").unwrap();
        assert_eq!(val, None);

        coalescer.shutdown();
    }
}
