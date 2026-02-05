use byteorder::{BigEndian, WriteBytesExt};

// Key Structure:
// [Prefix: 1 byte] [UID: 8 bytes] [Predicate: N bytes]
// Prefix:
// 0x01: Data
// 0x02: Index

pub struct Codec;

impl Codec {
    pub fn encode_data_key(uid: u64, predicate: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + predicate.len());
        buf.push(0x01); // Data Prefix
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf.extend_from_slice(predicate.as_bytes());
        buf
    }

    pub fn encode_data_prefix(uid: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(9);
        buf.push(0x01);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }

    pub fn encode_index_key(predicate: &str, value: &str, uid: u64) -> Vec<u8> {
        // Index: [Prefix][Predicate][Value][UID]
        // Note: Real Dgraph uses more complex encoding for values.
        let mut buf = Vec::new();
        buf.push(0x02); // Index Prefix
        // Length prefixed predicate to avoid collision? 
        // For simplicity: Predicate + 0x00 + Value + 0x00 + UID
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(value.as_bytes());
        buf.push(0x00);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }
    pub fn encode_unique_index_key(predicate: &str, value: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x02); // Index Prefix
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(value.as_bytes());
        buf
    }

    pub fn encode_type_index_key(type_name: &str, uid: u64) -> Vec<u8> {
        // [0x03][Type][0x00][UID]
        let mut buf = Vec::new();
        buf.push(0x03); 
        buf.extend_from_slice(type_name.as_bytes());
        buf.push(0x00);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }
    
    pub fn encode_type_prefix(type_name: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x03);
        buf.extend_from_slice(type_name.as_bytes());
        buf.push(0x00);
        buf
    }
    pub fn encode_term_index_key(predicate: &str, term: &str, uid: u64) -> Vec<u8> {
        // [0x04][Predicate][0x00][Term][0x00][UID]
        // Term Index Prefix: 0x04
        let mut buf = Vec::new();
        buf.push(0x04);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(term.as_bytes());
        buf.push(0x00);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }
    
    pub fn encode_term_index_prefix(predicate: &str, term: &str) -> Vec<u8> {
        // [0x04][Predicate][0x00][Term][0x00]
        let mut buf = Vec::new();
        buf.push(0x04);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(term.as_bytes());
        buf.push(0x00);
        buf
    }

    // --- Evolu / Varda Extensions ---

    /// History Key: [Timestamp: 16][UID: 8][Predicate: Var]
    /// Used for Range-Based Set Reconciliation (Sync)
    pub fn encode_history_key(timestamp: &[u8; 16], uid: u64, predicate: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(16 + 8 + predicate.len());
        buf.extend_from_slice(timestamp);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf.extend_from_slice(predicate.as_bytes());
        buf
    }

    pub fn encode_quarantine_key(uid: u64, predicate: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + predicate.len());
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf.extend_from_slice(predicate.as_bytes());
        buf
    }

    pub fn decode_quarantine_key(key: &[u8]) -> anyhow::Result<(u64, String)> {
        if key.len() < 8 {
            return Err(anyhow::anyhow!("Quarantine key too short"));
        }
        use byteorder::ByteOrder;
        let uid = byteorder::BigEndian::read_u64(&key[0..8]);
        let pred_bytes = &key[8..];
        let predicate = std::str::from_utf8(pred_bytes)?.to_string();
        Ok((uid, predicate))
    }

    pub fn decode_quarantine_value(val: &[u8]) -> anyhow::Result<(crate::storage::timestamp::Timestamp, Vec<u8>)> {
        if val.len() < 16 {
            return Err(anyhow::anyhow!("Quarantine value too short"));
        }
        let ts_bytes: [u8; 16] = val[0..16].try_into().unwrap();
        let timestamp = crate::storage::timestamp::Timestamp::from_bytes(&ts_bytes);
        let data = val[16..].to_vec();
        Ok((timestamp, data))
    }

    pub fn decode_history_key(key: &[u8]) -> anyhow::Result<(crate::storage::timestamp::Timestamp, u64, String)> {
        if key.len() < 24 { // 16 (TS) + 8 (UID) + Min 0 (Pred)
            return Err(anyhow::anyhow!("History key too short"));
        }
        let ts_bytes: [u8; 16] = key[0..16].try_into().unwrap();
        let timestamp = crate::storage::timestamp::Timestamp::from_bytes(&ts_bytes);
        
        use byteorder::ByteOrder;
        let uid = byteorder::BigEndian::read_u64(&key[16..24]);
        
        let pred_bytes = &key[24..];
        let predicate = std::str::from_utf8(pred_bytes)?.to_string();
        
        Ok((timestamp, uid, predicate))
    }

    // --- BM25 Stats ---
    // Prefix: 0x05
    // Key: [0x05][Pred][0x00][StatType]
    // StatType: 0=DocCount, 1=TotalLen, 2=DF(Term)
    
    pub fn encode_stat_key(predicate: &str, stat_type: u8, term: Option<&str>) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x05);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.push(stat_type);
        if let Some(t) = term {
            buf.push(0x00);
            buf.extend_from_slice(t.as_bytes());
        }
        buf
    }

    // --- Doc Meta (Length) ---
    // Prefix: 0x06
    // Key: [0x06][Pred][0x00][UID]
    pub fn encode_doc_meta_key(predicate: &str, uid: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x06);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }
}
