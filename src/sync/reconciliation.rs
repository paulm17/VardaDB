use crate::storage::backend::Storage;
use crate::storage::timestamp::Timestamp;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RangeFingerprint {
    pub start: Timestamp,
    pub end: Timestamp,
    pub count: u64,
    pub hash: u64, // 64-bit fingerprint (e.g. XOR sum or CRC)
}

impl RangeFingerprint {
    pub fn empty(start: Timestamp, end: Timestamp) -> Self {
        Self {
            start,
            end,
            count: 0,
            hash: 0,
        }
    }
}

/// Compute fingerprint for a time range
/// Using XOR of (TimestampHash + KeyHash + ValueHash) is order-independent and allows easy diffing?
/// Actually, RBSR usually checks "is set identical". XOR sum is good for that.
pub fn compute_fingerprint(
    storage: &Storage,
    db_name: &str,
    start: &Timestamp,
    end: &Timestamp,
) -> anyhow::Result<RangeFingerprint> {
    // Optimization: If full range (0 to MAX), return global incremental fingerprint
    let is_min_start = start.millis == 0 && start.counter == 0 && start.node_id == 0;
    let is_max_end = end.millis == u64::MAX && end.counter == u16::MAX && end.node_id == u64::MAX;

    if is_min_start && is_max_end {
        if let Some((h, c)) = storage.get_global_fingerprint(db_name) {
            // println!("DEBUG: Using Cached Fingerprint for {} (Hash: {:x}, Count: {})", db_name, h, c);
            return Ok(RangeFingerprint {
                start: *start,
                end: *end,
                count: c,
                hash: h,
            });
        }
    }

    let items = storage.get_history_range(db_name, Some(start), Some(end))?;

    let mut hash: u64 = 0;
    let mut count = 0;

    for (k, v) in items {
        let item_hash = Storage::hash_item(&k, &v);
        hash ^= item_hash;
        count += 1;
    }

    Ok(RangeFingerprint {
        start: *start,
        end: *end,
        count,
        hash,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SyncMessage {
    Gossip(RangeFingerprint),
    RequestRange { start: Timestamp, end: Timestamp },
    RangeResponse(RangeFingerprint),
    RequestData { start: Timestamp, end: Timestamp },
    DataResponse(Vec<(Vec<u8>, Vec<u8>)>),
    RequestSchema,
    SchemaResponse(String),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyncEnvelope {
    pub sender: u64,
    pub message: SyncMessage,
}
