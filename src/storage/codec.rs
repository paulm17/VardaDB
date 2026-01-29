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
}
