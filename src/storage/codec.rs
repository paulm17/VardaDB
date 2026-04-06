use byteorder::{BigEndian, ByteOrder, WriteBytesExt};

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

    pub fn decode_data_uid(key: &[u8]) -> Option<u64> {
        if key.len() < 9 || key[0] != 0x01 {
            return None;
        }
        Some(BigEndian::read_u64(&key[1..9]))
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

    pub fn encode_order_index_key(
        type_name: &str,
        field: &str,
        descending: bool,
        encoded_value: &[u8],
        uid: u64,
    ) -> Vec<u8> {
        // [0x09][Type][0x00][Field][0x00][Dir][Value][0x00][UID]
        let mut buf =
            Vec::with_capacity(1 + type_name.len() + field.len() + encoded_value.len() + 11);
        buf.push(0x09);
        buf.extend_from_slice(type_name.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(field.as_bytes());
        buf.push(0x00);
        buf.push(if descending { 1 } else { 0 });
        buf.extend_from_slice(encoded_value);
        buf.push(0x00);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }

    pub fn encode_order_index_prefix(type_name: &str, field: &str, descending: bool) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + type_name.len() + field.len() + 4);
        buf.push(0x09);
        buf.extend_from_slice(type_name.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(field.as_bytes());
        buf.push(0x00);
        buf.push(if descending { 1 } else { 0 });
        buf
    }

    pub fn decode_order_index_uid(key: &[u8]) -> Option<u64> {
        if key.len() >= 8 {
            Some(BigEndian::read_u64(&key[key.len() - 8..]))
        } else {
            None
        }
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

    pub fn decode_quarantine_value(
        val: &[u8],
    ) -> anyhow::Result<(crate::storage::timestamp::Timestamp, Vec<u8>)> {
        if val.len() < 16 {
            return Err(anyhow::anyhow!("Quarantine value too short"));
        }
        let ts_bytes: [u8; 16] = val[0..16].try_into().unwrap();
        let timestamp = crate::storage::timestamp::Timestamp::from_bytes(&ts_bytes);
        let data = val[16..].to_vec();
        Ok((timestamp, data))
    }

    pub fn decode_history_key(
        key: &[u8],
    ) -> anyhow::Result<(crate::storage::timestamp::Timestamp, u64, String)> {
        if key.len() < 24 {
            // 16 (TS) + 8 (UID) + Min 0 (Pred)
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

    // --- Edge Index (Inverse Links) ---
    // Prefix: 0x07
    // Key: [0x07][TargetUID:8][Field][0x00][SourceUID:8]
    // O(1) write per edge, prefix scan to read all edges for a field

    pub fn encode_edge_key(target_uid: u64, field: &str, source_uid: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + field.len() + 1 + 8);
        buf.push(0x07);
        buf.write_u64::<BigEndian>(target_uid).unwrap();
        buf.extend_from_slice(field.as_bytes());
        buf.push(0x00);
        buf.write_u64::<BigEndian>(source_uid).unwrap();
        buf
    }

    pub fn encode_edge_prefix(target_uid: u64, field: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + field.len() + 1);
        buf.push(0x07);
        buf.write_u64::<BigEndian>(target_uid).unwrap();
        buf.extend_from_slice(field.as_bytes());
        buf.push(0x00);
        buf
    }

    /// Decode a source_uid from an edge key. The source_uid is the last 8 bytes.
    pub fn decode_edge_source_uid(key: &[u8]) -> Option<u64> {
        if key.len() >= 8 {
            use byteorder::ByteOrder;
            Some(BigEndian::read_u64(&key[key.len() - 8..]))
        } else {
            None
        }
    }

    // --- Reverse Edge Index (Delete Cleanup) ---
    // Prefix: 0x08
    // Key: [0x08][SourceUID:8][TargetUID:8][Field][0x00]
    pub fn encode_reverse_edge_key(source_uid: u64, target_uid: u64, field: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + 8 + field.len() + 1);
        buf.push(0x08);
        buf.write_u64::<BigEndian>(source_uid).unwrap();
        buf.write_u64::<BigEndian>(target_uid).unwrap();
        buf.extend_from_slice(field.as_bytes());
        buf.push(0x00);
        buf
    }

    pub fn encode_reverse_edge_prefix(source_uid: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8);
        buf.push(0x08);
        buf.write_u64::<BigEndian>(source_uid).unwrap();
        buf
    }

    pub fn decode_reverse_edge_target_and_field(key: &[u8]) -> Option<(u64, String)> {
        if key.len() < 1 + 8 + 8 + 1 || key[0] != 0x08 {
            return None;
        }
        let target_uid = BigEndian::read_u64(&key[9..17]);
        let field_bytes = &key[17..key.len() - 1];
        let field = std::str::from_utf8(field_bytes).ok()?.to_string();
        Some((target_uid, field))
    }

    // --- Trigram Index (Substring Search) ---
    // Prefix: 0x0B
    // Key: [0x0B][Predicate][0x00][Trigram][UID]

    pub fn encode_trigram_index_key(predicate: &str, trigram: &str, uid: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + predicate.len() + 1 + trigram.len() + 8);
        buf.push(0x0B);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(trigram.as_bytes());
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }

    pub fn encode_trigram_prefix(predicate: &str, trigram: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + predicate.len() + 1 + trigram.len());
        buf.push(0x0B);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(trigram.as_bytes());
        buf
    }

    pub fn decode_trigram_index_uid(key: &[u8]) -> Option<u64> {
        if key.len() < 9 || key[0] != 0x0B {
            return None;
        }
        Some(BigEndian::read_u64(&key[key.len() - 8..]))
    }

    // --- Geohash Index (Geo Spatial) ---
    // Prefix: 0x0A
    // Key: [0x0A][Predicate][0x00][Geohash][UID]
    pub fn encode_geohash_index_key(predicate: &str, geohash: &str, uid: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + predicate.len() + 1 + geohash.len() + 8);
        buf.push(0x0A);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(geohash.as_bytes());
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }

    pub fn encode_geohash_prefix(predicate: &str, geohash_prefix: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + predicate.len() + 1 + geohash_prefix.len());
        buf.push(0x0A);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(geohash_prefix.as_bytes());
        buf
    }

    pub fn decode_geohash_index_uid(key: &[u8]) -> Option<u64> {
        if key.len() < 9 || key[0] != 0x0A {
            return None;
        }
        Some(BigEndian::read_u64(&key[key.len() - 8..]))
    }
}
