use crate::engine::resolver::{RequestCache, Resolver};
use crate::storage::backend::Storage;
use crate::storage::codec::Codec;
use crate::storage::timestamp::Timestamp;
use async_graphql::Value;
use byteorder::{BigEndian, ByteOrder};
use std::sync::Arc;

use crate::realtime::bus::{EventBus, MutationEvent, MutationType};

#[derive(Clone)]
pub struct SqliteResolver {
    pub storage: Arc<Storage>,
    pub bus: EventBus,
    pub db_name: String,
}

impl SqliteResolver {
    fn preload_objects_for_uids(&self, uids: &[u64], cache: &RequestCache) {
        if uids.len() < 8 {
            return;
        }

        let missing: std::collections::HashSet<u64> = uids
            .iter()
            .copied()
            .filter(|uid| cache.get_loaded_object(*uid).is_none())
            .collect();

        if missing.len() < 8 {
            return;
        }

        let mut sorted_uids: Vec<u64> = missing.iter().copied().collect();
        sorted_uids.sort_unstable();
        let Some(min_uid) = sorted_uids.first().copied() else {
            return;
        };
        let Some(max_uid) = sorted_uids.last().copied() else {
            return;
        };

        let lower = Codec::encode_data_prefix(min_uid);
        let upper = crate::storage::sqlite_backend::compute_prefix_upper_bound(
            &Codec::encode_data_prefix(max_uid),
        )
        .unwrap_or_else(|| vec![0x02]);

        let mut grouped: std::collections::HashMap<u64, std::collections::HashMap<String, Value>> =
            std::collections::HashMap::new();
        if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
            for (key, value) in main_ks.range(&lower, &upper) {
                let Some(uid) = Codec::decode_data_uid(&key) else {
                    continue;
                };
                if !missing.contains(&uid) || key.len() <= 9 {
                    continue;
                }
                let Ok(field_name) = std::str::from_utf8(&key[9..]) else {
                    continue;
                };
                if let Some(parsed) = Self::parse_resolved_value(&value) {
                    grouped
                        .entry(uid)
                        .or_default()
                        .insert(field_name.to_string(), parsed);
                }
            }
        }

        for uid in sorted_uids {
            cache.insert_loaded_object(uid, grouped.remove(&uid).unwrap_or_default());
        }
    }

    pub(crate) fn load_object_fields(&self, uid: u64) -> std::collections::HashMap<String, Value> {
        let prefix = Codec::encode_data_prefix(uid);
        let mut fields = std::collections::HashMap::new();
        if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
            for (key, value) in main_ks.prefix(&prefix) {
                if !key.starts_with(&prefix) || key.len() <= prefix.len() {
                    break;
                }
                let Ok(field_name) = std::str::from_utf8(&key[prefix.len()..]) else {
                    continue;
                };
                if let Some(parsed) = Self::parse_resolved_value(&value) {
                    fields.insert(field_name.to_string(), parsed);
                }
            }
        }
        fields
    }

    fn parse_resolved_value(bytes: &[u8]) -> Option<Value> {
        if bytes.len() > 16 {
            serde_json::from_slice(&bytes[16..]).ok()
        } else {
            serde_json::from_slice(bytes).ok()
        }
    }

    fn value_to_uid(value: &Value) -> Option<u64> {
        match value {
            Value::String(s) => s.parse::<u64>().ok(),
            Value::Number(n) => n.as_u64(),
            Value::Object(map) => map
                .get("uid")
                .or(map.get("id"))
                .and_then(Self::value_to_uid),
            _ => None,
        }
    }

    fn parse_uid_list(value: Value) -> Vec<u64> {
        match value {
            Value::List(list) => list
                .into_iter()
                .filter_map(|item| Self::value_to_uid(&item))
                .collect(),
            other => Self::value_to_uid(&other).into_iter().collect(),
        }
    }

    pub(crate) fn load_related_uids(&self, parent_uid: u64, field_name: &str) -> Vec<u64> {
        let key = Codec::encode_data_key(parent_uid, field_name);
        let mut uids = self
            .storage
            .get(&self.db_name, &key)
            .ok()
            .flatten()
            .and_then(|bytes| Self::parse_resolved_value(&bytes))
            .map(Self::parse_uid_list)
            .unwrap_or_default();

        let edge_prefix = Codec::encode_edge_prefix(parent_uid, field_name);
        if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
            for (key, _val) in main_ks.prefix(&edge_prefix) {
                if !key.starts_with(&edge_prefix) {
                    break;
                }
                if let Some(source_uid) = Codec::decode_edge_source_uid(&key) {
                    uids.push(source_uid);
                }
            }
        }

        uids
    }

    pub(crate) fn load_resolved_value(&self, uid: u64, field_name: &str) -> Option<Value> {
        if field_name == "id" {
            return Some(Value::String(uid.to_string()));
        }

        let key = Codec::encode_data_key(uid, field_name);
        match self.storage.get(&self.db_name, &key) {
            Ok(Some(bytes)) => Self::parse_resolved_value(&bytes),
            _ => {
                let edge_uids = self.load_related_uids(uid, field_name);
                if edge_uids.is_empty() {
                    None
                } else {
                    Some(Value::List(
                        edge_uids
                            .into_iter()
                            .map(|u| Value::String(u.to_string()))
                            .collect(),
                    ))
                }
            }
        }
    }

    fn resolve_cached(
        &self,
        uid: u64,
        field_name: &str,
        cache: Option<&RequestCache>,
    ) -> Option<Value> {
        if let Some(cache) = cache {
            if let Some(value) = cache.get_resolved(uid, field_name) {
                return value;
            }

            if cache.get_loaded_object(uid).is_none() {
                let loaded_fields = self.load_object_fields(uid);
                cache.insert_loaded_object(uid, loaded_fields);
            }

            if let Some(value) = cache.get_resolved(uid, field_name) {
                return value;
            }
        }

        let value = self.load_resolved_value(uid, field_name);
        if let Some(cache) = cache {
            cache.insert_resolved(uid, field_name, value.clone());
        }
        value
    }



    fn encode_sortable_f64(value: f64) -> [u8; 8] {
        let bits = value.to_bits();
        let sortable = if bits & (1 << 63) != 0 {
            !bits
        } else {
            bits ^ (1 << 63)
        };
        sortable.to_be_bytes()
    }

    fn encode_order_index_value(value: &serde_json::Value, descending: bool) -> Option<Vec<u8>> {
        let mut encoded = match value {
            serde_json::Value::String(s) => s.as_bytes().to_vec(),
            serde_json::Value::Number(n) => {
                let numeric = if let Some(i) = n.as_i64() {
                    i as f64
                } else if let Some(u) = n.as_u64() {
                    u as f64
                } else {
                    n.as_f64()?
                };
                Self::encode_sortable_f64(numeric).to_vec()
            }
            serde_json::Value::Bool(b) => vec![u8::from(*b)],
            _ => return None,
        };

        if descending {
            for byte in &mut encoded {
                *byte = !*byte;
            }
        }

        Some(encoded)
    }

    fn write_order_index(
        &self,
        type_name: &str,
        uid: u64,
        field: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let Some(asc_value) = Self::encode_order_index_value(value, false) else {
            return Ok(());
        };
        let desc_value = Self::encode_order_index_value(value, true).expect("descending encoding");

        let asc_key = Codec::encode_order_index_key(type_name, field, false, &asc_value, uid);
        let desc_key = Codec::encode_order_index_key(type_name, field, true, &desc_value, uid);
        self.storage
            .insert(&self.db_name, &asc_key, &[])
            .map_err(|e| e.to_string())?;
        self.storage
            .insert(&self.db_name, &desc_key, &[])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn remove_order_index(
        &self,
        type_name: &str,
        uid: u64,
        field: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let Some(asc_value) = Self::encode_order_index_value(value, false) else {
            return Ok(());
        };
        let desc_value = Self::encode_order_index_value(value, true).expect("descending encoding");

        let asc_key = Codec::encode_order_index_key(type_name, field, false, &asc_value, uid);
        let desc_key = Codec::encode_order_index_key(type_name, field, true, &desc_value, uid);
        self.storage
            .remove(&self.db_name, &asc_key)
            .map_err(|e| e.to_string())?;
        self.storage
            .remove(&self.db_name, &desc_key)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(crate) fn rebuild_order_index_for_field(&self, type_name: &str, field: &str) -> Result<(), String> {
        let (main_ks, _) = self
            .storage
            .get_database(&self.db_name)
            .ok_or_else(|| format!("Database not found: {}", self.db_name))?;

        let type_prefix = Codec::encode_type_prefix(type_name);
        let upper = crate::storage::sqlite_backend::compute_prefix_upper_bound(&type_prefix)
            .expect("valid prefix bounds");

        let mut pending = Vec::new();
        for (key, _val) in main_ks.range(&type_prefix, &upper) {
            if !key.starts_with(&type_prefix) || key.len() < 8 {
                break;
            }
            let uid = BigEndian::read_u64(&key[key.len() - 8..]);
            let data_key = Codec::encode_data_key(uid, field);
            if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                let payload = if bytes.len() > 16 {
                    &bytes[16..]
                } else {
                    &bytes
                };
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(payload) {
                    if let Some(encoded) = Self::encode_order_index_value(&value, false) {
                        pending.push(Codec::encode_order_index_key(
                            type_name, field, false, &encoded, uid,
                        ));
                    }
                    if let Some(encoded) = Self::encode_order_index_value(&value, true) {
                        pending.push(Codec::encode_order_index_key(
                            type_name, field, true, &encoded, uid,
                        ));
                    }
                }
            }
        }

        if pending.is_empty() {
            return Ok(());
        }

        let backend = self
            .storage
            .backends
            .get(&self.db_name)
            .ok_or_else(|| format!("Backend not found: {}", self.db_name))?
            .clone();
        backend
            .write_batch(|conn| {
                for key in &pending {
                    main_ks.batch_insert_on_conn(conn, key, &[])?;
                }
                Ok(())
            })
            .map_err(|e| e.to_string())
    }

    pub fn new(storage: Arc<Storage>, db_name: &str) -> Self {
        Self {
            storage,
            db_name: db_name.to_string(),
            bus: EventBus::new(),
        }
    }

    /// Create a SqliteResolver with a shared EventBus.
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

    pub fn compute_fingerprint(
        &self,
    ) -> anyhow::Result<crate::sync::reconciliation::RangeFingerprint> {
        // Full range fingerprint
        let start = crate::storage::timestamp::Timestamp::new(0, 0, 0);
        let end = crate::storage::timestamp::Timestamp::new(u64::MAX, u16::MAX, u64::MAX);
        crate::sync::reconciliation::compute_fingerprint(&self.storage, &self.db_name, &start, &end)
    }

    pub fn compute_fingerprint_range(
        &self,
        start: &Timestamp,
        end: &Timestamp,
    ) -> anyhow::Result<crate::sync::reconciliation::RangeFingerprint> {
        crate::sync::reconciliation::compute_fingerprint(&self.storage, &self.db_name, start, end)
    }

    /// Convert a serde_json::Value to a rusqlite::types::Value for SQL pushdown.
    /// json_extract() returns typed values: TEXT for strings, INTEGER for ints,
    /// REAL for floats, so the comparison parameter must match.
    pub(crate) fn json_to_sqlite_value(val: &Value) -> rusqlite::types::Value {
        match val {
            Value::String(s) => rusqlite::types::Value::Text(s.clone()),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    rusqlite::types::Value::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    rusqlite::types::Value::Real(f)
                } else {
                    rusqlite::types::Value::Null
                }
            }
            Value::Boolean(b) => rusqlite::types::Value::Integer(if *b { 1 } else { 0 }),
            Value::Null => rusqlite::types::Value::Null,
            Value::Enum(e) => rusqlite::types::Value::Text(e.to_string()),
            // For complex types (Object, List), fall back to text representation
            other => rusqlite::types::Value::Text(serde_json::to_string(other).unwrap_or_default()),
        }
    }

    pub fn get_history_range(
        &self,
        start: &Timestamp,
        end: &Timestamp,
    ) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.storage
            .get_history_range(&self.db_name, Some(start), Some(end))
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
            println!(
                "Sync: Partitioned batch: {} type items, {} other items",
                type_items.len(),
                other_items.len()
            );
        }

        // Initialize Event Buffer
        // Map: UID -> (Type, MutationType, Payload, MinTimestamp)
        // We use MinTimestamp because events are usually batched from the same "transaction" or we want the earliest causal time?
        // Actually, for LWW, if we have multiple updates, we want the LATEST timestamp.
        let mut pending_emissions: std::collections::HashMap<
            u64,
            (
                String,
                crate::realtime::bus::MutationType,
                std::collections::HashMap<String, serde_json::Value>,
                crate::storage::timestamp::Timestamp,
            ),
        > = std::collections::HashMap::new();

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
                            let data_key =
                                crate::storage::codec::Codec::encode_data_key(uid, "_type");
                            if let Ok(Some(current_bytes)) =
                                self.storage.get(&self.db_name, &data_key)
                            {
                                if let Ok(serde_json::Value::String(current_type)) =
                                    serde_json::from_slice(&current_bytes)
                                {
                                    let type_idx_key =
                                        crate::storage::codec::Codec::encode_type_index_key(
                                            &current_type,
                                            uid,
                                        );
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

                        self.storage
                            .delete_with_lww(&self.db_name, uid, &pred, &ts)?;

                        if should_emit {
                            // Emit Event Immediately for Delete (Atomic enough usually)
                            let payload =
                                if mutation_type == crate::realtime::bus::MutationType::Delete {
                                    None
                                } else {
                                    Some(std::collections::HashMap::from([(
                                        pred,
                                        serde_json::Value::Null,
                                    )]))
                                };

                            let event = crate::realtime::bus::MutationEvent {
                                type_name: event_type_name,
                                uid,
                                mutation_type,
                                source: crate::realtime::bus::MutationSource::Remote,
                                payload,
                                metadata: None,
                                timestamp: Some(ts),
                                node_id: self.storage.node_id,
                            };
                            let _ = self.bus.publish(event);
                        }
                    } else {
                        self.storage
                            .put_with_lww(&self.db_name, uid, &pred, &v, &ts)?;

                        // 1. Buffer Event Emission
                        if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&v) {
                            // 2. Index Maintenance (Type Index)
                            let mut resolved_type_name = "Unknown".to_string();

                            if pred == "_type" {
                                if let serde_json::Value::String(ref type_name) = json_val {
                                    let type_idx_key = Codec::encode_type_index_key(type_name, uid);
                                    let _res =
                                        self.storage.insert(&self.db_name, &type_idx_key, &[]);
                                    if crate::debug_logging() {
                                        println!(
                                            "Sync: Insert Type Index Key: {:?}, Result: {:?}",
                                            type_idx_key, _res
                                        );
                                    }
                                    resolved_type_name = type_name.clone();
                                } else {
                                    println!("Sync: ERROR: _type predicate found but value is not a String! Value: {:?}", json_val);
                                }
                            } else {
                                // Try to lookup type from storage
                                let type_key =
                                    crate::storage::codec::Codec::encode_data_key(uid, "_type");
                                if let Ok(Some(type_bytes)) =
                                    self.storage.get(&self.db_name, &type_key)
                                {
                                    if let Ok(serde_json::Value::String(t)) =
                                        serde_json::from_slice(&type_bytes)
                                    {
                                        resolved_type_name = t;
                                    }
                                }
                            }

                            // Add to Buffer
                            let entry = pending_emissions.entry(uid).or_insert_with(|| {
                                (
                                    "Unknown".to_string(),
                                    crate::realtime::bus::MutationType::Update,
                                    std::collections::HashMap::new(),
                                    ts,
                                )
                            });

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
                }
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
                node_id: self.storage.node_id,
            };
            let _ = self.bus.publish(event);
        }

        Ok(())
    }

    pub fn try_restore_quarantine(
        &self,
        valid_predicates: &std::collections::HashSet<String>,
    ) -> anyhow::Result<usize> {
        let items = self.storage.scan_quarantine()?;
        let mut restored = 0;
        for (k, v) in items {
            if let Ok((uid, pred)) = Codec::decode_quarantine_key(&k) {
                if valid_predicates.contains(&pred) {
                    if let Ok((ts, data)) = Codec::decode_quarantine_value(&v) {
                        // Restore to LATEST/HISTORY using LWW
                        self.storage
                            .put_with_lww(&self.db_name, uid, &pred, &data, &ts)?;
                        // Remove from Quarantine
                        self.storage.delete_quarantine(&k)?;
                        restored += 1;
                    }
                }
            }
        }
        Ok(restored)
    }

    fn link_inverse(
        &self,
        target_uid: u64,
        inverse_field: &str,
        is_list: bool,
        self_uid: u64,
        timestamp: &Timestamp,
    ) -> Result<(), String> {
        if is_list {
            // O(1) write: just insert a single edge key
            let edge_key = Codec::encode_edge_key(target_uid, inverse_field, self_uid);
            let reverse_edge_key =
                Codec::encode_reverse_edge_key(self_uid, target_uid, inverse_field);
            self.storage
                .insert(&self.db_name, &edge_key, &[])
                .map_err(|e| e.to_string())?;
            self.storage
                .insert(&self.db_name, &reverse_edge_key, &[])
                .map_err(|e| e.to_string())?;
        } else {
            // 1:1 or N:1 - Overwrite (unchanged, single value is fast)
            let val = Value::String(self_uid.to_string());
            let val_bytes = serde_json::to_vec(&val).map_err(|e| e.to_string())?;
            let mut val_buf = Vec::with_capacity(16 + val_bytes.len());
            val_buf.extend_from_slice(&timestamp.to_bytes());
            val_buf.extend_from_slice(&val_bytes);
            self.storage
                .put_with_lww(
                    &self.db_name,
                    target_uid,
                    inverse_field,
                    &val_buf,
                    timestamp,
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn unlink_inverse(
        &self,
        target_uid: u64,
        inverse_field: &str,
        is_list: bool,
        self_uid: u64,
        timestamp: &Timestamp,
    ) -> Result<(), String> {
        if is_list {
            // O(1) delete: just remove the single edge key
            let edge_key = Codec::encode_edge_key(target_uid, inverse_field, self_uid);
            let reverse_edge_key =
                Codec::encode_reverse_edge_key(self_uid, target_uid, inverse_field);
            self.storage
                .delete_key(&self.db_name, &edge_key)
                .map_err(|e| e.to_string())?;
            self.storage
                .delete_key(&self.db_name, &reverse_edge_key)
                .map_err(|e| e.to_string())?;
        } else {
            // 1:1 - If the current value IS self, remove it
            let key = Codec::encode_data_key(target_uid, inverse_field);
            if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &key) {
                let payload = if bytes.len() > 16 {
                    &bytes[16..]
                } else {
                    &bytes
                };
                if let Ok(val) = serde_json::from_slice::<Value>(payload) {
                    let matches = match val {
                        Value::String(s) => s == self_uid.to_string(),
                        Value::Number(n) => n.as_u64() == Some(self_uid),
                        _ => false,
                    };
                    if matches {
                        self.storage
                            .delete_with_lww(&self.db_name, target_uid, inverse_field, timestamp)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        Ok(())
    }

    fn write_term_index(
        &self,
        uid: u64,
        field: &str,
        text: &str,
        strategy: &str,
    ) -> Result<(), String> {
        let index_field = if strategy == "term" {
            field.to_string()
        } else {
            format!("{}.{}", field, strategy)
        };
        let uid_i64 = uid as i64;
        let table = if strategy == "term" {
            "fts_term_data"
        } else {
            "fts_data"
        };
        let sql_del = format!("DELETE FROM {} WHERE uid = ?1 AND field = ?2", table);
        let sql_ins = format!(
            "INSERT INTO {}(uid, field, text_content) VALUES (?1, ?2, ?3)",
            table
        );

        let backend = self.storage.backends.get(&self.db_name).unwrap().clone();
        backend
            .with_writer(|conn| {
                conn.execute(&sql_del, rusqlite::params![uid_i64, index_field])?;
                conn.execute(&sql_ins, rusqlite::params![uid_i64, index_field, text])?;
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn remove_term_index(
        &self,
        uid: u64,
        field: &str,
        _text: &str,
        strategy: &str,
    ) -> Result<(), String> {
        let index_field = if strategy == "term" {
            field.to_string()
        } else {
            format!("{}.{}", field, strategy)
        };
        let uid_i64 = uid as i64;
        let table = if strategy == "term" {
            "fts_term_data"
        } else {
            "fts_data"
        };
        let sql_del = format!("DELETE FROM {} WHERE uid = ?1 AND field = ?2", table);
        let backend = self.storage.backends.get(&self.db_name).unwrap().clone();
        backend
            .with_writer(|conn| {
                conn.execute(&sql_del, rusqlite::params![uid_i64, index_field])?;
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // Ranked Search (BM25)
    pub fn search_text_bm25(
        &self,
        query: &str,
        field: &str,
        strategy: &str,
        k: usize,
        require_all: bool,
    ) -> Vec<(u64, f64)> {
        let index_field = if strategy == "term" {
            field.to_string()
        } else {
            format!("{}.{}", field, strategy)
        };
        let safe_query: String = query
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();
        let terms: Vec<&str> = safe_query.split_whitespace().collect();
        if terms.is_empty() {
            return vec![];
        }

        let fts_query = if require_all {
            terms.join(" AND ")
        } else {
            terms.join(" OR ")
        };

        let backend = self.storage.backends.get(&self.db_name).unwrap().clone();
        let conn = match backend.get_reader() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let table = if strategy == "term" {
            "fts_term_data"
        } else {
            "fts_data"
        };
        let sql = format!("SELECT uid, bm25({}) FROM {} WHERE text_content MATCH ?1 AND field = ?2 ORDER BY rank LIMIT ?3", table, table);
        let out = (|| -> Result<Vec<(u64, f64)>, rusqlite::Error> {
            // Note: ranking in sqlite FTS is negative (most negative is best). So we negate it to positive.
            let mut stmt = conn.prepare(&sql)?;

            let rows =
                stmt.query_map(rusqlite::params![fts_query, index_field, k as i64], |row| {
                    let uid: i64 = row.get(0)?;
                    let score: f64 = row.get(1)?;
                    Ok((uid as u64, -score))
                })?;

            let mut out = Vec::new();
            for r in rows {
                if let Ok(val) = r {
                    out.push(val);
                }
            }
            Ok(out)
        })()
        .unwrap_or_default();

        backend.return_reader(conn);
        out
    }

    // Hybrid Search (RRF)
    pub fn search_hybrid(
        &self,
        text_query: &str,
        field: &str,
        vector: &[f64],
        k: usize,
        require_all: bool,
    ) -> Vec<(u64, f64)> {
        let index_field = format!("{}.fulltext", field); // strategy is assumed 'fulltext'
        let safe_query: String = text_query
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();
        let terms: Vec<&str> = safe_query.split_whitespace().collect();
        if terms.is_empty() {
            return vec![];
        }

        let fts_query = if require_all {
            terms.join(" AND ")
        } else {
            terms.join(" OR ")
        };

        let vec_f32: Vec<f32> = vector.iter().map(|v| *v as f32).collect();
        let vec_bytes =
            unsafe { std::slice::from_raw_parts(vec_f32.as_ptr() as *const u8, vec_f32.len() * 4) };

        let backend = self.storage.backends.get(&self.db_name).unwrap().clone();
        let conn = match backend.get_reader() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        // CTE RRF using FULL OUTER JOIN
        // Note: FTS5 rank returns negative score, sqlite-vec returns positive distance, lower is better for both.
        // We use ROW_NUMBER() over ranking. FTS matches use `rank`, vec matches use `distance`.
        let sql = "
        WITH text_search AS (
            SELECT uid, 
                   ROW_NUMBER() OVER (ORDER BY rank) as position
            FROM fts_data WHERE text_content MATCH ?1 AND field = ?2 LIMIT 100
        ),
        vec_search AS (
            SELECT uid, 
                   ROW_NUMBER() OVER (ORDER BY distance) as position
            FROM vec_data WHERE embedding MATCH ?3 AND k = 100
        )
        SELECT COALESCE(t.uid, v.uid) as id,
               (COALESCE(1.0 / (60.0 + t.position), 0.0) +
                COALESCE(1.0 / (60.0 + v.position), 0.0)) as rrf_score
        FROM text_search t
        FULL OUTER JOIN vec_search v ON t.uid = v.uid
        ORDER BY rrf_score DESC
        LIMIT ?4;
        ";

        let out = (|| -> Result<Vec<(u64, f64)>, rusqlite::Error> {
            let mut stmt = conn.prepare(sql)?;

            let rows = stmt.query_map(
                rusqlite::params![fts_query, index_field, vec_bytes, k as i64],
                |row| {
                    let uid: i64 = row.get(0)?;
                    let score: f64 = row.get(1)?;
                    Ok((uid as u64, score))
                },
            )?;

            let mut out = Vec::new();
            for r in rows {
                if let Ok(val) = r {
                    out.push(val);
                }
            }
            Ok(out)
        })();

        if let Err(e) = &out {
            println!("search_hybrid error: {:?}", e);
        }

        let out = out.unwrap_or_default();

        backend.return_reader(conn);
        out
    }

    pub(crate) fn check_condition(&self, stored_val: &Option<Value>, condition: &Value) -> bool {
        // If condition is a Map, it's a Filter Object (eq, gt, etc.)
        // If condition is a Scalar, it's an implicit Equality check (Backward Compat / scalar input)

        match condition {
            Value::Object(map) => {
                for (op, target) in map {
                    match op.as_str() {
                        "eq" => {
                            if let Some(sv) = stored_val {
                                if sv != target {
                                    return false;
                                }
                            } else {
                                if target != &Value::Null {
                                    return false;
                                }
                            }
                        }
                        "gt" => {
                            // Comparison Logic (only if types match or are compatible)
                            match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) {
                                        if !(sf > tf) {
                                            return false;
                                        }
                                    }
                                }
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    // Try parsing as i64 (Int64 parity)
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>())
                                    {
                                        if !(si > ti) {
                                            return false;
                                        }
                                    } else if ss <= ts {
                                        return false;
                                    } // Lexical fallback
                                }
                                _ => {}
                            }
                        }
                        "lt" => match (stored_val, target) {
                            (Some(Value::Number(sn)), Value::Number(tn)) => {
                                if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) {
                                    if !(sf < tf) {
                                        return false;
                                    }
                                }
                            }
                            (Some(Value::String(ss)), Value::String(ts)) => {
                                if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                    if !(si < ti) {
                                        return false;
                                    }
                                } else if ss >= ts {
                                    return false;
                                }
                            }
                            _ => {}
                        },
                        "ge" => match (stored_val, target) {
                            (Some(Value::Number(sn)), Value::Number(tn)) => {
                                if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) {
                                    if !(sf >= tf) {
                                        return false;
                                    }
                                }
                            }
                            (Some(Value::String(ss)), Value::String(ts)) => {
                                if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                    if !(si >= ti) {
                                        return false;
                                    }
                                } else if ss < ts {
                                    return false;
                                }
                            }
                            _ => {}
                        },
                        "le" => match (stored_val, target) {
                            (Some(Value::Number(sn)), Value::Number(tn)) => {
                                if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) {
                                    if !(sf <= tf) {
                                        return false;
                                    }
                                }
                            }
                            (Some(Value::String(ss)), Value::String(ts)) => {
                                if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                    if !(si <= ti) {
                                        return false;
                                    }
                                } else if ss > ts {
                                    return false;
                                }
                            }
                            _ => {}
                        },
                        "contains" => {
                            if let (Some(Value::String(ss)), Value::String(ts)) =
                                (stored_val, target)
                            {
                                if !ss.contains(ts) {
                                    return false;
                                }
                            } else {
                                return false;
                            }
                        }
                        "between" => {
                            if let (Some(Value::Number(sn)), Value::List(items)) =
                                (stored_val, target)
                            {
                                if items.len() == 2 {
                                    if let (Value::Number(min_v), Value::Number(max_v)) =
                                        (&items[0], &items[1])
                                    {
                                        if let (Some(sf), Some(min_f), Some(max_f)) =
                                            (sn.as_f64(), min_v.as_f64(), max_v.as_f64())
                                        {
                                            if sf < min_f || sf > max_f {
                                                return false;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        "near" => {
                            // target is { "distance": Float, "coordinate": { "latitude": Float, "longitude": Float } }
                            if let Value::Object(near_args) = target {
                                if let (
                                    Some(Value::Number(dist_val)),
                                    Some(Value::Object(coord_map)),
                                ) = (near_args.get("distance"), near_args.get("coordinate"))
                                {
                                    if let (
                                        Some(Value::Number(lat_val)),
                                        Some(Value::Number(lon_val)),
                                    ) = (coord_map.get("latitude"), coord_map.get("longitude"))
                                    {
                                        if let (
                                            Some(max_meters),
                                            Some(target_lat),
                                            Some(target_lon),
                                        ) =
                                            (dist_val.as_f64(), lat_val.as_f64(), lon_val.as_f64())
                                        {
                                            // Check stored value
                                            // Stored: { "latitude": ..., "longitude": ... }
                                            if let Some(Value::Object(stored_map)) = stored_val {
                                                if let (
                                                    Some(Value::Number(s_lat_v)),
                                                    Some(Value::Number(s_lon_v)),
                                                ) = (
                                                    stored_map.get("latitude"),
                                                    stored_map.get("longitude"),
                                                ) {
                                                    if let (Some(s_lat), Some(s_lon)) =
                                                        (s_lat_v.as_f64(), s_lon_v.as_f64())
                                                    {
                                                        // Haversine Calculation
                                                        let earth_radius_m = 6371000.0;
                                                        let d_lat =
                                                            (target_lat - s_lat).to_radians();
                                                        let d_lon =
                                                            (target_lon - s_lon).to_radians();
                                                        let lat1 = s_lat.to_radians();
                                                        let lat2 = target_lat.to_radians();

                                                        let a = (d_lat / 2.0).sin().powi(2)
                                                            + lat1.cos()
                                                                * lat2.cos()
                                                                * (d_lon / 2.0).sin().powi(2);
                                                        let c =
                                                            2.0 * a.sqrt().atan2((1.0 - a).sqrt());
                                                        let distance = earth_radius_m * c;

                                                        if distance > max_meters {
                                                            return false;
                                                        }
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
                                    if let (
                                        Some(Value::Number(lat_v)),
                                        Some(Value::Number(lon_v)),
                                    ) = (stored_map.get("latitude"), stored_map.get("longitude"))
                                    {
                                        if let (Some(lat), Some(lon)) =
                                            (lat_v.as_f64(), lon_v.as_f64())
                                        {
                                            let point = geo::Point::new(lon, lat); // Geo uses (x, y) = (lon, lat)

                                            // Parse Target Polygon
                                            if let Some(Value::List(ext_list)) =
                                                polygon.get("exterior")
                                            {
                                                let mut ext_coords = Vec::new();
                                                for p in ext_list {
                                                    if let Value::Object(pmap) = p {
                                                        if let (
                                                            Some(Value::Number(plat)),
                                                            Some(Value::Number(plon)),
                                                        ) = (
                                                            pmap.get("latitude"),
                                                            pmap.get("longitude"),
                                                        ) {
                                                            if let (Some(ylat), Some(xlon)) =
                                                                (plat.as_f64(), plon.as_f64())
                                                            {
                                                                ext_coords.push((xlon, ylat));
                                                            }
                                                        }
                                                    }
                                                }
                                                if !ext_coords.is_empty() {
                                                    let line_string =
                                                        geo::LineString::from(ext_coords);
                                                    let poly =
                                                        geo::Polygon::new(line_string, vec![]);
                                                    use geo::contains::Contains;
                                                    if !poly.contains(&point) {
                                                        return false;
                                                    }
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
                                            if let (
                                                Some(Value::Number(plat)),
                                                Some(Value::Number(plon)),
                                            ) = (pmap.get("latitude"), pmap.get("longitude"))
                                            {
                                                if let (Some(ylat), Some(xlon)) =
                                                    (plat.as_f64(), plon.as_f64())
                                                {
                                                    stored_coords.push((xlon, ylat));
                                                }
                                            }
                                        }
                                    }
                                    if !stored_coords.is_empty() {
                                        let stored_poly = geo::Polygon::new(
                                            geo::LineString::from(stored_coords),
                                            vec![],
                                        );

                                        if let Value::Object(polygon) = target {
                                            if let Some(Value::List(ext_list)) =
                                                polygon.get("exterior")
                                            {
                                                let mut target_coords = Vec::new();
                                                for p in ext_list {
                                                    if let Value::Object(pmap) = p {
                                                        if let (
                                                            Some(Value::Number(plat)),
                                                            Some(Value::Number(plon)),
                                                        ) = (
                                                            pmap.get("latitude"),
                                                            pmap.get("longitude"),
                                                        ) {
                                                            if let (Some(ylat), Some(xlon)) =
                                                                (plat.as_f64(), plon.as_f64())
                                                            {
                                                                target_coords.push((xlon, ylat));
                                                            }
                                                        }
                                                    }
                                                }
                                                if !target_coords.is_empty() {
                                                    let target_poly = geo::Polygon::new(
                                                        geo::LineString::from(target_coords),
                                                        vec![],
                                                    );
                                                    use geo::intersects::Intersects;
                                                    if !stored_poly.intersects(&target_poly) {
                                                        return false;
                                                    }
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
                                    if !list.contains(sv) {
                                        return false;
                                    }
                                } else {
                                    // If stored value is null, can it be IN list? Only if list has null.
                                    if !list.contains(&Value::Null) {
                                        return false;
                                    }
                                }
                            }
                        }
                        "ne" => {
                            if let Some(sv) = stored_val {
                                if sv == target {
                                    return false;
                                }
                            } else {
                                if target == &Value::Null {
                                    return false;
                                }
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
                    None => condition == &Value::Null,
                }
            }
        }
    }

    fn check_filter_recursive_cached(
        &self,
        uid: u64,
        filter: &indexmap::IndexMap<async_graphql::Name, Value>,
        cache: Option<&RequestCache>,
    ) -> bool {
        for (key, condition) in filter {
            match key.as_str() {
                "and" => {
                    if let Value::List(list) = condition {
                        for sub in list {
                            if let Value::Object(map) = sub {
                                if !self.check_filter_recursive_cached(uid, map, cache) {
                                    return false;
                                }
                            }
                        }
                    }
                }
                "or" => {
                    if let Value::List(list) = condition {
                        let mut any = false;
                        for sub in list {
                            if let Value::Object(map) = sub {
                                if self.check_filter_recursive_cached(uid, map, cache) {
                                    any = true;
                                    break;
                                }
                            }
                        }
                        if !any {
                            return false;
                        }
                    }
                }
                "not" => {
                    if let Value::Object(map) = condition {
                        if self.check_filter_recursive_cached(uid, map, cache) {
                            return false;
                        }
                    }
                }
                field_name => {
                    let stored_val = self.resolve_cached(uid, field_name, cache);

                    if let Value::Object(sub_filter) = condition {
                        let is_operator_map = sub_filter.keys().any(|k| {
                            [
                                "eq",
                                "gt",
                                "lt",
                                "ge",
                                "le",
                                "contains",
                                "between",
                                "near",
                                "within",
                                "intersects",
                                "in",
                                "ne",
                                "allofterms",
                                "anyofterms",
                                "alloftext",
                                "anyoftext",
                            ]
                            .contains(&k.as_str())
                        });

                        if !is_operator_map {
                            match stored_val {
                                Some(Value::String(s)) => {
                                    if let Ok(child_uid) = s.parse::<u64>() {
                                        if !self.check_filter_recursive_cached(
                                            child_uid, sub_filter, cache,
                                        ) {
                                            return false;
                                        }
                                    } else {
                                        return false;
                                    }
                                }
                                Some(Value::Number(n)) => {
                                    if let Some(child_uid) = n.as_u64() {
                                        if !self.check_filter_recursive_cached(
                                            child_uid, sub_filter, cache,
                                        ) {
                                            return false;
                                        }
                                    } else {
                                        return false;
                                    }
                                }
                                Some(Value::List(list)) => {
                                    let mut match_found = false;
                                    for item in list {
                                        if let Some(child_uid) = Self::value_to_uid(&item) {
                                            if self.check_filter_recursive_cached(
                                                child_uid, sub_filter, cache,
                                            ) {
                                                match_found = true;
                                                break;
                                            }
                                        }
                                    }
                                    if !match_found {
                                        return false;
                                    }
                                }
                                _ => return false,
                            }
                            continue;
                        }
                    }

                    if !self.check_condition(&stored_val, condition) {
                        return false;
                    }
                }
            }
        }
        true
    }

    // Updated check_filter_recursive to support Deep Filtering (Relation Traversal)
    pub fn check_filter_recursive(
        &self,
        uid: u64,
        filter: &indexmap::IndexMap<async_graphql::Name, Value>,
    ) -> bool {
        self.check_filter_recursive_cached(uid, filter, None)
    }

    pub fn create_node_internal(
        &self,
        type_name: &str,
        uid: u64,
        mut fields: std::collections::HashMap<String, serde_json::Value>,
        uniques: &[String],
        inverses: &[crate::engine::resolver::InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
        source: crate::realtime::bus::MutationSource,
        timestamp_override: Option<crate::storage::timestamp::Timestamp>,
    ) -> Result<(), String> {
        let fn_start = std::time::Instant::now();
        // // Debug: trace inverse edge creation
        // if !inverses.is_empty() {
        //     eprintln!(
        //         "[create_node_internal] type={} uid={} inverses={}",
        //         type_name,
        //         uid,
        //         inverses.len()
        //     );
        //     for info in inverses {
        //         let field_val = fields.get(&info.field);
        //         eprintln!(
        //             "  inverse field='{}' val={:?} → {}.{} is_list={}",
        //             info.field,
        //             field_val,
        //             info.inverse_type,
        //             info.inverse_field,
        //             info.inverse_is_list
        //         );
        //     }
        // }
        // Normalize fields: If value is Object with uid/id, flatten to String(uid)
        for (_, value) in fields.iter_mut() {
            if let serde_json::Value::Object(map) = value {
                let uid_val = map.get("uid").or(map.get("id"));
                if let Some(u) = uid_val {
                    match u {
                        serde_json::Value::String(s) => {
                            *value = serde_json::Value::String(s.clone())
                        }
                        serde_json::Value::Number(n) => {
                            *value = serde_json::Value::String(n.to_string())
                        }
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
                                serde_json::Value::String(s) => {
                                    *item = serde_json::Value::String(s.clone())
                                }
                                serde_json::Value::Number(n) => {
                                    *item = serde_json::Value::String(n.to_string())
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Generate Timestamp for this Atomic Mutation or use override
        let timestamp = timestamp_override.unwrap_or_else(|| self.storage.next_timestamp());

        // Read keyspace before writer lock.
        let keyspaces = self.storage.keyspaces.read().unwrap();
        let (main, _history) = keyspaces
            .get(&self.db_name)
            .ok_or_else(|| format!("Database not found: {}", self.db_name))?;
        let main = main.clone();
        drop(keyspaces);

        let ts_bytes = timestamp.to_bytes();
        let mut items_to_index = Vec::new();
        let mut deferred_inverses: Vec<(u64, String, bool)> = Vec::new();
        let mut inv_count = 0u32;
        let uniq_start = std::time::Instant::now();

        // 1. User fields + unique checks
        for (field, value) in &fields {
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
            }
        }
        let uniq_time = uniq_start.elapsed();

        // 2. Inverse edges — list-type added to batch, non-list deferred
        for info in inverses {
            if let Some(val) = fields.get(&info.field) {
                let mut new_targets = Vec::new();
                Self::extract_target_uids(val, &mut new_targets);

                for target in new_targets {
                    if info.inverse_is_list {
                        deferred_inverses.push((target, info.inverse_field.clone(), true));
                    } else {
                        // Non-list needs read-before-write, defer after commit
                        deferred_inverses.push((target, info.inverse_field.clone(), false));
                    }
                    inv_count += 1;
                }
            }
        }

        // 3. Single atomic commit for all local key/value writes.
        let commit_start = std::time::Instant::now();
        let backend = self.storage.backends.get(&self.db_name).unwrap().clone();
        backend
            .write_batch(|conn| {
                let type_key_idx = Codec::encode_type_index_key(type_name, uid);
                main.batch_insert_on_conn(conn, &type_key_idx, &[])?;

                let type_val_bytes =
                    serde_json::to_vec(&serde_json::Value::String(type_name.to_string()))?;
                let type_data_key = Codec::encode_data_key(uid, "_type");
                let mut type_val_buf = Vec::with_capacity(16 + type_val_bytes.len());
                type_val_buf.extend_from_slice(&ts_bytes);
                type_val_buf.extend_from_slice(&type_val_bytes);
                main.batch_upsert_lww_on_conn(conn, &type_data_key, &type_val_buf, &ts_bytes)?;

                for (field, value) in &fields {
                    if uniques.contains(field) {
                        let index_pred = format!("{}.{}", type_name, field);
                        let val_str = serde_json::to_string(value)?;
                        let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                        let mut uid_bytes = vec![0u8; 8];
                        BigEndian::write_u64(&mut uid_bytes, uid);
                        main.batch_insert_on_conn(conn, &idx_key, &uid_bytes)?;
                    }

                    let val_bytes = serde_json::to_vec(value)?;
                    let key = Codec::encode_data_key(uid, field);
                    let mut val_buf = Vec::with_capacity(16 + val_bytes.len());
                    val_buf.extend_from_slice(&ts_bytes);
                    val_buf.extend_from_slice(&val_bytes);
                    main.batch_upsert_lww_on_conn(conn, &key, &val_buf, &ts_bytes)?;

                    if let Some(encoded_value) = Self::encode_order_index_value(value, false) {
                        let asc_key = Codec::encode_order_index_key(
                            type_name,
                            field,
                            false,
                            &encoded_value,
                            uid,
                        );
                        main.batch_insert_on_conn(conn, &asc_key, &[])?;
                    }
                    if let Some(encoded_value) = Self::encode_order_index_value(value, true) {
                        let desc_key = Codec::encode_order_index_key(
                            type_name,
                            field,
                            true,
                            &encoded_value,
                            uid,
                        );
                        main.batch_insert_on_conn(conn, &desc_key, &[])?;
                    }
                }

                for (target, inverse_field, is_list) in &deferred_inverses {
                    if *is_list {
                        let edge_key = Codec::encode_edge_key(*target, inverse_field, uid);
                        let reverse_edge_key =
                            Codec::encode_reverse_edge_key(uid, *target, inverse_field);
                        main.batch_insert_on_conn(conn, &edge_key, &[])?;
                        main.batch_insert_on_conn(conn, &reverse_edge_key, &[])?;
                    }
                }

                Ok(())
            })
            .map_err(|e| e.to_string())?;
        let commit_time = commit_start.elapsed();

        // 4. Handle deferred non-list inverse links after commit.
        for (target, inverse_field, is_list) in deferred_inverses {
            if !is_list {
                self.link_inverse(target, &inverse_field, false, uid, &timestamp)?;
            }
        }

        // 5. Handle Search Indexing after commit.
        for (field, val, tokenizers) in items_to_index {
            for strategy in tokenizers {
                if let Err(e) = self.write_term_index(uid, &field, &val, &strategy) {
                    eprintln!(
                        "Search Indexing Failed (create_node) for uid={}: {}",
                        uid, e
                    );
                }
            }
        }

        let total = fn_start.elapsed();
        if crate::debug_logging() && total.as_millis() > 2 {
            eprintln!(
                "[RESOLVER] create_node {} | uniq={:?} commit={:?} inv_count={} total={:?}",
                type_name, uniq_time, commit_time, inv_count, total
            );
        }

        // 6. Publish Event (Realtime)
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
            node_id: self.storage.node_id,
        });

        Ok(())
    }

    /// High-throughput batch insert — wraps ALL records in ONE SQLite transaction.
    ///
    /// `create_node_internal` auto-commits each `main.insert()` individually.
    /// For 203k SynsetRelations × ~5 fields = ~1M individual commits — extremely slow.
    /// This method holds the writer lock for the full batch (one BEGIN/COMMIT), which
    /// is typically 100× faster for large bulk imports.
    ///
    /// Trade-offs vs `create_node_internal`:
    ///   ✗ No unique-field enforcement (caller must ensure data is clean)
    ///   ✗ No BM25 / term indexing
    ///   ✗ No realtime event publishing
    pub fn batch_create_nodes(
        &self,
        records: &[crate::server::bulk_ingest::BulkRecord],
        type_metadata: &std::collections::HashMap<String, crate::engine::schema::TypeMetadata>,
    ) -> Result<(), String> {
        // Read keyspace BEFORE writer lock (different mutex — no deadlock concern).
        let keyspaces = self.storage.keyspaces.read().unwrap();
        let (main, _) = keyspaces
            .get(&self.db_name)
            .ok_or_else(|| format!("Database not found: {}", self.db_name))?;
        let main_clone = main.clone();
        drop(keyspaces);

        let ts = self.storage.next_timestamp();
        let ts_bytes = ts.to_bytes();

        let backend = self.storage.backends.get(&self.db_name).unwrap().clone();
        backend
            .write_batch(|conn| {
                // Log metadata for debugging
                for (type_name, meta) in type_metadata {
                    if !meta.inverses.is_empty() {
                        tracing::debug!(
                            "[batch_create_nodes] Type '{}' has {} inverses",
                            type_name,
                            meta.inverses.len()
                        );
                    }
                }

                for record in records {
                    let uid = record.uid.unwrap_or_else(|| {
                        let t = self.storage.next_timestamp();
                        (t.millis << 16) | (t.counter as u64)
                    });

                    // Type index entry
                    let type_idx_key = Codec::encode_type_index_key(&record.type_name, uid);
                    main_clone.batch_insert_on_conn(conn, &type_idx_key, &[])?;

                    // _type data field
                    let type_val =
                        serde_json::to_vec(&serde_json::Value::String(record.type_name.clone()))
                            .map_err(|e| anyhow::anyhow!(e))?;
                    let type_data_key = Codec::encode_data_key(uid, "_type");
                    let mut type_buf = Vec::with_capacity(16 + type_val.len());
                    type_buf.extend_from_slice(&ts_bytes);
                    type_buf.extend_from_slice(&type_val);
                    main_clone.batch_upsert_lww_on_conn(
                        conn,
                        &type_data_key,
                        &type_buf,
                        &ts_bytes,
                    )?;

                    // User fields — normalize { id/uid: X } references to their string value
                    for (field, value) in &record.fields {
                        let normalized: serde_json::Value = match value {
                            serde_json::Value::Object(map) => map
                                .get("uid")
                                .or_else(|| map.get("id"))
                                .and_then(|v| match v {
                                    serde_json::Value::String(s) => {
                                        Some(serde_json::Value::String(s.clone()))
                                    }
                                    serde_json::Value::Number(n) => {
                                        Some(serde_json::Value::String(n.to_string()))
                                    }
                                    _ => None,
                                })
                                .unwrap_or_else(|| value.clone()),
                            _ => value.clone(),
                        };
                        let val_bytes =
                            serde_json::to_vec(&normalized).map_err(|e| anyhow::anyhow!(e))?;
                        let key = Codec::encode_data_key(uid, field);
                        let mut val_buf = Vec::with_capacity(16 + val_bytes.len());
                        val_buf.extend_from_slice(&ts_bytes);
                        val_buf.extend_from_slice(&val_bytes);
                        main_clone.batch_upsert_lww_on_conn(conn, &key, &val_buf, &ts_bytes)?;

                        if let Some(encoded_value) =
                            Self::encode_order_index_value(&normalized, false)
                        {
                            let asc_key = Codec::encode_order_index_key(
                                &record.type_name,
                                field,
                                false,
                                &encoded_value,
                                uid,
                            );
                            main_clone.batch_insert_on_conn(conn, &asc_key, &[])?;
                        }
                        if let Some(encoded_value) =
                            Self::encode_order_index_value(&normalized, true)
                        {
                            let desc_key = Codec::encode_order_index_key(
                                &record.type_name,
                                field,
                                true,
                                &encoded_value,
                                uid,
                            );
                            main_clone.batch_insert_on_conn(conn, &desc_key, &[])?;
                        }
                    }

                    // @hasInverse edge writing
                    if let Some(meta) = type_metadata.get(&record.type_name) {
                        for info in &meta.inverses {
                            if let Some(val) = record.fields.get(&info.field) {
                                // Extract target UIDs from the ORIGINAL field value (before normalization)
                                let mut targets = Vec::new();
                                Self::extract_target_uids(val, &mut targets);

                                for target in targets {
                                    if info.inverse_is_list {
                                        // List inverse: write edge key
                                        let edge_key = Codec::encode_edge_key(
                                            target,
                                            &info.inverse_field,
                                            uid,
                                        );
                                        let reverse_edge_key = Codec::encode_reverse_edge_key(
                                            uid,
                                            target,
                                            &info.inverse_field,
                                        );
                                        main_clone.batch_insert_on_conn(conn, &edge_key, &[])?;
                                        main_clone.batch_insert_on_conn(
                                            conn,
                                            &reverse_edge_key,
                                            &[],
                                        )?;
                                    } else {
                                        // Non-list inverse: write data key with LWW
                                        let inv_val = serde_json::Value::String(uid.to_string());
                                        let inv_val_bytes = serde_json::to_vec(&inv_val)
                                            .map_err(|e| anyhow::anyhow!(e))?;
                                        let inv_key =
                                            Codec::encode_data_key(target, &info.inverse_field);
                                        let mut inv_buf =
                                            Vec::with_capacity(16 + inv_val_bytes.len());
                                        inv_buf.extend_from_slice(&ts_bytes);
                                        inv_buf.extend_from_slice(&inv_val_bytes);
                                        main_clone.batch_upsert_lww_on_conn(
                                            conn, &inv_key, &inv_buf, &ts_bytes,
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            })
            .map_err(|e| e.to_string())
    }

    /// Helper: extract u64 target UIDs from a JSON field value (string, number, object with id/uid, or array thereof)
    fn extract_target_uids(val: &serde_json::Value, targets: &mut Vec<u64>) {
        match val {
            serde_json::Value::String(s) => {
                if let Ok(id) = s.parse::<u64>() {
                    targets.push(id);
                }
            }
            serde_json::Value::Number(n) => {
                if let Some(id) = n.as_u64() {
                    targets.push(id);
                }
            }
            serde_json::Value::Object(map) => {
                if let Some(id_val) = map.get("uid").or_else(|| map.get("id")) {
                    match id_val {
                        serde_json::Value::String(s) => {
                            if let Ok(id) = s.parse::<u64>() {
                                targets.push(id);
                            }
                        }
                        serde_json::Value::Number(n) => {
                            if let Some(id) = n.as_u64() {
                                targets.push(id);
                            }
                        }
                        _ => {}
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    Self::extract_target_uids(item, targets);
                }
            }
            _ => {}
        }
    }

    fn extract_vector(val: &Value) -> Option<Vec<f64>> {
        let Value::List(list) = val else {
            return None;
        };

        let vec_data: Vec<f64> = list
            .iter()
            .filter_map(|v| match v {
                Value::Number(n) => n.as_f64(),
                _ => None,
            })
            .collect();

        if vec_data.is_empty() {
            None
        } else {
            Some(vec_data)
        }
    }

    pub fn update_node_internal(
        &self,
        type_name: &str,
        uid: u64,
        mut fields: std::collections::HashMap<String, serde_json::Value>,
        uniques: &[String],
        inverses: &[crate::engine::resolver::InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
        source: crate::realtime::bus::MutationSource,
        timestamp_override: Option<crate::storage::timestamp::Timestamp>,
    ) -> Result<(), String> {
        // Normalize fields: If value is Object with uid/id, flatten to String(uid)
        for (_, value) in fields.iter_mut() {
            if let serde_json::Value::Object(map) = value {
                let uid_val = map.get("uid").or(map.get("id"));
                if let Some(u) = uid_val {
                    match u {
                        serde_json::Value::String(s) => {
                            *value = serde_json::Value::String(s.clone())
                        }
                        serde_json::Value::Number(n) => {
                            *value = serde_json::Value::String(n.to_string())
                        }
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
                                serde_json::Value::String(s) => {
                                    *item = serde_json::Value::String(s.clone())
                                }
                                serde_json::Value::Number(n) => {
                                    *item = serde_json::Value::String(n.to_string())
                                }
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
                    let payload = if bytes.len() > 16 {
                        &bytes[16..]
                    } else {
                        &bytes
                    };
                    if let Ok(serde_json::Value::String(s)) =
                        serde_json::from_slice::<serde_json::Value>(payload)
                    {
                        for strategy in tokenizers {
                            self.remove_term_index(uid, field, &s, strategy)?;
                        }
                    }
                }
            }

            let data_key = Codec::encode_data_key(uid, field);
            if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                let payload = if bytes.len() > 16 {
                    &bytes[16..]
                } else {
                    &bytes
                };
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(payload) {
                    self.remove_order_index(type_name, uid, field, &val)?;
                }
            }
        }
        // 1. Unlink Inverses
        for info in inverses {
            if fields.contains_key(&info.field) {
                let data_key = Codec::encode_data_key(uid, &info.field);
                if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                    let mut old_targets = Vec::new();
                    let payload = if bytes.len() > 16 {
                        &bytes[16..]
                    } else {
                        &bytes
                    };
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(payload) {
                        match val {
                            serde_json::Value::String(s) => {
                                if let Ok(id) = s.parse::<u64>() {
                                    old_targets.push(id);
                                }
                            }
                            serde_json::Value::Number(n) => {
                                if let Some(id) = n.as_u64() {
                                    old_targets.push(id);
                                }
                            }
                            serde_json::Value::Object(map) => {
                                // Try "uid" then "id"
                                if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                                    match uid_val {
                                        serde_json::Value::String(s) => {
                                            if let Ok(id) = s.parse::<u64>() {
                                                old_targets.push(id);
                                            }
                                        }
                                        serde_json::Value::Number(n) => {
                                            if let Some(id) = n.as_u64() {
                                                old_targets.push(id);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            serde_json::Value::Array(items) => {
                                for item in items {
                                    match item {
                                        serde_json::Value::String(s) => {
                                            if let Ok(id) = s.parse::<u64>() {
                                                old_targets.push(id);
                                            }
                                        }
                                        serde_json::Value::Number(n) => {
                                            if let Some(id) = n.as_u64() {
                                                old_targets.push(id);
                                            }
                                        }
                                        serde_json::Value::Object(map) => {
                                            if let Some(uid_val) = map.get("uid").or(map.get("id"))
                                            {
                                                match uid_val {
                                                    serde_json::Value::String(s) => {
                                                        if let Ok(id) = s.parse::<u64>() {
                                                            old_targets.push(id);
                                                        }
                                                    }
                                                    serde_json::Value::Number(n) => {
                                                        if let Some(id) = n.as_u64() {
                                                            old_targets.push(id);
                                                        }
                                                    }
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
                        self.unlink_inverse(
                            target,
                            &info.inverse_field,
                            info.inverse_is_list,
                            uid,
                            &timestamp,
                        )?;
                    }
                }
            }
        }
        // 2. Remove Old Unique Indexes
        for field in uniques {
            if fields.contains_key(field) {
                let data_key = Codec::encode_data_key(uid, field);
                if let Ok(Some(val_bytes)) = self.storage.get(&self.db_name, &data_key) {
                    let payload = if val_bytes.len() > 16 {
                        &val_bytes[16..]
                    } else {
                        &val_bytes
                    };
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(payload) {
                        let val_str = serde_json::to_string(&val).unwrap_or_default();
                        let index_pred = format!("{}.{}", type_name, field);
                        let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                        self.storage
                            .remove(&self.db_name, &idx_key)
                            .map_err(|e| e.to_string())?;
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
                self.storage
                    .insert(&self.db_name, &idx_key, &uid_bytes)
                    .map_err(|e| e.to_string())?;
            }
            let mut val_buf = Vec::with_capacity(16 + val_bytes.len());
            let ts_bytes = timestamp.to_bytes();
            val_buf.extend_from_slice(&ts_bytes);
            val_buf.extend_from_slice(&val_bytes);
            batch_items.push((uid, field.clone(), val_buf));
            self.write_order_index(type_name, uid, field, value)?;
        }

        self.storage
            .put_batch_lww(&self.db_name, batch_items, &timestamp)
            .map_err(|e| e.to_string())?;
        // 4. Link New Inverses
        for info in inverses {
            if let Some(val) = fields.get(&info.field) {
                let mut new_targets = Vec::new();
                match val {
                    serde_json::Value::String(s) => {
                        if let Ok(id) = s.parse::<u64>() {
                            new_targets.push(id);
                        }
                    }
                    serde_json::Value::Number(n) => {
                        if let Some(id) = n.as_u64() {
                            new_targets.push(id);
                        }
                    }
                    serde_json::Value::Object(map) => {
                        // Try "uid" then "id"
                        if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                            match uid_val {
                                serde_json::Value::String(s) => {
                                    if let Ok(id) = s.parse::<u64>() {
                                        new_targets.push(id);
                                    }
                                }
                                serde_json::Value::Number(n) => {
                                    if let Some(id) = n.as_u64() {
                                        new_targets.push(id);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    serde_json::Value::Array(items) => {
                        for item in items {
                            match item {
                                serde_json::Value::String(s) => {
                                    if let Ok(id) = s.parse::<u64>() {
                                        new_targets.push(id);
                                    }
                                }
                                serde_json::Value::Number(n) => {
                                    if let Some(id) = n.as_u64() {
                                        new_targets.push(id);
                                    }
                                }
                                serde_json::Value::Object(map) => {
                                    if let Some(uid_val) = map.get("uid").or(map.get("id")) {
                                        match uid_val {
                                            serde_json::Value::String(s) => {
                                                if let Ok(id) = s.parse::<u64>() {
                                                    new_targets.push(id);
                                                }
                                            }
                                            serde_json::Value::Number(n) => {
                                                if let Some(id) = n.as_u64() {
                                                    new_targets.push(id);
                                                }
                                            }
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
                    self.link_inverse(
                        target,
                        &info.inverse_field,
                        info.inverse_is_list,
                        uid,
                        &timestamp,
                    )?;
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
            node_id: self.storage.node_id,
        });
        Ok(())
    }

    pub fn delete_node_internal(
        &self,
        type_name: &str,
        uid: u64,
        uniques: &[String],
        inverses: &[crate::engine::resolver::InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
        source: crate::realtime::bus::MutationSource,
        timestamp_override: Option<crate::storage::timestamp::Timestamp>,
    ) -> Result<(), String> {
        let timestamp = timestamp_override.unwrap_or_else(|| self.storage.next_timestamp());
        // 0. Remove Search Indexes
        for (field, tokenizers) in search_fields {
            let data_key = Codec::encode_data_key(uid, field);
            if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                // Skip 16-byte timestamp prefix
                let payload = if bytes.len() > 16 {
                    &bytes[16..]
                } else {
                    &bytes
                };
                if let Ok(serde_json::Value::String(s)) =
                    serde_json::from_slice::<serde_json::Value>(payload)
                {
                    for strategy in tokenizers {
                        self.remove_term_index(uid, field, &s, strategy)?;
                    }
                }
            }

            if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                let payload = if bytes.len() > 16 {
                    &bytes[16..]
                } else {
                    &bytes
                };
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(payload) {
                    self.remove_order_index(type_name, uid, field, &val)?;
                }
            }
        }
        // 1. Handle Inverses (Unlink)
        for info in inverses {
            let data_key = Codec::encode_data_key(uid, &info.field);
            if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &data_key) {
                let mut targets = Vec::new();
                // Skip 16-byte timestamp prefix
                let payload = if bytes.len() > 16 {
                    &bytes[16..]
                } else {
                    &bytes
                };
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(payload) {
                    match val {
                        serde_json::Value::String(s) => {
                            if let Ok(id) = s.parse::<u64>() {
                                targets.push(id);
                            }
                        }
                        serde_json::Value::Number(n) => {
                            if let Some(id) = n.as_u64() {
                                targets.push(id);
                            }
                        }
                        serde_json::Value::Object(map) => {
                            if let Some(uid_val) = map.get("uid").or_else(|| map.get("id")) {
                                if let Some(s) = uid_val.as_str() {
                                    if let Ok(id) = s.parse::<u64>() {
                                        targets.push(id);
                                    }
                                }
                            }
                        }
                        serde_json::Value::Array(items) => {
                            for item in items {
                                match item {
                                    serde_json::Value::String(s) => {
                                        if let Ok(id) = s.parse::<u64>() {
                                            targets.push(id);
                                        }
                                    }
                                    serde_json::Value::Number(n) => {
                                        if let Some(id) = n.as_u64() {
                                            targets.push(id);
                                        }
                                    }
                                    serde_json::Value::Object(map) => {
                                        if let Some(uid_val) =
                                            map.get("uid").or_else(|| map.get("id"))
                                        {
                                            if let Some(s) = uid_val.as_str() {
                                                if let Ok(id) = s.parse::<u64>() {
                                                    targets.push(id);
                                                }
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
                for target in targets {
                    self.unlink_inverse(
                        target,
                        &info.inverse_field,
                        info.inverse_is_list,
                        uid,
                        &timestamp,
                    )?;
                }
            }
        }
        // 2. Remove Unique Indexes
        for field in uniques {
            let data_key = Codec::encode_data_key(uid, field);
            if let Ok(Some(val_bytes)) = self.storage.get(&self.db_name, &data_key) {
                // Skip 16-byte timestamp prefix
                let payload = if val_bytes.len() > 16 {
                    &val_bytes[16..]
                } else {
                    &val_bytes
                };
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(payload) {
                    let val_str = serde_json::to_string(&val).unwrap_or_default();
                    let index_pred = format!("{}.{}", type_name, field);
                    let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                    self.storage
                        .remove(&self.db_name, &idx_key)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        // 3. Remove Ordered Secondary Indexes
        let data_prefix = Codec::encode_data_prefix(uid);
        if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
            for (key, bytes) in main_ks.prefix(&data_prefix) {
                if !key.starts_with(&data_prefix) || key.len() <= data_prefix.len() {
                    break;
                }
                let field = match std::str::from_utf8(&key[data_prefix.len()..]) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                if field == "_type" {
                    continue;
                }
                let payload = if bytes.len() > 16 {
                    &bytes[16..]
                } else {
                    &bytes
                };
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(payload) {
                    self.remove_order_index(type_name, uid, field, &val)?;
                }
            }
        }
        // 4. Remove Type Index
        let type_key = Codec::encode_type_index_key(type_name, uid);
        self.storage
            .remove(&self.db_name, &type_key)
            .map_err(|e| e.to_string())?;

        // 5. Remove Vector Data (Soft Delete)
        // We delete indiscriminately; if no vector existed, it's a safe no-op.
        self.storage.delete_vector(uid).map_err(|e| e.to_string())?;

        // 6. Remove Data Keys (Scan Prefix)
        let prefix = Codec::encode_data_prefix(uid);
        let (main_ks, _) = self
            .storage
            .get_database(&self.db_name)
            .ok_or("Database not found".to_string())?;
        let iter = main_ks.prefix(&prefix);
        let mut keys_to_delete: Vec<Vec<u8>> = Vec::new();
        for (key, _val) in iter {
            if !key.starts_with(&prefix) {
                break;
            }
            keys_to_delete.push(key.to_vec());
        }
        for k in keys_to_delete {
            if k.len() > 9 {
                if let Ok(pred) = std::str::from_utf8(&k[9..]) {
                    self.storage
                        .delete_with_lww(&self.db_name, uid, pred, &timestamp)
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        // 6. Remove any outbound list-edge keys, including undeclared legacy edges.
        let reverse_prefix = Codec::encode_reverse_edge_prefix(uid);
        let reverse_edges = main_ks.prefix(&reverse_prefix);
        for (key, _val) in reverse_edges {
            if !key.starts_with(&reverse_prefix) {
                break;
            }
            if let Some((target_uid, field)) = Codec::decode_reverse_edge_target_and_field(&key) {
                let edge_key = Codec::encode_edge_key(target_uid, &field, uid);
                self.storage
                    .delete_key(&self.db_name, &edge_key)
                    .map_err(|e| e.to_string())?;
                self.storage
                    .delete_key(&self.db_name, &key)
                    .map_err(|e| e.to_string())?;
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
            node_id: self.storage.node_id,
        });
        Ok(())
    }

    pub fn apply_remote_mutation(
        &self,
        event: crate::realtime::bus::MutationEvent,
    ) -> Result<(), String> {
        let metadata = event
            .metadata
            .ok_or("Missing metadata for remote mutation")?;
        let source = crate::realtime::bus::MutationSource::Remote;

        let result = match event.mutation_type {
            crate::realtime::bus::MutationType::Create => {
                let payload = event.payload.clone().ok_or("Missing payload for Create")?;
                self.create_node_internal(
                    &event.type_name,
                    event.uid,
                    payload,
                    &metadata.uniques,
                    &metadata.inverses,
                    &metadata.search_fields,
                    source,
                    event.timestamp,
                )
            }
            crate::realtime::bus::MutationType::Update => {
                let payload = event.payload.clone().ok_or("Missing payload for Update")?;
                self.update_node_internal(
                    &event.type_name,
                    event.uid,
                    payload,
                    &metadata.uniques,
                    &metadata.inverses,
                    &metadata.search_fields,
                    source,
                    event.timestamp,
                )
            }
            crate::realtime::bus::MutationType::Delete => self.delete_node_internal(
                &event.type_name,
                event.uid,
                &metadata.uniques,
                &metadata.inverses,
                &metadata.search_fields,
                source,
                event.timestamp,
            ),
        };

        if let Err(e) = result {
            eprintln!("Quarantining mutation due to error: {}", e);
            let timestamp = self.storage.next_timestamp();
            if let Some(payload) = event.payload {
                for (field, value) in payload {
                    if let Ok(bytes) = serde_json::to_vec(&value) {
                        let _ = self
                            .storage
                            .put_quarantine(event.uid, &field, &bytes, &timestamp);
                    }
                }
            }
            return Err(e);
        }
        Ok(())
    }
}

impl SqliteResolver {
    fn resolve_list_internal(
        &self,
        parent_uid: u64,
        field_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        near_vector: Option<Vec<f64>>,
        cache: Option<&RequestCache>,
    ) -> Result<Vec<u64>, String> {
        let start = std::time::Instant::now();
        // The relation pipeline never consults type metadata (edge scan +
        // cosine re-rank + generic operators only), so an empty catalog is
        // sufficient here.
        let metadata = std::collections::HashMap::new();
        let runtime = crate::query_planner::adapters::runtime_for(self, &metadata);
        let mut ctx = crate::query_planner::operators::ExecContext::new_with_explain(
            &runtime,
            &self.db_name,
            crate::query_planner::debug_capture::enabled(),
        );
        let built = crate::query_planner::operators::build_relation_pipeline(
            parent_uid,
            field_name,
            &filter,
            &sort,
            first,
            after.as_deref(),
            offset,
            near_vector.clone(),
        );
        metrics::histogram!("vardadb_planner_candidate_duration_seconds")
            .record(start.elapsed().as_secs_f64());
        metrics::counter!(
            "vardadb_planner_access_total",
            "shape" => built.shape.clone()
        )
        .increment(1);

        let batches = match built.root.execute(&mut ctx) {
            crate::query_planner::operators::FlowResult::Rows(batches) => batches,
            _ => Vec::new(),
        };
        let uids: Vec<u64> = batches
            .into_iter()
            .flat_map(|b| b.0.into_iter().map(|e| e.uid))
            .collect();

        crate::query_planner::debug_capture::record(
            crate::query_planner::debug_capture::CapturedPlan {
                captured_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or_default(),
                db: self.db_name.clone(),
                kind: "relation".to_string(),
                type_name: format!("[parent:{}] {}", parent_uid, field_name),
                shape: built.shape.clone(),
                text: built
                    .plan
                    .as_ref()
                    .map(crate::query_planner::render_candidate_plan)
                    .unwrap_or_else(|| format!("relation pipeline shape={}", built.shape)),
                plan_json: built
                    .plan
                    .as_ref()
                    .map(crate::query_planner::explain::candidate_plan_json),
                operator_stats: ctx.explain.take_stats(),
                elapsed_us: start.elapsed().as_micros() as u64,
            },
        );
        if let Some(cache) = cache {
            self.preload_objects_for_uids(&uids, cache);
        }

        let total = start.elapsed();
        if crate::debug_logging() && total.as_millis() > 10 {
            eprintln!(
                "[RESOLVER] resolve_list {}.{} parent={} base={} filter={} sort={} total={}ms shape={}",
                self.db_name,
                field_name,
                parent_uid,
                uids.len(),
                !filter.is_empty(),
                !sort.is_empty(),
                total.as_millis(),
                built.shape,
            );
        }

        Ok(uids)
    }


    /// Shared legacy text-predicate detection over the raw GraphQL filter map.
    /// Returns `(field, strategy, query, require_all)`.
    fn detect_text_search(
        filter: &std::collections::HashMap<String, Value>,
    ) -> Option<(String, String, String, bool)> {
        for (field, val) in filter {
            if let Value::Object(obj) = val {
                if let Some(Value::String(s)) = obj.get("allofterms") {
                    return Some((field.clone(), "term".to_string(), s.clone(), true));
                }
                if let Some(Value::String(s)) = obj.get("anyofterms") {
                    return Some((field.clone(), "term".to_string(), s.clone(), false));
                }
                if let Some(Value::String(s)) = obj.get("alloftext") {
                    return Some((field.clone(), "fulltext".to_string(), s.clone(), true));
                }
                if let Some(Value::String(s)) = obj.get("anyoftext") {
                    return Some((field.clone(), "fulltext".to_string(), s.clone(), false));
                }
            }
        }
        None
    }

    /// Thin dispatcher over the planner operator pipeline (Stage 2.1 cutover).
    pub fn scan_nodes_internal(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
        cache: Option<&RequestCache>,
    ) -> Vec<u64> {
        let start = std::time::Instant::now();
        let text_search = Self::detect_text_search(&filter);
        let has_text_search = text_search.is_some();

        let candidate_start = std::time::Instant::now();
        let runtime = crate::query_planner::adapters::runtime_for(self, query_metadata);
        let mut ctx = crate::query_planner::operators::ExecContext::new_with_explain(
            &runtime,
            &self.db_name,
            crate::query_planner::debug_capture::enabled(),
        );
        let built = crate::query_planner::operators::build_scan_pipeline(
            &self.db_name,
            type_name,
            &filter,
            &sort,
            first,
            after.as_deref(),
            offset,
            near_vector.as_ref(),
            text_search.as_ref(),
            uniques,
            query_metadata,
            &runtime,
            &mut ctx,
        );
        let candidate_time = candidate_start.elapsed();
        metrics::histogram!("vardadb_planner_candidate_duration_seconds")
            .record(candidate_time.as_secs_f64());
        metrics::counter!("vardadb_planner_access_total", "shape" => built.shape.clone())
            .increment(1);
        if crate::debug_logging() {
            if let Some(plan) = &built.plan {
                eprintln!(
                    "[PLANNER] candidate plan {}.{} shape={}:\n{}",
                    self.db_name,
                    type_name,
                    built.shape,
                    crate::query_planner::render_candidate_plan(plan)
                );
            }
        }

        let batches = match built.root.execute(&mut ctx) {
            crate::query_planner::operators::FlowResult::Rows(batches) => batches,
            _ => Vec::new(),
        };
        let uids: Vec<u64> = batches
            .into_iter()
            .flat_map(|b| b.0.into_iter().map(|e| e.uid))
            .collect();

        crate::query_planner::debug_capture::record(
            crate::query_planner::debug_capture::CapturedPlan {
                captured_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or_default(),
                db: self.db_name.clone(),
                kind: "scan".to_string(),
                type_name: type_name.to_string(),
                shape: built.shape.clone(),
                text: built
                    .plan
                    .as_ref()
                    .map(crate::query_planner::render_candidate_plan)
                    .unwrap_or_else(|| format!("search pipeline shape={}", built.shape)),
                plan_json: built
                    .plan
                    .as_ref()
                    .map(crate::query_planner::explain::candidate_plan_json),
                operator_stats: ctx.explain.take_stats(),
                elapsed_us: start.elapsed().as_micros() as u64,
            },
        );

        if let Some(cache) = cache {
            self.preload_objects_for_uids(&uids, cache);
        }

        let total = start.elapsed();
        if crate::debug_logging() && total.as_millis() > 10 {
            eprintln!(
                "[RESOLVER] scan_nodes {}.{} filter={} sort={} first={:?} offset={:?} candidates={} candidate_ms={} result_count={} total_ms={} near_vector={} text_search={} shape={}",
                self.db_name,
                type_name,
                !filter.is_empty(),
                !sort.is_empty(),
                first,
                offset,
                built.used_candidates,
                candidate_time.as_millis(),
                uids.len(),
                total.as_millis(),
                near_vector.is_some(),
                has_text_search,
                built.shape,
            );
        }

        uids
    }

    /// Legacy count dispatcher kept public for the Phase-1 parity bridge/tests.
    pub fn count_nodes_internal(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
        _cache: Option<&RequestCache>,
    ) -> usize {
        let start = std::time::Instant::now();
        let text_search = Self::detect_text_search(&filter);
        let has_text_search = text_search.is_some();

        // Fast path preserved from legacy: an unfiltered plain count uses the
        // O(prefix) SQL counter without assembling a pipeline.
        if filter.is_empty() && near_vector.is_none() && !has_text_search {
            if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                return main_ks
                    .count_prefix(&Codec::encode_type_prefix(type_name))
                    .unwrap_or(0);
            }
            return 0;
        }

        let candidate_start = std::time::Instant::now();
        let runtime = crate::query_planner::adapters::runtime_for(self, query_metadata);
        let mut ctx = crate::query_planner::operators::ExecContext::new_with_explain(
            &runtime,
            &self.db_name,
            crate::query_planner::debug_capture::enabled(),
        );
        let built = crate::query_planner::operators::build_count_pipeline(
            &self.db_name,
            type_name,
            &filter,
            near_vector.as_ref(),
            text_search.as_ref(),
            uniques,
            query_metadata,
            &runtime,
        );
        let candidate_time = candidate_start.elapsed();
        metrics::histogram!("vardadb_planner_candidate_duration_seconds")
            .record(candidate_time.as_secs_f64());
        metrics::counter!("vardadb_planner_access_total", "shape" => built.shape.clone())
            .increment(1);

        let count = match built.root.execute(&mut ctx) {
            crate::query_planner::operators::FlowResult::Rows(batches) => {
                batches.into_iter().map(|b| b.len()).sum()
            }
            _ => 0,
        };

        crate::query_planner::debug_capture::record(
            crate::query_planner::debug_capture::CapturedPlan {
                captured_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or_default(),
                db: self.db_name.clone(),
                kind: "count".to_string(),
                type_name: type_name.to_string(),
                shape: built.shape.clone(),
                text: built
                    .plan
                    .as_ref()
                    .map(crate::query_planner::render_candidate_plan)
                    .unwrap_or_else(|| format!("count pipeline shape={}", built.shape)),
                plan_json: built
                    .plan
                    .as_ref()
                    .map(crate::query_planner::explain::candidate_plan_json),
                operator_stats: ctx.explain.take_stats(),
                elapsed_us: start.elapsed().as_micros() as u64,
            },
        );

        if crate::debug_logging() && start.elapsed().as_millis() > 10 {
            eprintln!(
                "[RESOLVER] count_nodes {}.{} candidates={} candidate_ms={} count={} total_ms={} near_vector={} text_search={}",
                self.db_name,
                type_name,
                built.used_candidates,
                candidate_time.as_millis(),
                count,
                start.elapsed().as_millis(),
                near_vector.is_some(),
                has_text_search,
            );
        }

        count
    }
}

impl Resolver for SqliteResolver {
    fn bulk_check_permission(
        &self,
        ctx: &async_graphql::dynamic::ResolverContext<'_>,
        checks: Vec<(String, String, String)>,
    ) -> async_graphql::Result<Vec<(String, String, String, bool)>> {
        // Extract authenticated user from GraphQL context (injected by auth middleware)
        let subject = if let Ok(auth) = ctx.data::<auth::middleware::JWTAuthMiddleware>() {
            permissions::storage::tuple::Subject {
                entity: "user".to_string(),
                id: auth.user.id.clone(),
            }
        } else {
            // Unauthenticated — will be denied by default
            permissions::storage::tuple::Subject {
                entity: "anonymous".to_string(),
                id: "anonymous".to_string(),
            }
        };

        let eval_context = permissions::engine::context::Context::new();
        let schema_registry = permissions::schema::registry::SchemaRegistry::new();

        let check_refs: Vec<(&str, &str, &str)> = checks
            .iter()
            .map(|(et, eid, p)| (et.as_str(), eid.as_str(), p.as_str()))
            .collect();

        let results = permissions::engine::check::bulk_check(
            &self.storage.auth_store,
            &schema_registry,
            "default",
            check_refs,
            &subject,
            &eval_context,
        );

        let final_results = checks
            .into_iter()
            .zip(results)
            .map(|((et, eid, p), res)| {
                (
                    et,
                    eid,
                    p,
                    res == permissions::engine::check::CheckResult::Allow,
                )
            })
            .collect();

        Ok(final_results)
    }

    fn resolve_list(
        &self,
        parent_uid: u64,
        field_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        near_vector: Option<Vec<f64>>,
    ) -> Result<Vec<u64>, String> {
        self.resolve_list_internal(
            parent_uid,
            field_name,
            filter,
            sort,
            first,
            after,
            offset,
            near_vector,
            None,
        )
    }

    fn resolve_list_with_cache(
        &self,
        parent_uid: u64,
        field_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        near_vector: Option<Vec<f64>>,
        cache: &RequestCache,
    ) -> Result<Vec<u64>, String> {
        self.resolve_list_internal(
            parent_uid,
            field_name,
            filter,
            sort,
            first,
            after,
            offset,
            near_vector,
            Some(cache),
        )
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
        self.resolve_cached(uid, field_name, None)
    }

    fn resolve_with_cache(
        &self,
        uid: u64,
        field_name: &str,
        cache: &RequestCache,
    ) -> Option<Value> {
        self.resolve_cached(uid, field_name, Some(cache))
    }

    fn find_uid(&self, index_name: &str, value: &str) -> Option<u64> {
        let key = Codec::encode_unique_index_key(index_name, value);
        match self.storage.get(&self.db_name, &key) {
            Ok(Some(bytes)) if bytes.len() == 8 => Some(BigEndian::read_u64(&bytes)),
            _ => None,
        }
    }

    fn create_node(
        &self,
        type_name: &str,
        fields: std::collections::HashMap<String, Value>,
        uniques: &[String],
        inverses: &[crate::engine::resolver::InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
        vector_config: Option<&crate::engine::resolver::VectorConfig>,
    ) -> Result<u64, String> {
        let op_start = std::time::Instant::now();
        let start = std::time::SystemTime::now();
        let since_the_epoch = start
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards");
        let uid = since_the_epoch.as_nanos() as u64;

        if let Some(vec_data) = vector_config
            .and_then(|config| fields.get(&config.field))
            .and_then(Self::extract_vector)
        {
            let storage = self.storage.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = storage.put_vector(uid, vec_data) {
                    eprintln!("Background Vector Insert Error (UID {}): {}", uid, e);
                }
            });
        }

        let payload: std::collections::HashMap<String, serde_json::Value> = fields
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    serde_json::to_value(v).unwrap_or(serde_json::Value::Null),
                )
            })
            .collect();

        self.create_node_internal(
            type_name,
            uid,
            payload,
            uniques,
            inverses,
            search_fields,
            crate::realtime::bus::MutationSource::Local,
            None,
        )?;

        let elapsed = op_start.elapsed();
        if crate::debug_logging() && elapsed.as_secs() >= 1 {
            println!(
                "SLOW: create_node for {} took {:.2}s",
                type_name,
                elapsed.as_secs_f64()
            );
        }
        Ok(uid)
    }

    fn scan_nodes(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
    ) -> Vec<u64> {
        self.scan_nodes_internal(
            type_name,
            filter,
            sort,
            first,
            after,
            offset,
            uniques,
            near_vector,
            query_metadata,
            None,
        )
    }

    fn scan_nodes_with_cache(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
        cache: &RequestCache,
    ) -> Vec<u64> {
        self.scan_nodes_internal(
            type_name,
            filter,
            sort,
            first,
            after,
            offset,
            uniques,
            near_vector,
            query_metadata,
            Some(cache),
        )
    }

    fn count_nodes(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
    ) -> usize {
        self.count_nodes_internal(
            type_name,
            filter,
            uniques,
            near_vector,
            query_metadata,
            None,
        )
    }

    fn count_nodes_with_cache(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
        cache: &RequestCache,
    ) -> usize {
        self.count_nodes_internal(
            type_name,
            filter,
            uniques,
            near_vector,
            query_metadata,
            Some(cache),
        )
    }

    fn update_node(
        &self,
        type_name: &str,
        uid: u64,
        fields: std::collections::HashMap<String, Value>,
        uniques: &[String],
        inverses: &[crate::engine::resolver::InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
        vector_config: Option<&crate::engine::resolver::VectorConfig>,
    ) -> Result<(), String> {
        let op_start = std::time::Instant::now();

        // Automatic embedding generation was removed with the local model backend.
        if let Some(config) = vector_config {
            // HNSW Update
            if let Some(val) = fields.get(&config.field) {
                if let Value::List(list) = val {
                    let vec_data: Vec<f64> = list
                        .iter()
                        .filter_map(|v| match v {
                            Value::Number(n) => n.as_f64(),
                            _ => None,
                        })
                        .collect();
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

        let payload: std::collections::HashMap<String, serde_json::Value> = fields
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    serde_json::to_value(v).unwrap_or(serde_json::Value::Null),
                )
            })
            .collect();

        let result = self.update_node_internal(
            type_name,
            uid,
            payload,
            uniques,
            inverses,
            search_fields,
            crate::realtime::bus::MutationSource::Local,
            None,
        );

        let elapsed = op_start.elapsed();
        if crate::debug_logging() && elapsed.as_secs() >= 1 {
            println!(
                "SLOW: update_node for {} uid={} took {:.2}s",
                type_name,
                uid,
                elapsed.as_secs_f64()
            );
        }
        result
    }

    fn delete_node(
        &self,
        type_name: &str,
        uid: u64,
        uniques: &[String],
        inverses: &[crate::engine::resolver::InverseInfo],
        search_fields: &std::collections::HashMap<String, Vec<String>>,
    ) -> Result<(), String> {
        self.delete_node_internal(
            type_name,
            uid,
            uniques,
            inverses,
            search_fields,
            crate::realtime::bus::MutationSource::Local,
            None,
        )
    }

    fn node_exists(&self, type_name: &str, uid: u64) -> bool {
        let type_key = Codec::encode_type_index_key(type_name, uid);
        self.storage
            .contains_key(&self.db_name, &type_key)
            .unwrap_or(false)
    }

    fn get_node_type(&self, uid: u64) -> Option<String> {
        let type_key = Codec::encode_data_key(uid, "_type");
        if let Ok(Some(bytes)) = self.storage.get(&self.db_name, &type_key) {
            let payload = if bytes.len() > 16 {
                &bytes[16..]
            } else {
                &bytes
            };
            if let Ok(Value::String(s)) = serde_json::from_slice(payload) {
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
        self.storage.flush().map_err(|e| e.to_string())
    }

    fn compact(&self) -> Result<u64, String> {
        self.storage.compact().map_err(|e| e.to_string())
    }

    fn needs_compaction(&self) -> bool {
        self.storage.needs_compaction()
    }
}
