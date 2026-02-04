use crate::storage::timestamp::Timestamp;
use crate::storage::backend::Storage;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RangeFingerprint {
    pub start: Timestamp,
    pub end: Timestamp,
    pub count: u64,
    pub hash: u64, // 64-bit fingerprint (e.g. XOR sum or CRC)
}

impl RangeFingerprint {
    pub fn empty(start: Timestamp, end: Timestamp) -> Self {
        Self { start, end, count: 0, hash: 0 }
    }
}

/// Compute fingerprint for a time range
/// Using XOR of (TimestampHash + KeyHash + ValueHash) is order-independent and allows easy diffing?
/// Actually, RBSR usually checks "is set identical". XOR sum is good for that.
pub fn compute_fingerprint(storage: &Storage, start: &Timestamp, end: &Timestamp) -> anyhow::Result<RangeFingerprint> {
    let items = storage.get_history_range(Some(start), Some(end))?;
    
    let mut hash: u64 = 0;
    let mut count = 0;

    for (k, v) in items {
        // k is [Ts][UID][Pred]
        // v for history can be [Val] or [] (Tombstone)
        
        // We hash the Key + Value. 
        // We use a simple hash function for now (e.g. seahash or fxhash would be better, but we'll stick to std or simple math)
        // Let's use a simple shifting XOR mix for 64-bit.
        
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        k.hash(&mut hasher);
        v.hash(&mut hasher);
        let item_hash = hasher.finish();
        
        hash ^= item_hash; // XOR allows adding/removing items order-independently
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
