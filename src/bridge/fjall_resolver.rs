use crate::engine::resolver::Resolver;
use crate::storage::backend::Storage;
use crate::storage::codec::Codec;
use async_graphql::Value;
use std::sync::Arc;
use byteorder::{BigEndian, ByteOrder};
use crate::storage::timestamp::Timestamp;

use crate::realtime::bus::{EventBus, MutationEvent, MutationType};

#[derive(Clone)]
pub struct FjallResolver {
    pub storage: Arc<Storage>,
    pub bus: EventBus,
    pub db_name: String,
}

impl FjallResolver {
    pub fn new(storage: Arc<Storage>, db_name: &str) -> Self {
        Self {
            storage,
            db_name: db_name.to_string(),
            bus: EventBus::new(),
        }
    }

    /// Create a FjallResolver with a shared EventBus.
    /// Use this to ensure all resolver instances publish to the same bus.
    pub fn with_bus(storage: Arc<Storage>, bus: EventBus) -> Self {
        Self {
            storage,
            bus,
            db_name: "default".to_string(),
        }
    }
    
    pub fn with_db(storage: Arc<Storage>, bus: EventBus, db_name: String) -> Self {
        Self {
            storage,
            bus,
            db_name,
        }
    }

    pub fn compute_fingerprint(&self) -> anyhow::Result<crate::sync::reconciliation::RangeFingerprint> {
        // Full range fingerprint
        let start = crate::storage::timestamp::Timestamp::new(0, 0, 0);
        let end = crate::storage::timestamp::Timestamp::new(u64::MAX, u16::MAX, u64::MAX);
        crate::sync::reconciliation::compute_fingerprint(&self.storage, &self.db_name, &start, &end)
    }

    pub fn compute_fingerprint_range(&self, start: &Timestamp, end: &Timestamp) -> anyhow::Result<crate::sync::reconciliation::RangeFingerprint> {
        crate::sync::reconciliation::compute_fingerprint(&self.storage, &self.db_name, start, end)
    }

    pub fn get_history_range(&self, start: &Timestamp, end: &Timestamp) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.storage.get_history_range(&self.db_name, Some(start), Some(end))
    }

    pub fn apply_batch(&self, items: Vec<(Vec<u8>, Vec<u8>)>) -> anyhow::Result<()> {
        let mut deleted_uids = std::collections::HashSet::new();
        // Pre-scan for deletions
        for (k, v) in &items {
            if v.is_empty() {
                if let Ok((_, uid, pred)) = Codec::decode_history_key(k) {
                    if pred == "_type" {
                        deleted_uids.insert(uid);
                    }
                }
            }
        }

        if crate::debug_logging() {
            println!("Sync: apply_batch called with {} items", items.len());
        }
        
        // Partition items: _type first
        let (type_items, other_items): (Vec<_>, Vec<_>) = items.into_iter().partition(|(k, _)| {
             if let Ok((_, _, pred)) = crate::storage::codec::Codec::decode_history_key(k) {
                 pred == "_type"
             } else {
                 false
             }
        });

        if crate::debug_logging() {
            println!("Sync: Partitioned batch: {} type items, {} other items", type_items.len(), other_items.len());
        }

        // Initialize Event Buffer
        // Map: UID -> (Type, MutationType, Payload, MinTimestamp)
        // We use MinTimestamp because events are usually batched from the same "transaction" or we want the earliest causal time? 
        // Actually, for LWW, if we have multiple updates, we want the LATEST timestamp.
        let mut pending_emissions: std::collections::HashMap<u64, (String, crate::realtime::bus::MutationType, std::collections::HashMap<String, serde_json::Value>, crate::storage::timestamp::Timestamp)> = std::collections::HashMap::new();

        for (k, v) in type_items.into_iter().chain(other_items.into_iter()) {
             // Decode Key: [Ts][UID][Pred]
             match Codec::decode_history_key(&k) {
                 Ok((ts, uid, pred)) => {
                     // Proceed

                 if v.is_empty() {
                     // Tombstone
                     let mut event_type_name = "Unknown".to_string();
                     let mut mutation_type = crate::realtime::bus::MutationType::Update;
                     let mut should_emit = true;

                     // 1. Index Maintenance (Type Index Deletion)
                     if pred == "_type" {
                          let data_key = crate::storage::codec::Codec::encode_data_key(uid, "_type");
                          if let Ok(Some(current_bytes)) = self.storage.get(&self.db_name, &data_key) {
                               if let Ok(serde_json::Value::String(current_type)) = serde_json::from_slice(&current_bytes) {
                                    let type_idx_key = crate::storage::codec::Codec::encode_type_index_key(&current_type, uid);
                                    let _ = self.storage.remove(&self.db_name, &type_idx_key);
                                    
                                    event_type_name = current_type;
                                    mutation_type = crate::realtime::bus::MutationType::Delete;
                               }
                          }
                     } else {
                         // If this node is being deleted, suppress individual field tombstone events
                         if deleted_uids.contains(&uid) {
                             should_emit = false;
                         }
                     }

                     self.storage.delete_with_lww(&self.db_name, uid, &pred, &ts)?;
                     
                     if should_emit {
                         // Emit Event Immediately for Delete (Atomic enough usually)
                         let payload = if mutation_type == crate::realtime::bus::MutationType::Delete {
                             None
                         } else {
                             Some(std::collections::HashMap::from([(pred, serde_json::Value::Null)]))
                         };

                         let event = crate::realtime::bus::MutationEvent {
                             type_name: event_type_name,
                             uid,
                             mutation_type,
                             source: crate::realtime::bus::MutationSource::Remote,
                             payload,
                             metadata: None,
                             timestamp: Some(ts),
                         };
                         let _ = self.bus.publish(event);
                     }

                 } else {
                     self.storage.put_with_lww(&self.db_name, uid, &pred, &v, &ts)?;
                     
                     // 1. Buffer Event Emission
                     if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&v) {
                         // 2. Index Maintenance (Type Index)
                         let mut resolved_type_name = "Unknown".to_string();
                         
                         if pred == "_type" {
                             if let serde_json::Value::String(ref type_name) = json_val {
                                 let type_idx_key = Codec::encode_type_index_key(type_name, uid);
                                 let _res = self.storage.insert(&self.db_name, &type_idx_key, &[]);
                                 if crate::debug_logging() {
                                     println!("Sync: Insert Type Index Key: {:?}, Result: {:?}", type_idx_key, _res);
                                 }
                                 resolved_type_name = type_name.clone();
                             } else {
                                  println!("Sync: ERROR: _type predicate found but value is not a String! Value: {:?}", json_val);
                             }
                         } else {
                             // Try to lookup type from storage
                             let type_key = crate::storage::codec::Codec::encode_data_key(uid, "_type");
                             if let Ok(Some(type_bytes)) = self.storage.get(&self.db_name, &type_key) {
                                 if let Ok(serde_json::Value::String(t)) = serde_json::from_slice(&type_bytes) {
                                     resolved_type_name = t;
                                 }
                             }
                         }

                         // Add to Buffer
                         let entry = pending_emissions.entry(uid).or_insert_with(|| ("Unknown".to_string(), crate::realtime::bus::MutationType::Update, std::collections::HashMap::new(), ts));
                         
                         // Update Timestamp (Take Latest)
                         if ts > entry.3 {
                             entry.3 = ts;
                         }
                         
                         // Update Type Name if resolved
                         if resolved_type_name != "Unknown" {
                             entry.0 = resolved_type_name;
                         }
                         
                         // Determine Mutation Type
                         if pred == "_type" {
                             entry.1 = crate::realtime::bus::MutationType::Create;
                         }

                         entry.2.insert(pred, json_val);
                     }
                 }
                 self.storage.update_clock(&ts);
             },
             Err(_e) => {
                 println!("Sync: Failed to decode key: {:?}", k);
             }
        }
    }
    
    // Flush Pending Events
    for (uid, (type_name, mutation_type, mut payload, timestamp)) in pending_emissions {
        // Inject ID into payload for Frontend Cache compatibility
        payload.insert("id".to_string(), serde_json::Value::String(uid.to_string()));
        
        let event = crate::realtime::bus::MutationEvent {
            type_name,
            uid,
            mutation_type,
            source: crate::realtime::bus::MutationSource::Remote,
            payload: Some(payload),
            metadata: None,
            timestamp: Some(timestamp),
        };
        let _ = self.bus.publish(event);
    }

    Ok(())
}

    pub fn try_restore_quarantine(&self, valid_predicates: &std::collections::HashSet<String>) -> anyhow::Result<usize> {
        let items = self.storage.scan_quarantine()?;
        let mut restored = 0;
        for (k, v) in items {
             if let Ok((uid, pred)) = Codec::decode_quarantine_key(&k) {
                 if valid_predicates.contains(&pred) {
                     if let Ok((ts, data)) = Codec::decode_quarantine_value(&v) {
                         // Restore to LATEST/HISTORY using LWW
                         self.storage.put_with_lww(&self.db_name, uid, &pred, &data, &ts)?;
                         // Remove from Quarantine
                         self.storage.delete_quarantine(&k)?;
                         restored += 1;
                     }
                 }
             }
        }
        Ok(restored)
    }

    fn link_inverse(&self, target_uid: u64, inverse_field: &str, is_list: bool, self_uid: u64, timestamp: &Timestamp) -> Result<(), String> {
         if is_list {
             // O(1) write: just insert a single edge key
             let edge_key = Codec::encode_edge_key(target_uid, inverse_field, self_uid);
             self.storage.insert(&self.db_name, &edge_key, &[]).map_err(|e| e.to_string())?;
         } else {
             // 1:1 or N:1 - Overwrite (unchanged, single value is fast)
             let val = Value::String(self_uid.to_string());
             let bytes = serde_json::to_vec(&val).map_err(|e| e.to_string())?;
             self.storage.put_with_lww(&self.db_name, target_uid, inverse_field, &bytes, timestamp).map_err(|e| e.to_string())?;
         }
         Ok(())
    }

    fn unlink_inverse(&self, target_uid: u64, inverse_field: &str, is_list: bool, self_uid: u64, timestamp: &Timestamp) -> Result<(), String> {
         if is_list {
             // O(1) delete: just remove the single edge key
             let edge_key = Codec::encode_edge_key(target_uid, inverse_field, self_uid);
             self.storage.delete_key(&self.db_name, &edge_key).map_err(|e| e.to_string())?;
         } else {
             // 1:1 - If the current value IS self, remove it
             let key = Codec::encode_data_key(target_uid, inverse_field);
             if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &key) {
                  if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                       let matches = match val {
                           Value::String(s) => s == self_uid.to_string(),
                           Value::Number(n) => n.as_u64() == Some(self_uid),
                           _ => false
                       };
                       if matches {
                           self.storage.delete_with_lww(&self.db_name, target_uid, inverse_field, timestamp).map_err(|e| e.to_string())?;
                       }
                  }
             }
         }
         Ok(())
    }

    // Helper for BM25 Stats
    fn increment_stat(&self, key: &[u8], delta: i64) -> Result<(), String> {
        let val_opt = self.storage.get(&self.db_name, key).map_err(|e| e.to_string())?;
        let current = if let Some(bytes) = val_opt {
            if bytes.len() >= 8 {
               byteorder::BigEndian::read_i64(&bytes)
            } else { 0 }
        } else { 0 };
        
        let new_val = current + delta;
        let mut buf = [0u8; 8];
        byteorder::BigEndian::write_i64(&mut buf, new_val);
        self.storage.insert(&self.db_name, key, &buf).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_stat(&self, key: &[u8]) -> Option<i64> {
         if let Ok(Some(bytes)) = self.storage.get(&self.db_name, key) {
             if bytes.len() >= 8 {
                 Some(byteorder::BigEndian::read_i64(&bytes))
             } else { None }
         } else { None }
    }

    fn write_term_index(&self, uid: u64, field: &str, text: &str, strategy: &str) -> Result<(), String> {
        let tokens = crate::engine::tokenizer::Tokenizer::tokenize(text, strategy);
        let doc_len = tokens.len() as i64;
        let index_field = if strategy == "term" { field.to_string() } else { format!("{}.{}", field, strategy) };

        // 1. Update Doc Count (N)
        let n_key = Codec::encode_stat_key(&index_field, 0, None);
        self.increment_stat(&n_key, 1)?;

        // 2. Update Total Length (for AvgDL)
        let len_key = Codec::encode_stat_key(&index_field, 1, None);
        self.increment_stat(&len_key, doc_len)?;
        
        // 3. Store Doc Length (Per Doc)
        let doc_meta_key = Codec::encode_doc_meta_key(&index_field, uid);
        let len_u32 = doc_len as u32;
        self.storage.insert(&self.db_name, &doc_meta_key, &len_u32.to_be_bytes()).map_err(|e| e.to_string())?;

        // 4. Count Term Frequencies
        let mut tf_map: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for term in tokens {
            *tf_map.entry(term).or_insert(0) += 1;
        }

        // 5. Update Index and DF
        for (term, tf) in tf_map {
            let key = Codec::encode_term_index_key(&index_field, &term, uid);
            // Value = TF (u32)
            self.storage.insert(&self.db_name, &key, &tf.to_be_bytes()).map_err(|e| e.to_string())?;

            // Increment DF
            let df_key = Codec::encode_stat_key(&index_field, 2, Some(&term));
            self.increment_stat(&df_key, 1)?;
        }
        Ok(())
    }

    fn remove_term_index(&self, uid: u64, field: &str, text: &str, strategy: &str) -> Result<(), String> {
        let tokens = crate::engine::tokenizer::Tokenizer::tokenize(text, strategy);
        let doc_len = tokens.len() as i64;
        let index_field = if strategy == "term" { field.to_string() } else { format!("{}.{}", field, strategy) };

        // 1. Decrement Doc Count (N)
        let n_key = Codec::encode_stat_key(&index_field, 0, None);
        self.increment_stat(&n_key, -1)?;

        // 2. Decrement Total Length
        let len_key = Codec::encode_stat_key(&index_field, 1, None);
        self.increment_stat(&len_key, -doc_len)?;
        
        // 3. Remove Doc Length
        let doc_meta_key = Codec::encode_doc_meta_key(&index_field, uid);
        self.storage.remove(&self.db_name, &doc_meta_key).map_err(|e| e.to_string())?;

        // 4. Count TF
        let mut tf_map: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for term in tokens {
            *tf_map.entry(term).or_insert(0) += 1;
        }

        for (term, _) in tf_map {
            let key = Codec::encode_term_index_key(&index_field, &term, uid);
            self.storage.remove(&self.db_name, &key).map_err(|e| e.to_string())?;

            // Decrement DF
            let df_key = Codec::encode_stat_key(&index_field, 2, Some(&term));
            self.increment_stat(&df_key, -1)?;
        }
        Ok(())
    }
    
    // Ranked Search (BM25)
    pub fn search_text_bm25(&self, query: &str, field: &str, strategy: &str, k: usize, require_all: bool) -> Vec<(u64, f64)> {
        let index_field = if strategy == "term" { field.to_string() } else { format!("{}.{}", field, strategy) };
        let tokens = crate::engine::tokenizer::Tokenizer::tokenize(query, strategy);
        if tokens.is_empty() { return vec![]; }
        
        // Deduplicate tokens for counting unique matches if requiring all
        let unique_tokens: std::collections::HashSet<String> = if require_all {
            tokens.iter().cloned().collect()
        } else {
            std::collections::HashSet::new()
        };

        // 1. Get Global Stats
        let n_key = Codec::encode_stat_key(&index_field, 0, None);
        let n: f64 = self.get_stat(&n_key).unwrap_or(0) as f64;
        
        let len_key = Codec::encode_stat_key(&index_field, 1, None);
        let total_len: f64 = self.get_stat(&len_key).unwrap_or(0) as f64;
        let avg_dl = if n > 0.0 { total_len / n } else { 0.0 };

        if n == 0.0 { return vec![]; }

        let k1 = 1.2;
        let b = 0.75;
        
        let mut scores: std::collections::HashMap<u64, f64> = std::collections::HashMap::new();

        for term in tokens {
            // Get DF
            let df_key = Codec::encode_stat_key(&index_field, 2, Some(&term));
            let df: f64 = self.get_stat(&df_key).unwrap_or(0) as f64;
            
            if df == 0.0 { continue; }
            
            // IDF (Dgraph uses this variant usually)
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            
            // Scan Index
            let prefix = Codec::encode_term_index_prefix(&index_field, &term);
            use std::ops::Bound;
            // Note: Directly accessing self.storage.main_keyspace from here requires pub visibility or getter.
            // backend.rs has `pub main_keyspace`.
// use std::ops::Bound; // Removed redundant import
            let (main_ks, _) = match self.storage.get_database(&self.db_name) {
                 Some(d) => d,
                 None => return vec![],
            };

            // Note: Directly accessing self.storage.main_keyspace from here requires pub visibility or getter.
            // backend.rs has `pub main_keyspace`.
            let iter = main_ks.range((Bound::Included(prefix.clone()), Bound::Unbounded));
            
            for guard in iter {
                if let Ok((key, val)) = guard.into_inner() {
                    if !key.starts_with(&prefix) { break; }
                    
                    // Decode UID and TF
                    if key.len() < 8 { continue; }
                    let uid = byteorder::BigEndian::read_u64(&key[key.len()-8..]);
                    let tf = if val.len() >= 4 { byteorder::BigEndian::read_u32(&val) as f64 } else { 1.0 };
                    
                    // Get document length (dl)
                    let dl_key = Codec::encode_doc_meta_key(&index_field, uid);
                    let dl = if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &dl_key) {
                        if bytes.len() >= 4 { byteorder::BigEndian::read_u32(&bytes) as f64 } else { 10.0 }
                    } else { 10.0 };
                    
                    let score = idf * (tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * (dl / avg_dl)));
                    
                    *scores.entry(uid).or_insert(0.0) += score;
                }
            }
        }
        
        let mut final_scores = scores;

        if require_all {
             let mut intersection: Option<std::collections::HashSet<u64>> = None;
             for term in &unique_tokens {
                 let prefix = Codec::encode_term_index_prefix(&index_field, term);
                 let (main_ks, _) = match self.storage.get_database(&self.db_name) {
                     Some(d) => d,
                     None => return vec![],
                 };
                 let iter = main_ks.range((std::ops::Bound::Included(prefix.clone()), std::ops::Bound::Unbounded));
                 
                 let mut term_uids = std::collections::HashSet::new();
                 for guard in iter {
                     if let Ok((key, _)) = guard.into_inner() {
                         if !key.starts_with(&prefix) { break; }
                         if key.len() < 8 { continue; }
                         let uid = byteorder::BigEndian::read_u64(&key[key.len()-8..]);
                         term_uids.insert(uid);
                     }
                 }
                 
                 if let Some(existing) = intersection {
                     intersection = Some(existing.intersection(&term_uids).cloned().collect());
                 } else {
                     intersection = Some(term_uids);
                 }
                 
                 if intersection.as_ref().map(|s| s.is_empty()).unwrap_or(false) {
                     return vec![];
                 }
             }
             
             if let Some(valid_uids) = intersection {
                 final_scores.retain(|uid, _| valid_uids.contains(uid));
             }
        }
        
        // Sort and Take K
        let mut result: Vec<(u64, f64)> = final_scores.into_iter().collect();
        result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        result.truncate(k);
        result
    }

    // Hybrid Search (RRF)
    pub fn search_hybrid(&self, text_query: &str, field: &str, vector: &[f64], k: usize, require_all: bool) -> Vec<(u64, f64)> {
        let k_limit = k * 2; // Fetch more candidates
        
        // 1. Vector Search
        let vec_res = match self.storage.search_vectors(vector, k_limit) {
            Ok(res) => res, // [(uid, dist)]
            Err(_) => vec![]
        };
        
        // 2. Text Search (Assuming fulltext strategy)
        let text_res = self.search_text_bm25(text_query, field, "fulltext", k_limit, require_all); // [(uid, score)]
        
        // 3. RRF Fusion
        // Score = 1 / (C + rank)
        let c_const = 60.0;
        let mut rrf_scores: std::collections::HashMap<u64, f64> = std::collections::HashMap::new();
        
        // Process Vector Results (Rank based on distance asc: 0 is best)
        for (rank, (uid, _dist)) in vec_res.iter().enumerate() {
            let score = 1.0 / (c_const + (rank as f64) + 1.0);
            *rrf_scores.entry(*uid).or_insert(0.0) += score;
        }

        // Process Text Results (Rank based on score desc: 0 is best)
        for (rank, (uid, _score)) in text_res.iter().enumerate() {
            let score = 1.0 / (c_const + (rank as f64) + 1.0);
            *rrf_scores.entry(*uid).or_insert(0.0) += score;
        }
        
        // Result sorted by RRF Score DESC
        let mut result: Vec<(u64, f64)> = rrf_scores.into_iter().collect();
        result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        result.truncate(k);
        
        result
    }


    fn check_condition(&self, stored_val: &Option<Value>, condition: &Value) -> bool {
        // If condition is a Map, it's a Filter Object (eq, gt, etc.)
        // If condition is a Scalar, it's an implicit Equality check (Backward Compat / scalar input)
        
        match condition {
            Value::Object(map) => {
                for (op, target) in map {
                    match op.as_str() {
                        "eq" => {
                             if let Some(sv) = stored_val {
                                 if sv != target { return false; }
                             } else {
                                 if target != &Value::Null { return false; }
                             }
                        }
                        "gt" => {
                            // Comparison Logic (only if types match or are compatible)
                            match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) { if !(sf > tf) { return false; } }
                                },
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    // Try parsing as i64 (Int64 parity)
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                        if !(si > ti) { return false; }
                                    } else if ss <= ts { return false; } // Lexical fallback
                                },
                                _ => {}
                            }
                        }
                        "lt" => {
                            match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) { if !(sf < tf) { return false; } }
                                },
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                        if !(si < ti) { return false; }
                                    } else if ss >= ts { return false; }
                                },
                                _ => {}
                            }
                        }
                        "ge" => {
                            match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) { if !(sf >= tf) { return false; } }
                                },
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                        if !(si >= ti) { return false; }
                                    } else if ss < ts { return false; }
                                },
                                _ => {}
                            }
                        }
                        "le" => {
                             match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) { if !(sf <= tf) { return false; } }
                                },
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                        if !(si <= ti) { return false; }
                                    } else if ss > ts { return false; }
                                },
                                _ => {}
                            }
                        }
                        "contains" => {
                             if let (Some(Value::String(ss)), Value::String(ts)) = (stored_val, target) {
                                 if !ss.contains(ts) { return false; }
                             } else {
                                 return false; 
                             }
                        }
                        "between" => {
                             if let (Some(Value::Number(sn)), Value::List(items)) = (stored_val, target) {
                                 if items.len() == 2 {
                                     if let (Value::Number(min_v), Value::Number(max_v)) = (&items[0], &items[1]) {
                                         if let (Some(sf), Some(min_f), Some(max_f)) = (sn.as_f64(), min_v.as_f64(), max_v.as_f64()) {
                                             if sf < min_f || sf > max_f { return false; }
                                         }
                                     }
                                 }
                             }
                        }
                        "near" => {
                            // target is { "distance": Float, "coordinate": { "latitude": Float, "longitude": Float } }
                            if let Value::Object(near_args) = target {
                                if let (Some(Value::Number(dist_val)), Some(Value::Object(coord_map))) = (near_args.get("distance"), near_args.get("coordinate")) {
                                    if let (Some(Value::Number(lat_val)), Some(Value::Number(lon_val))) = (coord_map.get("latitude"), coord_map.get("longitude")) {
                                        if let (Some(max_meters), Some(target_lat), Some(target_lon)) = (dist_val.as_f64(), lat_val.as_f64(), lon_val.as_f64()) {
                                             // Check stored value
                                             // Stored: { "latitude": ..., "longitude": ... }
                                             if let Some(Value::Object(stored_map)) = stored_val {
                                                 if let (Some(Value::Number(s_lat_v)), Some(Value::Number(s_lon_v))) = (stored_map.get("latitude"), stored_map.get("longitude")) {
                                                     if let (Some(s_lat), Some(s_lon)) = (s_lat_v.as_f64(), s_lon_v.as_f64()) {
                                                         // Haversine Calculation
                                                         let earth_radius_m = 6371000.0;
                                                         let d_lat = (target_lat - s_lat).to_radians();
                                                         let d_lon = (target_lon - s_lon).to_radians();
                                                         let lat1 = s_lat.to_radians();
                                                         let lat2 = target_lat.to_radians();
                                                         
                                                         let a = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
                                                         let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
                                                         let distance = earth_radius_m * c;
                                                         
                                                         if distance > max_meters { return false; }
                                                     }
                                                 }
                                             }
                                        }
                                    }
                                }
                            }
                        }
                        "within" => {
                            // Check if stored Point is WITHIN target Polygon
                            if let Value::Object(polygon) = target {
                                if let Some(Value::Object(stored_map)) = stored_val {
                                     // Parse Stored Point
                                     if let (Some(Value::Number(lat_v)), Some(Value::Number(lon_v))) = (stored_map.get("latitude"), stored_map.get("longitude")) {
                                         if let (Some(lat), Some(lon)) = (lat_v.as_f64(), lon_v.as_f64()) {
                                              let point = geo::Point::new(lon, lat); // Geo uses (x, y) = (lon, lat)
                                              
                                              // Parse Target Polygon
                                              if let Some(Value::List(ext_list)) = polygon.get("exterior") {
                                                  let mut ext_coords = Vec::new();
                                                  for p in ext_list {
                                                      if let Value::Object(pmap) = p {
                                                          if let (Some(Value::Number(plat)), Some(Value::Number(plon))) = (pmap.get("latitude"), pmap.get("longitude")) {
                                                              if let (Some(ylat), Some(xlon)) = (plat.as_f64(), plon.as_f64()) {
                                                                  ext_coords.push((xlon, ylat));
                                                              }
                                                          }
                                                      }
                                                  }
                                                  if !ext_coords.is_empty() {
                                                       let line_string = geo::LineString::from(ext_coords);
                                                       let poly = geo::Polygon::new(line_string, vec![]); 
                                                       use geo::contains::Contains;
                                                       if !poly.contains(&point) { return false; }
                                                  }
                                              }
                                         }
                                     }
                                }
                            }
                        }
                        "intersects" => {
                             // Check if stored Polygon INTERSECTS target Polygon
                             if let Some(Value::Object(stored_map)) = stored_val {
                                 if let Some(Value::List(stored_ext)) = stored_map.get("exterior") {
                                      let mut stored_coords = Vec::new();
                                      for p in stored_ext {
                                           if let Value::Object(pmap) = p {
                                               if let (Some(Value::Number(plat)), Some(Value::Number(plon))) = (pmap.get("latitude"), pmap.get("longitude")) {
                                                   if let (Some(ylat), Some(xlon)) = (plat.as_f64(), plon.as_f64()) {
                                                       stored_coords.push((xlon, ylat));
                                                   }
                                               }
                                           }
                                      }
                                      if !stored_coords.is_empty() {
                                          let stored_poly = geo::Polygon::new(geo::LineString::from(stored_coords), vec![]);
                                          
                                          if let Value::Object(polygon) = target {
                                              if let Some(Value::List(ext_list)) = polygon.get("exterior") {
                                                  let mut target_coords = Vec::new();
                                                  for p in ext_list {
                                                      if let Value::Object(pmap) = p {
                                                          if let (Some(Value::Number(plat)), Some(Value::Number(plon))) = (pmap.get("latitude"), pmap.get("longitude")) {
                                                              if let (Some(ylat), Some(xlon)) = (plat.as_f64(), plon.as_f64()) {
                                                                  target_coords.push((xlon, ylat));
                                                              }
                                                          }
                                                      }
                                                  }
                                                  if !target_coords.is_empty() {
                                                      let target_poly = geo::Polygon::new(geo::LineString::from(target_coords), vec![]);
                                                      use geo::intersects::Intersects;
                                                      if !stored_poly.intersects(&target_poly) { return false; }

                                                  }
                                              }
                                          }
                                      }
                                 }
                             }
                        }
                        "in" => {
                            if let Value::List(list) = target {
                                if let Some(sv) = stored_val {
                                    if !list.contains(sv) { return false; }
                                } else {
                                     // If stored value is null, can it be IN list? Only if list has null.
                                     if !list.contains(&Value::Null) { return false; }
                                }
                            }
                        }
                        "ne" => {
                             if let Some(sv) = stored_val {
                                 if sv == target { return false; }
                             } else {
                                 if target == &Value::Null { return false; }
                             }
                        }
                        _ => {}
                    }
                }
                true
            }
            _ => {
                // Scalar Equality Fallback
                match stored_val {
                    Some(sv) => sv == condition,
                    None => condition == &Value::Null
                }
            }
        }
    }

    fn get_candidates(&self, type_name: &str, filter: &std::collections::HashMap<String, Value>, uniques: &[String]) -> Option<std::collections::HashSet<u64>> {
        // println!("Scan: get_candidates called for {} with filter {:?}", type_name, filter);
        let mut candidates: Option<std::collections::HashSet<u64>> = None;

        for (field, condition) in filter {
            // 1. Check Unique Indexes (Exact Equality)
            // { email: { eq: "..." } } OR { email: "..." }
            let eq_value = match condition {
                Value::Object(map) => map.get("eq"),
                val => Some(val), // Scalar equality
            };

            if let Some(val) = eq_value {
                // If it IS a unique field, we can optimize heavily
                if uniques.contains(field) {
                    if let Ok(val_str) = serde_json::to_string(val) {
                         let index_pred = format!("{}.{}", type_name, field);
                         let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                         
                         match self.storage.get(&self.db_name, &idx_key) {
                             Ok(Some(bytes)) if bytes.len() == 8 => {
                                 let uid = BigEndian::read_u64(&bytes);
                                 let mut set = std::collections::HashSet::new();
                                 set.insert(uid);
                                 
                                 // Intersection
                                 if let Some(current) = candidates {
                                     candidates = Some(current.into_iter().filter(|u| set.contains(u)).collect());
                                 } else {
                                     candidates = Some(set);
                                 }
                                 continue;
                             },
                             _ => {
                                 // Unique field queried, but NO entry found -> Return EMPTY set immediately
                                 return Some(std::collections::HashSet::new());
                             }
                         }
                    }
                }

                // Fallback for non-unique fields or legacy check (kept for safety if uniques list is incomplete?)
                // Actually, if it's NOT in uniques list, we can't assume index exists, so we skip index lookup unless we know we have an index.
                // But previously, it blindly tried to look up ANY field as if it were unique?
                // "We rely on trying to look it up in the Unique Index."
                // Since we now have explicit metadata, we should rely on it.
                // However, let's keep the old logic for "maybe there's an index" if we want to be safe, 
                // BUT the specific optimization of returning EMPTY set can only happen if we are SURE it's a unique field.
                
                if let Ok(val_str) = serde_json::to_string(val) {
                    let index_pred = format!("{}.{}", type_name, field);
                    let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                    if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &idx_key) {
                        if bytes.len() == 8 {
                           let uid = BigEndian::read_u64(&bytes);
                           let mut set = std::collections::HashSet::new();
                           set.insert(uid);
                           
                           // Intersection
                           if let Some(current) = candidates {
                               candidates = Some(current.into_iter().filter(|u| set.contains(u)).collect());
                           } else {
                               candidates = Some(set);
                           }
                           continue; // Optimized this field
                        }
                    }
                }
            }

            // 2. Check Search Indexes
            if let Value::Object(map) = condition {
                 // Handle "allofterms"
                     if let Some(Value::String(terms_str)) = map.get("allofterms") {
                    let terms = crate::engine::tokenizer::Tokenizer::tokenize(terms_str, "term");
                    let mut field_uids = std::collections::HashSet::new();
                    let mut first_term = true;

                    for term in terms {
                        let prefix = Codec::encode_term_index_prefix(field, &term);
                        use std::ops::Bound;
                        let (main_ks, _) = match self.storage.get_database(&self.db_name) {
                             Some(d) => d,
                             None => return Some(std::collections::HashSet::new()),
                        };
                        // use std::ops::Bound; // Removed redundant import
                        let iter = main_ks.range((Bound::Included(prefix.clone()), Bound::Unbounded));
                        
                        let mut term_uids = std::collections::HashSet::new();
                        for guard in iter {
                            if let Ok(key) = guard.key() {
                                if !key.starts_with(&prefix) { break; }
                                if key.len() >= 8 {
                                    let uid = BigEndian::read_u64(&key[key.len()-8..]);
                                    term_uids.insert(uid);
                                }
                            }
                        }

                        if first_term {
                            field_uids = term_uids;
                            first_term = false;
                        } else {
                            field_uids.retain(|u| term_uids.contains(u));
                        }
                    }
                    
                    if let Some(current) = candidates {
                        candidates = Some(current.into_iter().filter(|u| field_uids.contains(u)).collect());
                    } else {
                        candidates = Some(field_uids);
                    }
                }
                
                // Handle "anyofterms"
                if let Some(Value::String(terms_str)) = map.get("anyofterms") {
                     let terms = crate::engine::tokenizer::Tokenizer::tokenize(terms_str, "term");
                     let mut field_uids = std::collections::HashSet::new();
                     
                     for term in terms {
                        let prefix = Codec::encode_term_index_prefix(field, &term);
                        use std::ops::Bound;
                        let (main_ks, _) = match self.storage.get_database(&self.db_name) {
                             Some(d) => d,
                             None => return Some(std::collections::HashSet::new()),
                        };
                        // use std::ops::Bound; // Removed redundant import
                        let iter = main_ks.range((Bound::Included(prefix.clone()), Bound::Unbounded));
                        
                        for guard in iter {
                            if let Ok(key) = guard.key() {
                                if !key.starts_with(&prefix) { break; }
                                if key.len() >= 8 {
                                    let uid = BigEndian::read_u64(&key[key.len()-8..]);
                                    field_uids.insert(uid);
                                }
                            }
                        }
                     }
                     if let Some(current) = candidates {
                        candidates = Some(current.into_iter().filter(|u| field_uids.contains(u)).collect());
                     } else {
                        candidates = Some(field_uids);
                     }
                }

                if let Some(Value::String(terms_str)) = map.get("alloftext") {
                    let terms = crate::engine::tokenizer::Tokenizer::tokenize(terms_str, "fulltext");
                    let index_field = format!("{}.fulltext", field);
                    let mut field_uids = std::collections::HashSet::new();
                    let mut first_term = true;

                    for term in terms {
                        let prefix = Codec::encode_term_index_prefix(&index_field, &term);
                        use std::ops::Bound;
                        let (main_ks, _) = match self.storage.get_database(&self.db_name) {
                             Some(d) => d,
                             None => return Some(std::collections::HashSet::new()),
                        };
                        // use std::ops::Bound; // Removed redundant import
                        let iter = main_ks.range((Bound::Included(prefix.clone()), Bound::Unbounded));
                        
                        let mut term_uids = std::collections::HashSet::new();
                        for guard in iter {
                            if let Ok(key) = guard.key() {
                                if !key.starts_with(&prefix) { break; }
                                if key.len() >= 8 {
                                    let uid = BigEndian::read_u64(&key[key.len()-8..]);
                                    term_uids.insert(uid);
                                }
                            }
                        }

                        if first_term {
                            field_uids = term_uids;
                            first_term = false;
                        } else {
                            field_uids.retain(|u| term_uids.contains(u));
                        }
                    }
                    
                    if let Some(current) = candidates {
                        candidates = Some(current.into_iter().filter(|u| field_uids.contains(u)).collect());
                    } else {
                        candidates = Some(field_uids);
                    }
                }

                // Handle "anyoftext" (Stemmed)
                if let Some(Value::String(terms_str)) = map.get("anyoftext") {
                     let terms = crate::engine::tokenizer::Tokenizer::tokenize(terms_str, "fulltext");
                     let index_field = format!("{}.fulltext", field);
                     let mut field_uids = std::collections::HashSet::new();
                     
                     for term in terms {
                        let prefix = Codec::encode_term_index_prefix(&index_field, &term);
                        use std::ops::Bound;
                        let (main_ks, _) = match self.storage.get_database(&self.db_name) {
                             Some(d) => d,
                             None => return Some(std::collections::HashSet::new()),
                        };
                        // use std::ops::Bound; // Removed redundant import
                        let iter = main_ks.range((Bound::Included(prefix.clone()), Bound::Unbounded));
                        
                        for guard in iter {
                            if let Ok(key) = guard.key() {
                                if !key.starts_with(&prefix) { break; }
                                if key.len() >= 8 {
                                    let uid = BigEndian::read_u64(&key[key.len()-8..]);
                                    field_uids.insert(uid);
                                }
                            }
                        }
                     }
                     if let Some(current) = candidates {
                        candidates = Some(current.into_iter().filter(|u| field_uids.contains(u)).collect());
                     } else {
                        candidates = Some(field_uids);
                     }
                }
            }
        }
        candidates
    }
    // Updated check_filter_recursive to support Deep Filtering (Relation Traversal)
    pub fn check_filter_recursive(&self, uid: u64, filter: &indexmap::IndexMap<async_graphql::Name, Value>) -> bool {
        for (key, condition) in filter {
            match key.as_str() {
                "and" => {
                    if let Value::List(list) = condition {
                        for sub in list {
                            if let Value::Object(map) = sub {
                                if !self.check_filter_recursive(uid, map) { return false; }
                            }
                        }
                    }
                }
                "or" => {
                    if let Value::List(list) = condition {
                         let mut any = false;
                         for sub in list {
                             if let Value::Object(map) = sub {
                                 if self.check_filter_recursive(uid, map) { any = true; break; }
                             }
                         }
                         if !any { return false; }
                    }
                }
                "not" => {
                    if let Value::Object(map) = condition {
                        if self.check_filter_recursive(uid, map) { return false; }
                    }
                }
                field_name => {
                     let d_key = Codec::encode_data_key(uid, field_name);
                     let stored_val = if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &d_key) {
                         serde_json::from_slice::<Value>(&bytes).ok()
                     } else { None };
                     
                     // relation traversal check
                     // If condition is an Object and key is NOT a known operator (eq, gt...), it might be a Relation Filter
                     if let Value::Object(sub_filter) = condition {
                         // Check if this is a standard operator map (eq, gt) OR a nested filter
                         let is_operator_map = sub_filter.keys().any(|k| ["eq", "gt", "lt", "ge", "le", "contains", "between", "near", "within", "intersects", "in", "ne", "allofterms", "anyofterms", "alloftext", "anyoftext"].contains(&k.as_str()));
                         
                         if !is_operator_map {
                             // It's a Nested Relation Filter!
                             // stored_val should be a UID (String/Number) or List of UIDs
                             match stored_val {
                                 Some(Value::String(s)) => {
                                     if let Ok(child_uid) = s.parse::<u64>() {
                                         if !self.check_filter_recursive(child_uid, sub_filter) { return false; }
                                     } else { return false; }
                                 },
                                 Some(Value::Number(n)) => {
                                     if let Some(child_uid) = n.as_u64() {
                                         if !self.check_filter_recursive(child_uid, sub_filter) { return false; }
                                     } else { return false; }
                                 },
                                 Some(Value::List(list)) => {
                                     // 1:M Relation - "Some" semantics (match if ANY child matches)
                                     let mut match_found = false;
                                     for item in list {
                                         let u_opt = match item {
                                             Value::String(s) => s.parse::<u64>().ok(),
                                             Value::Number(n) => n.as_u64(),
                                             _ => None
                                         };
                                         if let Some(child_uid) = u_opt {
                                             if self.check_filter_recursive(child_uid, sub_filter) {
                                                 match_found = true;
                                                 break;
                                             }
                                         }
                                     }
                                     if !match_found { return false; }
                                 },
                                 _ => return false // Relation field is null or invalid
                             }
                             continue;
                         }
                     }

                     if !self.check_condition(&stored_val, condition) { return false; }
                }
            }
        }
        true
    }

    pub fn create_node_internal(&self, type_name: &str, uid: u64, mut fields: std::collections::HashMap<String, serde_json::Value>, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>, source: crate::realtime::bus::MutationSource, timestamp_override: Option<crate::storage::timestamp::Timestamp>) -> Result<(), String> {
        let fn_start = std::time::Instant::now();
        
        // Normalize fields: If value is Object with uid/id, flatten to String(uid)
        for (_, value) in fields.iter_mut() {
            if let serde_json::Value::Object(map) = value {
                let uid_val = map.get("uid").or(map.get("id"));
                if let Some(u) = uid_val {
                    match u {
                        serde_json::Value::String(s) => *value = serde_json::Value::String(s.clone()),
                        serde_json::Value::Number(n) => *value = serde_json::Value::String(n.to_string()),
                        _ => {}
                    }
                }
            } else if let serde_json::Value::Array(list) = value {
                // Handle List of Objects
                for item in list.iter_mut() {
                    if let serde_json::Value::Object(map) = item {
                        let uid_val = map.get("uid").or(map.get("id"));
                        if let Some(u) = uid_val {
                             match u {
                                serde_json::Value::String(s) => *item = serde_json::Value::String(s.clone()),
                                serde_json::Value::Number(n) => *item = serde_json::Value::String(n.to_string()),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Generate Timestamp for this Atomic Mutation or use override
        let timestamp = timestamp_override.unwrap_or_else(|| self.storage.next_timestamp());

        // === Single WriteBatch approach ===
        // Acquire keyspace lock ONCE (was 4-5 times before)
        let keyspaces = self.storage.keyspaces.read().unwrap();
        let (main, _history) = keyspaces.get(&self.db_name)
            .ok_or_else(|| format!("Database not found: {}", self.db_name))?;
        
        let mut batch = self.storage.db.batch();
        let ts_bytes = timestamp.to_bytes();

        // 1. Type Index — add to batch instead of separate storage.insert()
        let type_key_idx = Codec::encode_type_index_key(type_name, uid);
        batch.insert(main, &type_key_idx, &[]);

        // 2. _type data field
        let type_val_bytes = serde_json::to_vec(&serde_json::Value::String(type_name.to_string())).expect("Serialization failed");
        let type_data_key = Codec::encode_data_key(uid, "_type");
        let mut type_val_buf = Vec::with_capacity(16 + type_val_bytes.len());
        type_val_buf.extend_from_slice(&ts_bytes);
        type_val_buf.extend_from_slice(&type_val_bytes);
        batch.insert(main, &type_data_key, &type_val_buf);

        // 3. User fields + unique checks
        let mut items_to_index = Vec::new();
        let uniq_start = std::time::Instant::now();
        for (field, value) in &fields {
            let val_bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;

            if let Some(tokenizers) = search_fields.get(field) {
                 if let serde_json::Value::String(s) = value {
                     items_to_index.push((field.clone(), s.clone(), tokenizers.clone()));
                 }
            }
            
            if uniques.contains(&field) {
                 let index_pred = format!("{}.{}", type_name, field);
                 let val_str = serde_json::to_string(&value).map_err(|e| e.to_string())?;
                 let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                 // Unique check read — reads don't trigger backpressure
                 if main.get(&idx_key).map_err(|e| e.to_string())?.is_some() {
                     return Err(format!("Duplicate value for unique field: {}", field));
                 }
                 let mut uid_bytes = vec![0u8; 8];
                 BigEndian::write_u64(&mut uid_bytes, uid);
                 batch.insert(main, &idx_key, &uid_bytes);
            }
            
            // Data field with timestamp prefix
            let key = Codec::encode_data_key(uid, &field);
            let mut val_buf = Vec::with_capacity(16 + val_bytes.len());
            val_buf.extend_from_slice(&ts_bytes);
            val_buf.extend_from_slice(&val_bytes);
            batch.insert(main, &key, &val_buf);
        }
        let uniq_time = uniq_start.elapsed();

        // 4. Inverse edges — list-type added to batch, non-list deferred
        let mut deferred_inverses: Vec<(u64, String, bool)> = Vec::new();
        let mut inv_count = 0u32;
        for info in inverses {
             if let Some(val) = fields.get(&info.field) {
                  let mut new_targets = Vec::new();
                  match val {
                      serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                      serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                      serde_json::Value::Object(map) => {
                          if let Some(id_val) = map.get("id") {
                              match id_val {
                                  serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                                  serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                                  _ => {}
                              }
                          }
                      }
                      serde_json::Value::Array(items) => {
                          for item in items {
                              match item {
                                    serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                                    serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                                    serde_json::Value::Object(map) => {
                                        if let Some(id_val) = map.get("id") {
                                            match id_val {
                                                serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                                                serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                                                _ => {}
                                            }
                                        }
                                    }
                                    _ => {}
                              }
                          }
                      }
                      _ => {}
                  }
                  for target in new_targets {
                      if info.inverse_is_list {
                          // O(1) edge insert — add directly to batch
                          let edge_key = Codec::encode_edge_key(target, &info.inverse_field, uid);
                          batch.insert(main, &edge_key, &[]);
                      } else {
                          // Non-list needs read-before-write, defer after commit
                          deferred_inverses.push((target, info.inverse_field.clone(), false));
                      }
                      inv_count += 1;
                  }
              }
        }

        // 5. Single atomic commit for ALL writes
        let commit_start = std::time::Instant::now();
        batch.commit().map_err(|e| e.to_string())?;
        let commit_time = commit_start.elapsed();

        // Check L0 pressure after commit
        let l0_count = main.l0_table_count();
        
        // Release keyspace lock before any deferred work
        drop(keyspaces);

        if l0_count >= 8 {
            // Re-acquire for compaction (rare path)
            let keyspaces = self.storage.keyspaces.read().unwrap();
            if let Some((main, _)) = keyspaces.get(&self.db_name) {
                let compact_start = std::time::Instant::now();
                if crate::debug_logging() {
                    println!("⚠️ L0 pressure high (l0_tables={}), triggering compaction...", l0_count);
                }
                let _ = main.major_compact();
                if crate::debug_logging() {
                    println!("✅ Auto-compaction complete ({:?}, l0_tables={})",
                             compact_start.elapsed(), main.l0_table_count());
                }
            }
        }

        // 6. Handle deferred non-list inverse links (rare for this workload)
        for (target, inverse_field, _is_list) in deferred_inverses {
            self.link_inverse(target, &inverse_field, false, uid, &timestamp)?;
        }
        
        // 7. Handle Search Indexing (After commit/lock release)
        for (field, val, tokenizers) in items_to_index {
             for strategy in tokenizers {
                 if let Err(e) = self.write_term_index(uid, &field, &val, &strategy) {
                     eprintln!("Search Indexing Failed (create_node) for uid={}: {}", uid, e);
                 }
             }
        }

        let total = fn_start.elapsed();
        if crate::debug_logging() && total.as_millis() > 2 {
            eprintln!("[RESOLVER] create_node {} | uniq={:?} commit={:?} inv_count={} total={:?}",
                     type_name, uniq_time, commit_time, inv_count, total);
        }

        // 8. Publish Event (Realtime)
        self.bus.publish(MutationEvent {
             type_name: type_name.to_string(),
             uid,
             mutation_type: MutationType::Create,
             source,
             payload: Some(fields),
             metadata: Some(crate::realtime::bus::SchemaMetadata {
                uniques: uniques.to_vec(),
                inverses: inverses.to_vec(),
                search_fields: search_fields.clone(),
            }),
            timestamp: Some(timestamp),
        });

        Ok(())
    }

    pub fn update_node_internal(&self, type_name: &str, uid: u64, mut fields: std::collections::HashMap<String, serde_json::Value>, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>, source: crate::realtime::bus::MutationSource, timestamp_override: Option<crate::storage::timestamp::Timestamp>) -> Result<(), String> {
        // Normalize fields: If value is Object with uid/id, flatten to String(uid)
        for (_, value) in fields.iter_mut() {
            if let serde_json::Value::Object(map) = value {
                let uid_val = map.get("uid").or(map.get("id"));
                if let Some(u) = uid_val {
                    match u {
                        serde_json::Value::String(s) => *value = serde_json::Value::String(s.clone()),
                        serde_json::Value::Number(n) => *value = serde_json::Value::String(n.to_string()),
                        _ => {}
                    }
                }
            } else if let serde_json::Value::Array(list) = value {
                // Handle List of Objects
                for item in list.iter_mut() {
                    if let serde_json::Value::Object(map) = item {
                        let uid_val = map.get("uid").or(map.get("id"));
                        if let Some(u) = uid_val {
                             match u {
                                serde_json::Value::String(s) => *item = serde_json::Value::String(s.clone()),
                                serde_json::Value::Number(n) => *item = serde_json::Value::String(n.to_string()),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        let timestamp = timestamp_override.unwrap_or_else(|| self.storage.next_timestamp());
         // 0. Remove Old Search Indexes for updated fields
        for (field, _) in &fields {
             if let Some(tokenizers) = search_fields.get(field) {
                 let data_key = Codec::encode_data_key(uid, field);
                 if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                     if let Ok(serde_json::Value::String(s)) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                          for strategy in tokenizers {
                              self.remove_term_index(uid, field, &s, strategy)?;
                          }
                     }
                 }
             }
        }
        // 1. Unlink Inverses
        for info in inverses {
             if fields.contains_key(&info.field) {
                 let data_key = Codec::encode_data_key(uid, &info.field);
                 if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                     let mut old_targets = Vec::new();
                     if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                          match val {
                               serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { old_targets.push(id); } }
                               serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { old_targets.push(id); } }
                               serde_json::Value::Object(map) => {
                                   // Try "uid" then "id"
                                   if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                                       match uid_val {
                                           serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { old_targets.push(id); } }
                                           serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { old_targets.push(id); } }
                                           _ => {}
                                       }
                                   }
                               }
                               serde_json::Value::Array(items) => {
                                   for item in items {
                                       match item {
                                             serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { old_targets.push(id); } }
                                             serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { old_targets.push(id); } }
                                             serde_json::Value::Object(map) => {
                                                  if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                                                       match uid_val {
                                                           serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { old_targets.push(id); } }
                                                           serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { old_targets.push(id); } }
                                                           _ => {}
                                                       }
                                                  }
                                             }
                                             _ => {}
                                       }
                                   }
                               }
                               _ => {}
                          }
                     }
                     for target in old_targets {
                         self.unlink_inverse(target, &info.inverse_field, info.inverse_is_list, uid, &timestamp)?;
                     }
                 }
             }
        }
        // 2. Remove Old Unique Indexes
        for field in uniques {
             if fields.contains_key(field) {
                 let data_key = Codec::encode_data_key(uid, field);
                 if let Ok(Some(val_bytes)) = self.storage.get(&self.db_name, &data_key) {
                     if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&val_bytes) {
                          let val_str = serde_json::to_string(&val).unwrap_or_default();
                          let index_pred = format!("{}.{}", type_name, field);
                          let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                          self.storage.remove(&self.db_name, &idx_key).map_err(|e| e.to_string())?;
                     }
                 }
             }
        }
        // 3. Write New Data & Indexes
        let mut batch_items = Vec::new();
        for (field, value) in &fields {
            let val_bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
            if let Some(tokenizers) = search_fields.get(field) {
                if let serde_json::Value::String(s) = value {
                     for strategy in tokenizers {
                         self.write_term_index(uid, field, s, strategy)?;
                     }
                }
            }
            if uniques.contains(&field) {
                 let index_pred = format!("{}.{}", type_name, field);
                 let val_str = serde_json::to_string(&value).map_err(|e| e.to_string())?;
                 let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                 if let Ok(Some(_)) = self.storage.get(&self.db_name, &idx_key) {
                     return Err(format!("Duplicate value for unique field: {}", field));
                 }
                 let mut uid_bytes = vec![0u8; 8];
                 BigEndian::write_u64(&mut uid_bytes, uid);
                 self.storage.insert(&self.db_name, &idx_key, &uid_bytes).map_err(|e| e.to_string())?;
            }
            batch_items.push((uid, field.clone(), val_bytes));
        }
        
        self.storage.put_batch_lww(&self.db_name, batch_items, &timestamp).map_err(|e| e.to_string())?;
        // 4. Link New Inverses
        for info in inverses {
             if let Some(val) = fields.get(&info.field) {
                  let mut new_targets = Vec::new();
                  match val {
                      serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                      serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                      serde_json::Value::Object(map) => {
                           // Try "uid" then "id"
                           if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                               match uid_val {
                                   serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                                   serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                                   _ => {}
                               }
                           }
                      }
                      serde_json::Value::Array(items) => {
                          for item in items {
                              match item {
                                    serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                                    serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                                    serde_json::Value::Object(map) => {
                                         if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                                              match uid_val {
                                                  serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                                                  serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                                                  _ => {}
                                              }
                                         }
                                    }
                                    _ => {}
                              }
                          }
                      }
                      _ => {}
                  }
                  for target in new_targets {
                      self.link_inverse(target, &info.inverse_field, info.inverse_is_list, uid, &timestamp)?;
                  }
             }
        }

        // Inject ID into payload for Frontend Cache compatibility
        let mut event_payload = fields;
        event_payload.insert("id".to_string(), serde_json::Value::String(uid.to_string()));

        self.bus.publish(MutationEvent {
            type_name: type_name.to_string(),
            uid,
            mutation_type: MutationType::Update,
            source,
            payload: Some(event_payload),
            metadata: Some(crate::realtime::bus::SchemaMetadata {
                uniques: uniques.to_vec(),
                inverses: inverses.to_vec(),
                search_fields: search_fields.clone(),
            }),
            timestamp: Some(timestamp),
        });
        Ok(())
    }

    pub fn delete_node_internal(&self, type_name: &str, uid: u64, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>, source: crate::realtime::bus::MutationSource, timestamp_override: Option<crate::storage::timestamp::Timestamp>) -> Result<(), String> {
        let timestamp = timestamp_override.unwrap_or_else(|| self.storage.next_timestamp());
        // 0. Remove Search Indexes
        for (field, tokenizers) in search_fields {
            let data_key = Codec::encode_data_key(uid, field);
            if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                if let Ok(serde_json::Value::String(s)) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                     for strategy in tokenizers {
                         self.remove_term_index(uid, field, &s, strategy)?;
                     }
                }
            }
        }
        // 1. Handle Inverses (Unlink)
        for info in inverses {
             let data_key = Codec::encode_data_key(uid, &info.field);
             if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                 let mut targets = Vec::new();
                 if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                      match val {
                           serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { targets.push(id); } }
                           serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { targets.push(id); } }
                           serde_json::Value::Object(map) => {
                               if let Some(uid_val) = map.get("uid").or_else(|| map.get("id")) {
                                   if let Some(s) = uid_val.as_str() { if let Ok(id) = s.parse::<u64>() { targets.push(id); } }
                               }
                           }
                           serde_json::Value::Array(items) => {
                               for item in items {
                                   match item {
                                        serde_json::Value::String(s) => { if let Ok(id) = s.parse::<u64>() { targets.push(id); } }
                                        serde_json::Value::Number(n) => { if let Some(id) = n.as_u64() { targets.push(id); } }
                                        serde_json::Value::Object(map) => {
                                            if let Some(uid_val) = map.get("uid").or_else(|| map.get("id")) {
                                                if let Some(s) = uid_val.as_str() { if let Ok(id) = s.parse::<u64>() { targets.push(id); } }
                                            }
                                        }
                                        _ => {}
                                   }
                               }
                           }
                           _ => {}
                      }
                  }
                  for target in targets {
                      self.unlink_inverse(target, &info.inverse_field, info.inverse_is_list, uid, &timestamp)?;
                  }
             }
        }
        // 2. Remove Unique Indexes
        for field in uniques {
            let data_key = Codec::encode_data_key(uid, field);
            if let Ok(Some(val_bytes)) = self.storage.get(&self.db_name, &data_key) {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&val_bytes) {
                     let val_str = serde_json::to_string(&val).unwrap_or_default();
                     let index_pred = format!("{}.{}", type_name, field);
                     let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                     self.storage.remove(&self.db_name, &idx_key).map_err(|e| e.to_string())?;
                }
             }
        }
        // 3. Remove Type Index
        let type_key = Codec::encode_type_index_key(type_name, uid);
        self.storage.remove(&self.db_name, &type_key).map_err(|e| e.to_string())?;

        // 4. Remove Vector Data (Soft Delete)
        // We delete indiscriminately; if no vector existed, it's a safe no-op.
        self.storage.delete_vector(uid).map_err(|e| e.to_string())?;

        // 5. Remove Data Keys (Scan Prefix)
        let prefix = Codec::encode_data_prefix(uid);
        use std::ops::Bound;
        let (main_ks, _) = self.storage.get_database(&self.db_name).ok_or("Database not found".to_string())?;
        let iter = main_ks.range((Bound::Included(prefix.clone()), Bound::Unbounded));
        let mut keys_to_delete: Vec<Vec<u8>> = Vec::new();
        for guard in iter {
             if let Ok(key) = guard.key() {
             if !key.starts_with(&prefix) { break; }
             keys_to_delete.push(key.to_vec());
             }
        }
        for k in keys_to_delete {
            if k.len() > 9 {
                if let Ok(pred) = std::str::from_utf8(&k[9..]) {
                     self.storage.delete_with_lww(&self.db_name, uid, pred, &timestamp).map_err(|e| e.to_string())?;
                }
            }
        }

        self.bus.publish(MutationEvent {
             type_name: type_name.to_string(),
             uid,
             mutation_type: MutationType::Delete,
             source,
             payload: None,
             metadata: Some(crate::realtime::bus::SchemaMetadata {
                uniques: uniques.to_vec(),
                inverses: inverses.to_vec(),
                search_fields: search_fields.clone(),
            }),
            timestamp: Some(timestamp),
        });
        Ok(())
    }

    pub fn apply_remote_mutation(&self, event: crate::realtime::bus::MutationEvent) -> Result<(), String> {
         let metadata = event.metadata.ok_or("Missing metadata for remote mutation")?;
         let source = crate::realtime::bus::MutationSource::Remote;
         
         let result = match event.mutation_type {
             crate::realtime::bus::MutationType::Create => {
                  let payload = event.payload.clone().ok_or("Missing payload for Create")?;
                  self.create_node_internal(&event.type_name, event.uid, payload, &metadata.uniques, &metadata.inverses, &metadata.search_fields, source, event.timestamp)
             },
             crate::realtime::bus::MutationType::Update => {
                  let payload = event.payload.clone().ok_or("Missing payload for Update")?;
                  self.update_node_internal(&event.type_name, event.uid, payload, &metadata.uniques, &metadata.inverses, &metadata.search_fields, source, event.timestamp)
             },
             crate::realtime::bus::MutationType::Delete => {
                  self.delete_node_internal(&event.type_name, event.uid, &metadata.uniques, &metadata.inverses, &metadata.search_fields, source, event.timestamp)
             }
         };

         if let Err(e) = result {
             eprintln!("Quarantining mutation due to error: {}", e);
             let timestamp = self.storage.next_timestamp();
             if let Some(payload) = event.payload {
                 for (field, value) in payload {
                     if let Ok(bytes) = serde_json::to_vec(&value) {
                         let _ = self.storage.put_quarantine(event.uid, &field, &bytes, &timestamp);
                     }
                 }
             }
             return Err(e);
         }
         Ok(())
    }
}

impl Resolver for FjallResolver {
    fn resolve_list(&self, parent_uid: u64, field_name: &str, filter: std::collections::HashMap<String, Value>, sort: std::collections::HashMap<String, Value>, first: Option<usize>, after: Option<String>, near_vector: Option<Vec<f64>>) -> Result<Vec<u64>, String> {
        // 1. Resolve the List Field from Storage
        // First check the legacy data key (for directly-assigned lists)
        let key = Codec::encode_data_key(parent_uid, field_name);
        
        let mut uids: Vec<u64> = if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &key) {
             if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                 match val {
                     Value::List(list) => {
                         let mut result = Vec::new();
                         for item in list {
                             match item {
                                  Value::String(s) => { if let Ok(u) = s.parse::<u64>() { result.push(u); } },
                                  Value::Number(n) => { if let Some(u) = n.as_u64() { result.push(u); } },
                                  Value::Object(map) => {
                                      if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                                          match uid_val {
                                              Value::String(s) => { if let Ok(u) = s.parse::<u64>() { result.push(u); } },
                                              Value::Number(n) => { if let Some(u) = n.as_u64() { result.push(u); } },
                                              _ => {}
                                          }
                                      }
                                  },
                                  _ => {}
                             }
                         }
                         result
                     },
                     Value::String(s) => { if let Ok(u) = s.parse::<u64>() { vec![u] } else { vec![] } },
                     Value::Number(n) => { if let Some(u) = n.as_u64() { vec![u] } else { vec![] } },
                     Value::Object(map) => {
                         if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                             match uid_val {
                                 Value::String(s) => { if let Ok(u) = s.parse::<u64>() { vec![u] } else { vec![] } },
                                 Value::Number(n) => { if let Some(u) = n.as_u64() { vec![u] } else { vec![] } },
                                 _ => vec![]
                             }
                         } else { vec![] }
                     },
                     _ => vec![]
                 }
             } else { vec![] }
        } else { vec![] };

        // Also scan edge index keys (for inverse-linked lists)
        let edge_prefix = Codec::encode_edge_prefix(parent_uid, field_name);
        if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
            use std::ops::Bound;
            let iter = main_ks.range((Bound::Included(edge_prefix.clone()), Bound::Unbounded));
            for guard in iter {
                if let Ok(key) = guard.key() {
                    if !key.starts_with(&edge_prefix) { break; }
                    if let Some(source_uid) = Codec::decode_edge_source_uid(&key) {
                        uids.push(source_uid);
                    }
                }
            }
        }
        
        // 2. Vector Sort/Filter (Strategy B: Fetch & Sort)
        if let Some(ref vec) = near_vector {
             // We have the set of relevant UIDs.
             // We want to sort them by distance to `vec`.
             // We need to fetch the `embedding` field for each.
             // What field name holds the embedding? Default: "embedding" per schema.
             // Ideally we should know the vector field name from metadata, but schema.rs passes logic.
             // Let's assume field is named "embedding".
             // We can check `resolve(uid, "embedding")`.
             
             let mut uid_dists = Vec::new();
             for uid in &uids {
                 if let Some(val) = self.resolve(*uid, "embedding") {
                     if let Value::List(floats) = val {
                         let embed: Vec<f64> = floats.iter().filter_map(|v| match v {
                             Value::Number(n) => n.as_f64(),
                             _ => None
                         }).collect();
                         
                         // Compute Cosine Distance
                         // Check dims
                         if embed.len() == vec.len() {
                             // Simple Euclidean or Cosine? HNSW usually Cosine/Dot.
                             // Let's use Cosine Similarity -> Distance = 1 - Sim
                             let dot: f64 = embed.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
                             let norm_a: f64 = embed.iter().map(|a| a * a).sum::<f64>().sqrt();
                             let norm_b: f64 = vec.iter().map(|b| b * b).sum::<f64>().sqrt();
                             
                             if norm_a > 0.0 && norm_b > 0.0 {
                                 let sim = dot / (norm_a * norm_b);
                                 let dist = 1.0 - sim;
                                 uid_dists.push((*uid, dist));
                             } else {
                                 uid_dists.push((*uid, f64::MAX));
                             }
                         }
                     }
                 }
             }
             
             // Sort by Distance ASC
             uid_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
             
             // Update uids
             uids = uid_dists.into_iter().map(|(u, _)| u).collect();
        }

        // 3. Filter (Standard)
        if !filter.is_empty() {
             let mut filter_im = indexmap::IndexMap::new();
             for (k, v) in &filter {
                  filter_im.insert(async_graphql::Name::new(k), v.clone());
             }
             uids.retain(|uid| self.check_filter_recursive(*uid, &filter_im));
        }
        
        // 4. Sort (Explicit sort overrides Vector sort)
        if !sort.is_empty() {
             if let Some((field, direction)) = sort.iter().next() {
                 let asc = match direction {
                     Value::String(s) => s == "ASC",
                     Value::Enum(n) => n.as_str() == "ASC",
                      _ => true
                 };
                 
                 uids.sort_by(|a, b| {
                     let val_a = self.resolve(*a, field);
                     let val_b = self.resolve(*b, field);
                     
                     let cmp = match (val_a, val_b) {
                         (Some(Value::Number(na)), Some(Value::Number(nb))) => {
                              na.as_f64().partial_cmp(&nb.as_f64()).unwrap_or(std::cmp::Ordering::Equal)
                         },
                         (Some(Value::String(sa)), Some(Value::String(sb))) => {
                             sa.cmp(&sb)
                         },
                          (None, Some(_)) => std::cmp::Ordering::Less,
                          (Some(_), None) => std::cmp::Ordering::Greater,
                          _ => std::cmp::Ordering::Equal
                     };
                     
                     if asc { cmp } else { cmp.reverse() }
                 });
             }
        }
        
        // 5. Pagination
        if let Some(cursor_uid_str) = after {
             // We assume cursor is UID based (like scan_nodes fallback) or simple list indexing?
             // Since we have the whole list, we perform cursor pagination relative to valid UIDs.
             // If cursor key is UID (primary key):
             if let Ok(cursor_uid) = cursor_uid_str.parse::<u64>() {
                 if let Some(pos) = uids.iter().position(|u| *u == cursor_uid) {
                     uids = uids.into_iter().skip(pos + 1).collect();
                 }
             }
        }
        
        if let Some(limit) = first {
            uids.truncate(limit);
        }

        Ok(uids)
    }

    fn search_vectors(&self, query: &[f64], k: usize) -> Vec<(u64, f64)> {
        match self.storage.search_vectors(query, k) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("Vector Search Error: {}", e);
                vec![]
            }
        }
    }

    fn search_hybrid(&self, text: &str, field: &str, vector: &[f64], k: usize) -> Vec<(u64, f64)> {
        self.search_hybrid(text, field, vector, k, false)
    }

    fn resolve(&self, uid: u64, field_name: &str) -> Option<Value> {
        if field_name == "id" {
            return Some(Value::String(uid.to_string()));
        }

        let key = Codec::encode_data_key(uid, field_name);
        match self.storage.get(&self.db_name, &key) {
            Ok(Some(bytes)) => {
                let res = serde_json::from_slice(&bytes).ok();
                res
            }
            _ => {
                // FALLBACK: Check Edge Index (for Inverse Relationships)
                // If this field is a relationship (e.g., `posts` on `User`), it might only exist as edge keys.
                // We scan for edges starting with prefix derived from (uid, field_name).
                let edge_prefix = Codec::encode_edge_prefix(uid, field_name);
                if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                    let mut edge_uids = Vec::new();
                    use std::ops::Bound;
                    let iter = main_ks.range((Bound::Included(edge_prefix.clone()), Bound::Unbounded));
                    for guard in iter {
                        if let Ok(key) = guard.key() {
                             if !key.starts_with(&edge_prefix) { break; }
                             if let Some(target_uid) = Codec::decode_edge_source_uid(&key) {
                                 edge_uids.push(target_uid);
                             }
                        }
                    }
                    if !edge_uids.is_empty() {
                         // Return as List of strings (IDs)
                         let list: Vec<Value> = edge_uids.into_iter().map(|u| Value::String(u.to_string())).collect();
                         return Some(Value::List(list));
                    }
                }

                None
            },
        }
    }

    fn find_uid(&self, index_name: &str, value: &str) -> Option<u64> {
        let key = Codec::encode_unique_index_key(index_name, value);
        match self.storage.get(&self.db_name, &key) {
             Ok(Some(bytes)) if bytes.len() == 8 => {
                 Some(BigEndian::read_u64(&bytes))
             }
             _ => None,
        }
    }

    fn create_node(&self, type_name: &str, mut fields: std::collections::HashMap<String, Value>, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>, vector_config: Option<&crate::engine::resolver::VectorConfig>) -> Result<u64, String> {
        let op_start = std::time::Instant::now();
        let start = std::time::SystemTime::now();
        let since_the_epoch = start.duration_since(std::time::UNIX_EPOCH).expect("Time went backwards");
        let uid = since_the_epoch.as_nanos() as u64;

        // Automatic Embedding Generation
        if let Some(config) = vector_config {
            if !fields.contains_key(&config.field) {
                if let Some(Value::String(text)) = fields.get(&config.source) {
                    // Start Timer
                    let _embed_start = std::time::Instant::now();
                    match self.storage.embedding_model.lock().unwrap().embed(vec![text.clone()], None) {
                        Ok(embeddings) => {
                             if let Some(first) = embeddings.first() {
                                 let json_values: Vec<Value> = first.iter().map(|f| Value::Number(async_graphql::Number::from_f64((*f).into()).unwrap_or(async_graphql::Number::from(0))))
                                     .collect();
                                 fields.insert(config.field.clone(), Value::List(json_values));
                                 // println!("Auto-Embedded field {} from {} in {:.2}ms", config.field, config.source, embed_start.elapsed().as_secs_f64() * 1000.0);
                             }
                        },
                        Err(e) => {
                            eprintln!("Failed to generate embedding: {}", e);
                            // We continue without embedding? Or fail?
                            // For now continue, maybe user wants to retry later.
                        }
                    }
                }
            }
        
            // HNSW Indexing (Manual OR Auto)
            if let Some(val) = fields.get(&config.field) {
                if let Value::List(list) = val {
                    let vec_data: Vec<f64> = list.iter().filter_map(|v| match v {
                        Value::Number(n) => n.as_f64(),
                        _ => None
                    }).collect();
                    if !vec_data.is_empty() {
                        let storage = self.storage.clone();
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = storage.put_vector(uid, vec_data) {
                                eprintln!("Background Vector Insert Error (UID {}): {}", uid, e);
                            }
                        });
                    }
                }
            }
        }

        let payload: std::collections::HashMap<String, serde_json::Value> = fields.iter()
            .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(serde_json::Value::Null)))
            .collect();

        self.create_node_internal(type_name, uid, payload, uniques, inverses, search_fields, crate::realtime::bus::MutationSource::Local, None)?;
        
        let elapsed = op_start.elapsed();
        if crate::debug_logging() && elapsed.as_secs() >= 1 {
            println!("SLOW: create_node for {} took {:.2}s", type_name, elapsed.as_secs_f64());
        }
        Ok(uid)
    }










    fn scan_nodes(&self, type_name: &str, filter: std::collections::HashMap<String, Value>, sort: std::collections::HashMap<String, Value>, first: Option<usize>, after: Option<String>, uniques: &[String], near_vector: Option<Vec<f64>>) -> Vec<u64> {
        // println!("Scan: scan_nodes called for {}. Filter: {:?}, Sort: {:?}, NearVector: {:?}", type_name, filter, sort, near_vector.is_some());

         let mut uids = Vec::new();
         let mut filter_im = indexmap::IndexMap::new();
         for (k, v) in &filter {
            filter_im.insert(async_graphql::Name::new(k), v.clone());
        }

        // Strategy:
        // 1. If `near_vector` is present, it DRIVES the scan (Simulated Vector Index Scan).
        // 2. If `uniques` provided (Identity Map), use that.
        // 3. Else Full Scan.

        // Detect Text Search Predicates
        let mut text_search: Option<(String, String, String, bool)> = None; // (field, strategy, query, require_all)
        for (field, val) in &filter {
             if let Value::Object(obj) = val {
                 if let Some(q) = obj.get("allofterms") {
                     if let Value::String(s) = q {
                         // Use "term" strategy for allofterms, require_all=true
                         text_search = Some((field.clone(), "term".to_string(), s.clone(), true));
                         break; 
                     }
                 }
                 if let Some(q) = obj.get("anyofterms") {
                      if let Value::String(s) = q {
                         // Use "term" strategy for anyofterms, require_all=false
                         text_search = Some((field.clone(), "term".to_string(), s.clone(), false));
                         break;
                     }
                 }
                 if let Some(q) = obj.get("alloftext") {
                    if let Value::String(s) = q {
                        text_search = Some((field.clone(), "fulltext".to_string(), s.clone(), true));
                        break; 
                    }
                }
                if let Some(q) = obj.get("anyoftext") {
                     if let Value::String(s) = q {
                        text_search = Some((field.clone(), "fulltext".to_string(), s.clone(), false));
                        break;
                    }
                }
             }
        }

        if let Some(ref vec) = near_vector {
             // Case 1: Hybrid Search (Vector + Text) or Vector Search
             let k = first.unwrap_or(50) * 4; 
             
             let search_results = if let Some((field, _strat, query, require_all)) = text_search {
                 // Hybrid
                 self.search_hybrid(&query, &field, vec, k, require_all)
             } else {
                 // Pure Vector
                 self.search_vectors(vec, k)
             };

             for (uid, _dist) in search_results {
                 // Verify Type & Apply Filters
                 if self.node_exists(type_name, uid) { 
                      if let Some(stored_type) = self.get_node_type(uid) {
                          if stored_type == type_name {
                              if filter.is_empty() || self.check_filter_recursive(uid, &filter_im) {
                                  uids.push(uid);
                              }
                          }
                      }
                 }
             }
             // Results are already sorted by Score/Distance (ASC/DESC depending on impl)
             
        } else if let Some((field, strat, query, require_all)) = text_search {
             // Case 2: Pure Text Search (BM25)
             let k = first.unwrap_or(50) * 4;
             let results = self.search_text_bm25(&query, &field, &strat, k, require_all);
             
             for (uid, _score) in results {
                 if self.node_exists(type_name, uid) {
                      if let Some(stored_type) = self.get_node_type(uid) {
                          if stored_type == type_name {
                              if self.check_filter_recursive(uid, &filter_im) {
                                  uids.push(uid);
                              }
                          }
                      }
                 }
             }
             // BM25 results are sorted by score (DESC) usually.
             
        } else if let Some(candidates) = self.get_candidates(type_name, &filter, uniques) {
             // Candidate Set Optimization
             // Try parallelize if set is large enough?
             // For now, always parallelize as user requested it explicitly.
             use rayon::prelude::*;
            
             // Collect into Vec for Rayon (HashSet is not parallel iterator by default usually, or needs explicit support)
             // Rayon supports HashSet parallel iter if we import it.
             // But strict order for vector collection?
            
             let mut matched_uids: Vec<u64> = candidates.par_iter()
                 .filter(|uid| {
                      let matches_filter = if filter.is_empty() {
                          true
                      } else {
                          self.check_filter_recursive(**uid, &filter_im)
                      };
                      matches_filter
                 })
                 .cloned()
                 .collect();
            
             uids.append(&mut matched_uids);

             // Candidate set has no order guarantees. We MUST sort if pagination/sorting is active.
             // If no explicit sort, we should probably sort by UID for consistency?
             // Existing logic for full scan yields sorted by key (UID).
             if sort.is_empty() {
                  uids.sort(); 
                  // Handle pagination below
             }
        } else {
             // FULL SCAN FALLBACK
            let prefix = Codec::encode_type_prefix(type_name);
            let needs_sorting = !sort.is_empty();
            
            // If we have a candidate set, we iterate THAT instead of the DB prefix scan
            // UNLESS we need to sort, in which case we still might generally fetch all, but we can filter the candidate set.
            
            let start_key = if !needs_sorting {
                 if let Some(cursor) = after.clone() {
                     let uid = cursor.parse::<u64>().unwrap_or(0);
                     if uid == u64::MAX { return vec![]; }
                     Codec::encode_type_index_key(type_name, uid + 1)
                 } else {
                     prefix.clone()
                 }
            } else {
                prefix.clone()
            };

            use std::ops::Bound;
            if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                 let iter = main_ks.range((Bound::Included(start_key.clone()), Bound::Unbounded));
                 if crate::debug_logging() {
                     println!("Scan: Starting Full Scan for type: {}. Prefix/StartKey: {:?}", type_name, start_key);
                 }
                 for guard in iter {
                    if let Ok(key) = guard.key() {
                        // scanned_count += 1;
                        // if scanned_count <= 10 {
                        //    println!("Scan: Seeing Key: {:?}", key);
                        // }
                        if !key.starts_with(&prefix) { 
                            // if scanned_count <= 10 { println!("Scan: Key {:?} does not match prefix {:?}", key, prefix); }
                            break; 
                        }
                        if key.len() >= 8 {
                            let uid = BigEndian::read_u64(&key[key.len()-8..]);
                            
                             if filter.is_empty() || self.check_filter_recursive(uid, &filter_im) {
                                 uids.push(uid);
                                 // If NO sorting, we can break early
                                 if !needs_sorting && near_vector.is_none() {
                                     if let Some(limit) = first {
                                         if uids.len() >= limit { break; }
                                     }
                                 }
                             }
                        }
                    }
                 }
                 if crate::debug_logging() {
                     println!("Scan: Completed. Found {} uids", uids.len());
                 }
            }
        }

        // Apply Sorting (if explicit sort OR if implicit ID sort required)
        if !sort.is_empty() {
            // In-Memory Sort
            if let Some((field, direction)) = sort.iter().next() {
                let asc = match direction {
                    Value::String(s) => s == "ASC",
                     _ => true
                };
                
                uids.sort_by(|a, b| {
                    let val_a = self.resolve(*a, field);
                    let val_b = self.resolve(*b, field);
                    
                    let cmp = match (val_a, val_b) {
                        (Some(Value::Number(na)), Some(Value::Number(nb))) => {
                             na.as_f64().partial_cmp(&nb.as_f64()).unwrap_or(std::cmp::Ordering::Equal)
                        },
                        (Some(Value::String(sa)), Some(Value::String(sb))) => {
                            sa.cmp(&sb)
                        },
                         (None, Some(_)) => std::cmp::Ordering::Less,
                         (Some(_), None) => std::cmp::Ordering::Greater,
                         _ => std::cmp::Ordering::Equal
                    };
                    
                    if asc { cmp } else { cmp.reverse() }
                });
            }
        } else if near_vector.is_none() {
             // If NOT vector search, and NO explicit sort, we sort by UID usually?
             // Actually Full Scan builds `uids` in order (if not filtering via candidates).
             // But Candidate set is unordered. 
             // To be safe, we sort if came from candidates.
             if self.get_candidates(type_name, &filter, uniques).is_some() {
                 uids.sort();
             }
        }

        // Pagination
        // Vector search: `after` cursor is tricky (cursor needs to be offset or UID-based?)
        // Standard GraphQL: Cursor is usually opaque. 
        // For Vector Search, we usually don't support `after` efficiently without keeping state.
        // We will support `after` simply by filtering `uids` if `after` is a UID.
        // BUT if `after` is used with Vector Search, it implies "Item X was the last one".
        // If sorting by Distance, checking "UID > X" is meaningless.
        // We need to find X in the list and skip past it.
        
        if let Some(cursor_uid_str) = after {
             if let Ok(cursor_uid) = cursor_uid_str.parse::<u64>() {
                 if let Some(pos) = uids.iter().position(|u| *u == cursor_uid) {
                     uids = uids.into_iter().skip(pos + 1).collect();
                 }
             }
        }
        
        if let Some(limit) = first {
            uids.truncate(limit);
        }

        uids
    }

    fn update_node(&self, type_name: &str, uid: u64, mut fields: std::collections::HashMap<String, Value>, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>, vector_config: Option<&crate::engine::resolver::VectorConfig>) -> Result<(), String> {
        let op_start = std::time::Instant::now();
        
        // Automatic Embedding Generation (on Update)
        if let Some(config) = vector_config {
             // If source field is being updated, and embedding is NOT manually provided, regenerate it.
             if fields.contains_key(&config.source) && !fields.contains_key(&config.field) {
                 if let Some(Value::String(text)) = fields.get(&config.source) {
                     match self.storage.embedding_model.lock().unwrap().embed(vec![text.clone()], None) {
                        Ok(embeddings) => {
                             if let Some(first) = embeddings.first() {
                                 let json_values: Vec<Value> = first.iter().map(|f| Value::Number(async_graphql::Number::from_f64((*f).into()).unwrap_or(async_graphql::Number::from(0))))
                                     .collect();
                                 fields.insert(config.field.clone(), Value::List(json_values));
                             }
                        },
                        Err(e) => eprintln!("Failed to generate embedding (update): {}", e)
                     }
                 }
             }

             // HNSW Update
             if let Some(val) = fields.get(&config.field) {
                if let Value::List(list) = val {
                    let vec_data: Vec<f64> = list.iter().filter_map(|v| match v {
                        Value::Number(n) => n.as_f64(),
                        _ => None
                    }).collect();
                     if !vec_data.is_empty() {
                        let storage = self.storage.clone();
                         tokio::task::spawn_blocking(move || {
                            if let Err(e) = storage.put_vector(uid, vec_data) {
                                eprintln!("Background Vector Update Error (UID {}): {}", uid, e);
                            }
                        });
                    }
                }
            }
        }

        let payload: std::collections::HashMap<String, serde_json::Value> = fields.iter()
            .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(serde_json::Value::Null)))
            .collect();
            
        let result = self.update_node_internal(type_name, uid, payload, uniques, inverses, search_fields, crate::realtime::bus::MutationSource::Local, None);
        
        let elapsed = op_start.elapsed();
        if crate::debug_logging() && elapsed.as_secs() >= 1 {
            println!("SLOW: update_node for {} uid={} took {:.2}s", type_name, uid, elapsed.as_secs_f64());
        }
        result
    }

    fn delete_node(&self, type_name: &str, uid: u64, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> {
        self.delete_node_internal(type_name, uid, uniques, inverses, search_fields, crate::realtime::bus::MutationSource::Local, None)
    }

    fn node_exists(&self, type_name: &str, uid: u64) -> bool {
        let type_key = Codec::encode_type_index_key(type_name, uid);
        self.storage.contains_key(&self.db_name, &type_key).unwrap_or(false)
    }

    fn get_node_type(&self, uid: u64) -> Option<String> {
        let type_key = Codec::encode_data_key(uid, "_type");
        if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &type_key) {
            if let Ok(Value::String(s)) = serde_json::from_slice(&bytes) {
                return Some(s);
            }
        }
        None
    }

    fn subscribe_events(&self) -> EventBus {
        self.bus.clone()
    }

    fn flush(&self) -> Result<(), String> {
        // Call Storage::flush() which rotates memtables, persists fingerprints, and syncs to disk
        self.storage.flush()
            .map_err(|e| e.to_string())
    }

    fn compact(&self) -> Result<u64, String> {
        self.storage.compact()
            .map_err(|e| e.to_string())
    }

    fn needs_compaction(&self) -> bool {
        self.storage.needs_compaction()
    }
}
