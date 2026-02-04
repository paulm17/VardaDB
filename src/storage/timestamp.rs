use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Serialize, Deserialize};
use byteorder::{BigEndian, ByteOrder};

/// Hybrid Logical Clock Timestamp
/// 16 bytes total:
/// - 6 bytes: Physical time (millis since epoch)
/// - 2 bytes: Logical counter
/// - 8 bytes: Node ID (randomly generated on startup)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timestamp {
    pub millis: u64, // We verify it fits in 48 bits
    pub counter: u16,
    pub node_id: u64,
}

impl Timestamp {
    /// Create a new Timestamp (initial)
    pub fn new(millis: u64, counter: u16, node_id: u64) -> Self {
        Self { millis, counter, node_id }
    }

    /// Get current wall clock time in millis
    pub fn physical_now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis() as u64
    }

    /// Serialization for Storage Keys (Big Endian for sorting)
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut buf = [0u8; 16];
        // 48-bit millis + 16-bit counter = 64-bit combined
        // But Evolu does: 6 bytes millis, 2 bytes counter.
        // We can write millis as u64 then overwrite last 2 bytes? No.
        
        // Manual pack:
        // Millis: 6 bytes
        let m = self.millis;
        buf[0] = (m >> 40) as u8;
        buf[1] = (m >> 32) as u8;
        buf[2] = (m >> 24) as u8;
        buf[3] = (m >> 16) as u8;
        buf[4] = (m >> 8) as u8;
        buf[5] = m as u8;

        // Counter: 2 bytes
        let c = self.counter;
        buf[6] = (c >> 8) as u8;
        buf[7] = c as u8;

        // NodeID: 8 bytes
        BigEndian::write_u64(&mut buf[8..16], self.node_id);
        
        buf
    }

    pub fn from_bytes(buf: &[u8; 16]) -> Self {
        let millis = 
            ((buf[0] as u64) << 40) |
            ((buf[1] as u64) << 32) |
            ((buf[2] as u64) << 24) |
            ((buf[3] as u64) << 16) |
            ((buf[4] as u64) << 8) |
            (buf[5] as u64);
        
        let counter = ((buf[6] as u16) << 8) | (buf[7] as u16);
        let node_id = BigEndian::read_u64(&buf[8..16]);

        Self { millis, counter, node_id }
    }
}

// HLC Logic
impl Timestamp {
    pub fn send(&self, physical_time: u64) -> Self {
        let next_millis = self.millis.max(physical_time);
        let next_counter = if next_millis == self.millis {
            self.counter.wrapping_add(1)
        } else {
            0
        };
        
        Self {
            millis: next_millis,
            counter: next_counter,
            node_id: self.node_id
        }
    }

    pub fn receive(&self, remote: &Timestamp, physical_time: u64) -> Self {
        let next_millis = self.millis.max(remote.millis).max(physical_time);
        
        let next_counter = if next_millis == self.millis && next_millis == remote.millis {
            self.counter.max(remote.counter).wrapping_add(1)
        } else if next_millis == self.millis {
            self.counter.wrapping_add(1)
        } else if next_millis == remote.millis {
            remote.counter.wrapping_add(1)
        } else {
            0
        };

        Self {
            millis: next_millis,
            counter: next_counter,
            node_id: self.node_id
        }
    }

    pub fn midpoint(&self, other: &Self) -> Self {
        let mid_millis = (self.millis + other.millis) / 2;
        // Midpoint acts as a partition key.
        Self::new(mid_millis, 0, 0)
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        // Lexicographical comparison
        self.millis.cmp(&other.millis)
            .then(self.counter.cmp(&other.counter))
            .then(self.node_id.cmp(&other.node_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ordering() {
        let t1 = Timestamp::new(100, 0, 1);
        let t2 = Timestamp::new(100, 1, 1);
        let t3 = Timestamp::new(101, 0, 1);
        
        assert!(t1 < t2);
        assert!(t2 < t3);
    }

    #[test]
    fn test_serialization() {
        let t = Timestamp::new(123456789, 555, 999999);
        let bytes = t.to_bytes();
        let t2 = Timestamp::from_bytes(&bytes);
        assert_eq!(t, t2);
    }

    #[test]
    fn test_send() {
        let t = Timestamp::new(100, 5, 1);
        
        // Case 1: Physical time is behind (clock drift) -> Increment counter
        let next = t.send(90); 
        assert_eq!(next.millis, 100);
        assert_eq!(next.counter, 6);

        // Case 2: Physical time is ahead -> Jump to new time, reset counter
        let next = t.send(101);
        assert_eq!(next.millis, 101);
        assert_eq!(next.counter, 0);
    }
}
