use crate::engine::resolver::{RequestCache, Resolver};
use crate::storage::backend::Storage;
use crate::storage::codec::Codec;
use crate::storage::tantivy_search::FieldBoost;
use crate::storage::timestamp::Timestamp;
use async_graphql::Value;
use byteorder::{BigEndian, ByteOrder};
use std::sync::Arc;

use crate::realtime::bus::{EventBus, MutationEvent, MutationType};

#[derive(Clone)]
pub struct RedbResolver {
    pub storage: Arc<Storage>,
    pub bus: EventBus,
    pub db_name: String,
}

impl RedbResolver {
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
        let upper = crate::storage::redb_backend::compute_prefix_upper_bound(
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

    fn load_object_fields(&self, uid: u64) -> std::collections::HashMap<String, Value> {
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

    fn load_related_uids(&self, parent_uid: u64, field_name: &str) -> Vec<u64> {
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

    fn related_uids_cached(
        &self,
        parent_uid: u64,
        field_name: &str,
        cache: Option<&RequestCache>,
    ) -> Vec<u64> {
        if let Some(cache) = cache {
            if let Some(uids) = cache.get_related_uids(parent_uid, field_name) {
                return uids;
            }
        }

        let uids = self.load_related_uids(parent_uid, field_name);
        if let Some(cache) = cache {
            cache.insert_related_uids(parent_uid, field_name, uids.clone());
        }
        uids
    }

    fn load_resolved_value(&self, uid: u64, field_name: &str) -> Option<Value> {
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

    fn compare_optional_values(a: &Option<Value>, b: &Option<Value>) -> std::cmp::Ordering {
        match (a, b) {
            (Some(Value::Number(na)), Some(Value::Number(nb))) => na
                .as_f64()
                .partial_cmp(&nb.as_f64())
                .unwrap_or(std::cmp::Ordering::Equal),
            (Some(Value::String(sa)), Some(Value::String(sb))) => sa.cmp(sb),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        }
    }

    fn sort_uids_by_field(
        &self,
        uids: &mut [u64],
        field_name: &str,
        asc: bool,
        cache: Option<&RequestCache>,
    ) {
        let sort_values: std::collections::HashMap<u64, Option<Value>> = uids
            .iter()
            .copied()
            .map(|uid| (uid, self.resolve_cached(uid, field_name, cache)))
            .collect();

        uids.sort_by(|a, b| {
            let cmp = Self::compare_optional_values(
                sort_values.get(a).unwrap_or(&None),
                sort_values.get(b).unwrap_or(&None),
            );
            if asc {
                cmp
            } else {
                cmp.reverse()
            }
        });
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

    fn sorted_index_scan(
        &self,
        type_name: &str,
        field: &str,
        asc: bool,
        filter: &std::collections::HashMap<String, Value>,
        filter_im: &indexmap::IndexMap<async_graphql::Name, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        candidate_set: Option<&std::collections::HashSet<u64>>,
        cache: Option<&RequestCache>,
    ) -> Option<Vec<u64>> {
        let descending = !asc;
        let prefix = Codec::encode_order_index_prefix(type_name, field, descending);
        let (main_ks, _) = self.storage.get_database(&self.db_name)?;

        if main_ks.count_prefix(&prefix).ok()? == 0 {
            self.rebuild_order_index_for_field(type_name, field).ok()?;
            if main_ks.count_prefix(&prefix).ok()? == 0 {
                return None;
            }
        }

        let mut matched = Vec::new();
        let mut seen_after = after.is_none();
        let mut skipped = 0usize;

        for (key, _val) in main_ks.prefix(&prefix) {
            if !key.starts_with(&prefix) {
                break;
            }
            let Some(uid) = Codec::decode_order_index_uid(&key) else {
                continue;
            };

            if let Some(candidates) = candidate_set {
                if !candidates.contains(&uid) {
                    continue;
                }
            }

            if !seen_after {
                if after.as_deref() == Some(&uid.to_string()) {
                    seen_after = true;
                }
                continue;
            }

            if !filter.is_empty() && !self.check_filter_recursive_cached(uid, filter_im, cache) {
                continue;
            }

            if skipped < offset.unwrap_or(0) {
                skipped += 1;
                continue;
            }

            matched.push(uid);
            if let Some(limit) = first {
                if matched.len() >= limit {
                    break;
                }
            }
        }

        Some(matched)
    }

    fn rebuild_order_index_for_field(&self, type_name: &str, field: &str) -> Result<(), String> {
        let (main_ks, _) = self
            .storage
            .get_database(&self.db_name)
            .ok_or_else(|| format!("Database not found: {}", self.db_name))?;

        let type_prefix = Codec::encode_type_prefix(type_name);
        let upper = crate::storage::redb_backend::compute_prefix_upper_bound(&type_prefix)
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
                    main_ks.batch_insert_on_txn(conn, key, &[])?;
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

    /// Create a RedbResolver with a shared EventBus.
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

    /// Convert a serde_json::Value to a FilterTarget for filter pushdown.
    fn json_to_filter_target(val: &Value) -> crate::storage::redb_backend::FilterTarget {
        use crate::storage::redb_backend::FilterTarget;
        match val {
            Value::String(s) => FilterTarget::Text(s.clone()),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    FilterTarget::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    FilterTarget::Real(f)
                } else {
                    FilterTarget::Null
                }
            }
            Value::Boolean(b) => FilterTarget::Boolean(*b),
            Value::Null => FilterTarget::Null,
            Value::Enum(e) => FilterTarget::Text(e.to_string()),
            other => FilterTarget::Text(serde_json::to_string(other).unwrap_or_default()),
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
        _strategy: &str,
    ) -> Result<(), String> {
        self.storage
            .search_engine
            .index_document(&self.db_name, uid, field, text)
            .map_err(|e| e.to_string())
    }

    fn remove_term_index(
        &self,
        uid: u64,
        field: &str,
        _text: &str,
        _strategy: &str,
    ) -> Result<(), String> {
        self.storage
            .search_engine
            .remove_document(&self.db_name, uid, field)
            .map_err(|e| e.to_string())
    }

    /// BM25 ranked search backed by Tantivy.
    ///
    /// * `strategy` – `"term"` (no stemming) or `"fulltext"` (Porter stemmer).
    /// * `require_all` – `true` → AND semantics; `false` → OR semantics.
    /// * `fuzzy_distance` – Optional Levenshtein distance (0-2) for fuzzy matching.
    /// * `phrase_slop` – Optional slop for phrase queries (only used for quoted queries).
    pub fn search_text_bm25(
        &self,
        query: &str,
        field: &str,
        strategy: &str,
        k: usize,
        require_all: bool,
        fuzzy_distance: Option<u8>,
        phrase_slop: Option<u32>,
    ) -> Vec<(u64, f64)> {
        self.storage.search_engine.search_bm25(
            &self.db_name,
            query,
            field,
            strategy,
            k,
            require_all,
            fuzzy_distance,
            phrase_slop,
        )
    }

    /// Multi-field BM25 search with per-field boost weights.
    ///
    /// * `fields` – Slice of `FieldBoost` entries specifying field name and boost.
    /// * `strategy` – `"term"` (no stemming) or `"fulltext"` (Porter).
    /// * `require_all` – `true` → AND semantics; `false` → OR semantics.
    /// * `fuzzy_distance` – Optional Levenshtein distance (0-2) for fuzzy matching.
    /// * `phrase_slop` – Optional slop for phrase queries.
    pub fn search_text_bm25_multi(
        &self,
        query: &str,
        fields: &[FieldBoost],
        strategy: &str,
        k: usize,
        require_all: bool,
        fuzzy_distance: Option<u8>,
        phrase_slop: Option<u32>,
    ) -> Vec<(u64, f64)> {
        self.storage.search_engine.search_bm25_multi(
            &self.db_name,
            query,
            fields,
            strategy,
            k,
            require_all,
            fuzzy_distance,
            phrase_slop,
        )
    }

    /// Hybrid search combining BM25 (Tantivy) and ANN (usearch) via
    /// Reciprocal Rank Fusion (RRF, k=60).
    ///
    /// * `alpha` – Weight for vector search (0.0 = all BM25, 1.0 = all vector).
    ///             Defaults to 0.5 when None.
    pub fn search_hybrid(
        &self,
        text_query: &str,
        field: &str,
        vector: &[f64],
        k: usize,
        require_all: bool,
        alpha: Option<f32>,
    ) -> Vec<(u64, f64)> {
        let alpha = alpha.unwrap_or(0.5);
        let text_weight = 1.0 - alpha as f64;
        let vector_weight = alpha as f64;

        // BM25 results (over-fetch then fuse)
        let text_results = self.search_text_bm25(
            text_query,
            field,
            "fulltext",
            k * 2,
            require_all,
            None,
            None,
        );

        // ANN results (over-fetch then fuse)
        let vec_f32: Vec<f32> = vector.iter().map(|&x| x as f32).collect();
        let vec_results = self
            .storage
            .vector_engine
            .search(&self.db_name, &vec_f32, k * 2);

        // Reciprocal Rank Fusion with weighted alpha
        let mut scores: std::collections::HashMap<u64, f64> = std::collections::HashMap::new();
        for (rank, (uid, _)) in text_results.iter().enumerate() {
            *scores.entry(*uid).or_default() += text_weight / (60.0 + rank as f64 + 1.0);
        }
        for (rank, (uid, _)) in vec_results.iter().enumerate() {
            *scores.entry(*uid).or_default() += vector_weight / (60.0 + rank as f64 + 1.0);
        }

        let mut fused: Vec<(u64, f64)> = scores.into_iter().collect();
        fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        fused.truncate(k);
        fused
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

    fn get_candidates(
        &self,
        type_name: &str,
        filter: &std::collections::HashMap<String, Value>,
        uniques: &[String],
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
    ) -> Option<std::collections::HashSet<u64>> {
        // println!("Scan: get_candidates called for {} with filter {:?}", type_name, filter);
        let mut candidates: Option<std::collections::HashSet<u64>> = None;
        let type_meta = query_metadata.get(type_name);

        for (field, condition) in filter {
            // Skip logical operators — these are handled by check_filter_recursive, not here
            if field == "and" || field == "or" || field == "not" {
                continue;
            }
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
                                    candidates = Some(
                                        current.into_iter().filter(|u| set.contains(u)).collect(),
                                    );
                                } else {
                                    candidates = Some(set);
                                }
                                continue;
                            }
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
                                candidates =
                                    Some(current.into_iter().filter(|u| set.contains(u)).collect());
                            } else {
                                candidates = Some(set);
                            }
                            continue; // Optimized this field
                        }
                    }
                }

                // SQL filter pushdown for eq on non-unique fields
                if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                    let filter_val = Self::json_to_filter_target(val);
                    let uids = main_ks.filter_by_field_value(type_name, field, "=", filter_val);
                    // Even if empty, this is the definitive answer from SQL pushdown
                    let set: std::collections::HashSet<u64> = uids.into_iter().collect();
                    if let Some(current) = candidates {
                        candidates = Some(current.intersection(&set).copied().collect());
                    } else {
                        candidates = Some(set);
                    }
                    continue;
                }
            }

            // 2. SQL Filter Pushdown for comparison operators (gt, lt, ge, le, ne, contains, in)
            if let Value::Object(map) = condition {
                let mut handled_by_pushdown = false;

                // Check for simple scalar comparison ops that can be pushed to SQL
                for (op, target) in map.iter() {
                    let sql_op = match op.as_str() {
                        "gt" => Some(">"),
                        "lt" => Some("<"),
                        "ge" => Some(">="),
                        "le" => Some("<="),
                        "ne" => Some("!="),
                        _ => None,
                    };

                    if let Some(sql_op) = sql_op {
                        if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                            let filter_val = Self::json_to_filter_target(target);
                            let uids =
                                main_ks.filter_by_field_value(type_name, field, sql_op, filter_val);
                            let set: std::collections::HashSet<u64> = uids.into_iter().collect();
                            if let Some(current) = candidates {
                                candidates = Some(current.intersection(&set).copied().collect());
                            } else {
                                candidates = Some(set);
                            }
                            handled_by_pushdown = true;
                        }
                    }

                    // Handle "contains" operator
                    if op.as_str() == "contains" {
                        if let Value::String(substr) = target {
                            if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                                let uids = main_ks.filter_by_field_contains(field, substr);
                                let set: std::collections::HashSet<u64> =
                                    uids.into_iter().collect();
                                if let Some(current) = candidates {
                                    candidates =
                                        Some(current.intersection(&set).copied().collect());
                                } else {
                                    candidates = Some(set);
                                }
                                handled_by_pushdown = true;
                            }
                        }
                    }

                    // Handle "in" operator
                    if op.as_str() == "in" {
                        if let Value::List(list) = target {
                            let target_values: Vec<crate::storage::redb_backend::FilterTarget> =
                                list.iter()
                                    .map(|v| Self::json_to_filter_target(v))
                                    .collect();
                            if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                                let uids =
                                    main_ks.filter_by_field_in(type_name, field, &target_values);
                                let set: std::collections::HashSet<u64> =
                                    uids.into_iter().collect();
                                if let Some(current) = candidates {
                                    candidates =
                                        Some(current.intersection(&set).copied().collect());
                                } else {
                                    candidates = Some(set);
                                }
                                handled_by_pushdown = true;
                            }
                        }
                    }
                }

                if handled_by_pushdown {
                    continue;
                }

                let is_operator_map = map.keys().any(|k| {
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
                        "fuzzy",
                        "phrase",
                    ]
                    .contains(&k.as_str())
                });

                if !is_operator_map {
                    if let Some(meta) = type_meta {
                        if let Some(target_type) = meta.relations.get(field) {
                            if let Some(inverse) =
                                meta.inverses.iter().find(|info| info.field == *field)
                            {
                                let nested_filter: std::collections::HashMap<String, Value> = map
                                    .iter()
                                    .map(|(k, v)| (k.to_string(), v.clone()))
                                    .collect();
                                let child_uniques = query_metadata
                                    .get(target_type)
                                    .map(|m| m.uniques.as_slice())
                                    .unwrap_or(&[]);
                                let child_uids = self.scan_nodes_internal(
                                    target_type,
                                    nested_filter,
                                    std::collections::HashMap::new(),
                                    None,
                                    None,
                                    None,
                                    child_uniques,
                                    None,
                                    None,
                                    query_metadata,
                                    None,
                                );

                                let mut field_uids = std::collections::HashSet::new();
                                for child_uid in child_uids {
                                    for parent_uid in
                                        self.load_related_uids(child_uid, &inverse.inverse_field)
                                    {
                                        field_uids.insert(parent_uid);
                                    }
                                }

                                if let Some(current) = candidates {
                                    candidates = Some(
                                        current
                                            .into_iter()
                                            .filter(|u| field_uids.contains(u))
                                            .collect(),
                                    );
                                } else {
                                    candidates = Some(field_uids);
                                }
                                continue;
                            }
                        }
                    }
                }
                // Handle "allofterms" — all terms must match (AND), no stemming
                if let Some(Value::String(terms_str)) = map.get("allofterms") {
                    let field_uids: std::collections::HashSet<u64> =
                        if let Some(Value::List(fields_list)) = map.get("fields") {
                            let field_boosts: Vec<FieldBoost> = fields_list
                                .iter()
                                .filter_map(|v| {
                                    if let Value::Object(fmap) = v {
                                        let field_name = fmap.get("field").and_then(|fv| {
                                            if let Value::String(s) = fv {
                                                Some(s.clone())
                                            } else {
                                                None
                                            }
                                        })?;
                                        let boost = fmap
                                            .get("boost")
                                            .and_then(|bv| {
                                                if let Value::Number(n) = bv {
                                                    n.as_f64().map(|f| f as f32)
                                                } else {
                                                    Some(1.0)
                                                }
                                            })
                                            .unwrap_or(1.0);
                                        Some(FieldBoost {
                                            field: field_name,
                                            boost,
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if field_boosts.is_empty() {
                                self.search_text_bm25(
                                    terms_str, field, "term", 100_000, true, None, None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            } else {
                                self.search_text_bm25_multi(
                                    terms_str,
                                    &field_boosts,
                                    "term",
                                    100_000,
                                    true,
                                    None,
                                    None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            }
                        } else {
                            self.search_text_bm25(
                                terms_str, field, "term", 100_000, true, None, None,
                            )
                            .into_iter()
                            .map(|(uid, _)| uid)
                            .collect()
                        };

                    if let Some(current) = candidates {
                        candidates = Some(
                            current
                                .into_iter()
                                .filter(|u| field_uids.contains(u))
                                .collect(),
                        );
                    } else {
                        candidates = Some(field_uids);
                    }
                }

                // Handle "anyofterms" — any term matches (OR), no stemming
                if let Some(Value::String(terms_str)) = map.get("anyofterms") {
                    let field_uids: std::collections::HashSet<u64> =
                        if let Some(Value::List(fields_list)) = map.get("fields") {
                            let field_boosts: Vec<FieldBoost> = fields_list
                                .iter()
                                .filter_map(|v| {
                                    if let Value::Object(fmap) = v {
                                        let field_name = fmap.get("field").and_then(|fv| {
                                            if let Value::String(s) = fv {
                                                Some(s.clone())
                                            } else {
                                                None
                                            }
                                        })?;
                                        let boost = fmap
                                            .get("boost")
                                            .and_then(|bv| {
                                                if let Value::Number(n) = bv {
                                                    n.as_f64().map(|f| f as f32)
                                                } else {
                                                    Some(1.0)
                                                }
                                            })
                                            .unwrap_or(1.0);
                                        Some(FieldBoost {
                                            field: field_name,
                                            boost,
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if field_boosts.is_empty() {
                                self.search_text_bm25(
                                    terms_str, field, "term", 100_000, false, None, None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            } else {
                                self.search_text_bm25_multi(
                                    terms_str,
                                    &field_boosts,
                                    "term",
                                    100_000,
                                    false,
                                    None,
                                    None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            }
                        } else {
                            self.search_text_bm25(
                                terms_str, field, "term", 100_000, false, None, None,
                            )
                            .into_iter()
                            .map(|(uid, _)| uid)
                            .collect()
                        };

                    if let Some(current) = candidates {
                        candidates = Some(
                            current
                                .into_iter()
                                .filter(|u| field_uids.contains(u))
                                .collect(),
                        );
                    } else {
                        candidates = Some(field_uids);
                    }
                }

                // Handle "alloftext" — all terms must match (AND), Porter stemming
                if let Some(Value::String(terms_str)) = map.get("alloftext") {
                    let field_uids: std::collections::HashSet<u64> =
                        if let Some(Value::List(fields_list)) = map.get("fields") {
                            let field_boosts: Vec<FieldBoost> = fields_list
                                .iter()
                                .filter_map(|v| {
                                    if let Value::Object(fmap) = v {
                                        let field_name = fmap.get("field").and_then(|fv| {
                                            if let Value::String(s) = fv {
                                                Some(s.clone())
                                            } else {
                                                None
                                            }
                                        })?;
                                        let boost = fmap
                                            .get("boost")
                                            .and_then(|bv| {
                                                if let Value::Number(n) = bv {
                                                    n.as_f64().map(|f| f as f32)
                                                } else {
                                                    Some(1.0)
                                                }
                                            })
                                            .unwrap_or(1.0);
                                        Some(FieldBoost {
                                            field: field_name,
                                            boost,
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if field_boosts.is_empty() {
                                self.search_text_bm25(
                                    terms_str, field, "fulltext", 100_000, true, None, None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            } else {
                                self.search_text_bm25_multi(
                                    terms_str,
                                    &field_boosts,
                                    "fulltext",
                                    100_000,
                                    true,
                                    None,
                                    None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            }
                        } else {
                            self.search_text_bm25(
                                terms_str, field, "fulltext", 100_000, true, None, None,
                            )
                            .into_iter()
                            .map(|(uid, _)| uid)
                            .collect()
                        };

                    if let Some(current) = candidates {
                        candidates = Some(
                            current
                                .into_iter()
                                .filter(|u| field_uids.contains(u))
                                .collect(),
                        );
                    } else {
                        candidates = Some(field_uids);
                    }
                }

                // Handle "anyoftext" — any term matches (OR), Porter stemming
                if let Some(Value::String(terms_str)) = map.get("anyoftext") {
                    let field_uids: std::collections::HashSet<u64> =
                        if let Some(Value::List(fields_list)) = map.get("fields") {
                            let field_boosts: Vec<FieldBoost> = fields_list
                                .iter()
                                .filter_map(|v| {
                                    if let Value::Object(fmap) = v {
                                        let field_name = fmap.get("field").and_then(|fv| {
                                            if let Value::String(s) = fv {
                                                Some(s.clone())
                                            } else {
                                                None
                                            }
                                        })?;
                                        let boost = fmap
                                            .get("boost")
                                            .and_then(|bv| {
                                                if let Value::Number(n) = bv {
                                                    n.as_f64().map(|f| f as f32)
                                                } else {
                                                    Some(1.0)
                                                }
                                            })
                                            .unwrap_or(1.0);
                                        Some(FieldBoost {
                                            field: field_name,
                                            boost,
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if field_boosts.is_empty() {
                                self.search_text_bm25(
                                    terms_str, field, "fulltext", 100_000, false, None, None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            } else {
                                self.search_text_bm25_multi(
                                    terms_str,
                                    &field_boosts,
                                    "fulltext",
                                    100_000,
                                    false,
                                    None,
                                    None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            }
                        } else {
                            self.search_text_bm25(
                                terms_str, field, "fulltext", 100_000, false, None, None,
                            )
                            .into_iter()
                            .map(|(uid, _)| uid)
                            .collect()
                        };

                    if let Some(current) = candidates {
                        candidates = Some(
                            current
                                .into_iter()
                                .filter(|u| field_uids.contains(u))
                                .collect(),
                        );
                    } else {
                        candidates = Some(field_uids);
                    }
                }

                // Handle "fuzzy" — fuzzy matching with Levenshtein distance
                if let Some(Value::Object(fuzzy_map)) = map.get("fuzzy") {
                    if let Some(Value::String(terms_str)) = fuzzy_map.get("terms") {
                        let distance = match fuzzy_map.get("distance") {
                            Some(Value::Number(n)) => n.as_i64().map(|d| d as u8).unwrap_or(1),
                            _ => 1,
                        };

                        let field_uids: std::collections::HashSet<u64> =
                            if let Some(Value::List(fields_list)) = map.get("fields") {
                                let field_boosts: Vec<FieldBoost> = fields_list
                                    .iter()
                                    .filter_map(|v| {
                                        if let Value::Object(fmap) = v {
                                            let field_name = fmap.get("field").and_then(|fv| {
                                                if let Value::String(s) = fv {
                                                    Some(s.clone())
                                                } else {
                                                    None
                                                }
                                            })?;
                                            let boost = fmap
                                                .get("boost")
                                                .and_then(|bv| {
                                                    if let Value::Number(n) = bv {
                                                        n.as_f64().map(|f| f as f32)
                                                    } else {
                                                        Some(1.0)
                                                    }
                                                })
                                                .unwrap_or(1.0);
                                            Some(FieldBoost {
                                                field: field_name,
                                                boost,
                                            })
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if field_boosts.is_empty() {
                                    self.search_text_bm25(
                                        terms_str,
                                        field,
                                        "term",
                                        100_000,
                                        false,
                                        Some(distance),
                                        None,
                                    )
                                    .into_iter()
                                    .map(|(uid, _)| uid)
                                    .collect()
                                } else {
                                    self.search_text_bm25_multi(
                                        terms_str,
                                        &field_boosts,
                                        "term",
                                        100_000,
                                        false,
                                        Some(distance),
                                        None,
                                    )
                                    .into_iter()
                                    .map(|(uid, _)| uid)
                                    .collect()
                                }
                            } else {
                                self.search_text_bm25(
                                    terms_str,
                                    field,
                                    "term",
                                    100_000,
                                    false,
                                    Some(distance),
                                    None,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            };

                        if let Some(current) = candidates {
                            candidates = Some(
                                current
                                    .into_iter()
                                    .filter(|u| field_uids.contains(u))
                                    .collect(),
                            );
                        } else {
                            candidates = Some(field_uids);
                        }
                    }
                }

                // Handle "phrase" — phrase/proximity search
                if let Some(Value::Object(phrase_map)) = map.get("phrase") {
                    if let Some(Value::String(terms_str)) = phrase_map.get("terms") {
                        let slop = match phrase_map.get("slop") {
                            Some(Value::Number(n)) => n.as_i64().map(|s| s as u32),
                            _ => None,
                        };

                        let quoted_query = format!("\"{}\"", terms_str);
                        let field_uids: std::collections::HashSet<u64> =
                            if let Some(Value::List(fields_list)) = map.get("fields") {
                                let field_boosts: Vec<FieldBoost> = fields_list
                                    .iter()
                                    .filter_map(|v| {
                                        if let Value::Object(fmap) = v {
                                            let field_name = fmap.get("field").and_then(|fv| {
                                                if let Value::String(s) = fv {
                                                    Some(s.clone())
                                                } else {
                                                    None
                                                }
                                            })?;
                                            let boost = fmap
                                                .get("boost")
                                                .and_then(|bv| {
                                                    if let Value::Number(n) = bv {
                                                        n.as_f64().map(|f| f as f32)
                                                    } else {
                                                        Some(1.0)
                                                    }
                                                })
                                                .unwrap_or(1.0);
                                            Some(FieldBoost {
                                                field: field_name,
                                                boost,
                                            })
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if field_boosts.is_empty() {
                                    self.search_text_bm25(
                                        &quoted_query,
                                        field,
                                        "fulltext",
                                        100_000,
                                        true,
                                        None,
                                        slop,
                                    )
                                    .into_iter()
                                    .map(|(uid, _)| uid)
                                    .collect()
                                } else {
                                    self.search_text_bm25_multi(
                                        &quoted_query,
                                        &field_boosts,
                                        "fulltext",
                                        100_000,
                                        true,
                                        None,
                                        slop,
                                    )
                                    .into_iter()
                                    .map(|(uid, _)| uid)
                                    .collect()
                                }
                            } else {
                                self.search_text_bm25(
                                    &quoted_query,
                                    field,
                                    "fulltext",
                                    100_000,
                                    true,
                                    None,
                                    slop,
                                )
                                .into_iter()
                                .map(|(uid, _)| uid)
                                .collect()
                            };

                        if let Some(current) = candidates {
                            candidates = Some(
                                current
                                    .into_iter()
                                    .filter(|u| field_uids.contains(u))
                                    .collect(),
                            );
                        } else {
                            candidates = Some(field_uids);
                        }
                    }
                }
            }
        }
        candidates
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
                                "fuzzy",
                                "phrase",
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
                main.batch_insert_on_txn(conn, &type_key_idx, &[])?;

                let type_val_bytes =
                    serde_json::to_vec(&serde_json::Value::String(type_name.to_string()))?;
                let type_data_key = Codec::encode_data_key(uid, "_type");
                let mut type_val_buf = Vec::with_capacity(16 + type_val_bytes.len());
                type_val_buf.extend_from_slice(&ts_bytes);
                type_val_buf.extend_from_slice(&type_val_bytes);
                main.batch_upsert_lww_on_txn(conn, &type_data_key, &type_val_buf, &ts_bytes)?;

                for (field, value) in &fields {
                    if uniques.contains(field) {
                        let index_pred = format!("{}.{}", type_name, field);
                        let val_str = serde_json::to_string(value)?;
                        let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                        let mut uid_bytes = vec![0u8; 8];
                        BigEndian::write_u64(&mut uid_bytes, uid);
                        main.batch_insert_on_txn(conn, &idx_key, &uid_bytes)?;
                    }

                    let val_bytes = serde_json::to_vec(value)?;
                    let key = Codec::encode_data_key(uid, field);
                    let mut val_buf = Vec::with_capacity(16 + val_bytes.len());
                    val_buf.extend_from_slice(&ts_bytes);
                    val_buf.extend_from_slice(&val_bytes);
                    main.batch_upsert_lww_on_txn(conn, &key, &val_buf, &ts_bytes)?;

                    if let Some(encoded_value) = Self::encode_order_index_value(value, false) {
                        let asc_key = Codec::encode_order_index_key(
                            type_name,
                            field,
                            false,
                            &encoded_value,
                            uid,
                        );
                        main.batch_insert_on_txn(conn, &asc_key, &[])?;
                    }
                    if let Some(encoded_value) = Self::encode_order_index_value(value, true) {
                        let desc_key = Codec::encode_order_index_key(
                            type_name,
                            field,
                            true,
                            &encoded_value,
                            uid,
                        );
                        main.batch_insert_on_txn(conn, &desc_key, &[])?;
                    }
                }

                for (target, inverse_field, is_list) in &deferred_inverses {
                    if *is_list {
                        let edge_key = Codec::encode_edge_key(*target, inverse_field, uid);
                        let reverse_edge_key =
                            Codec::encode_reverse_edge_key(uid, *target, inverse_field);
                        main.batch_insert_on_txn(conn, &edge_key, &[])?;
                        main.batch_insert_on_txn(conn, &reverse_edge_key, &[])?;
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
                    main_clone.batch_insert_on_txn(conn, &type_idx_key, &[])?;

                    // _type data field
                    let type_val =
                        serde_json::to_vec(&serde_json::Value::String(record.type_name.clone()))
                            .map_err(|e| anyhow::anyhow!(e))?;
                    let type_data_key = Codec::encode_data_key(uid, "_type");
                    let mut type_buf = Vec::with_capacity(16 + type_val.len());
                    type_buf.extend_from_slice(&ts_bytes);
                    type_buf.extend_from_slice(&type_val);
                    main_clone.batch_upsert_lww_on_txn(
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
                        main_clone.batch_upsert_lww_on_txn(conn, &key, &val_buf, &ts_bytes)?;

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
                            main_clone.batch_insert_on_txn(conn, &asc_key, &[])?;
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
                            main_clone.batch_insert_on_txn(conn, &desc_key, &[])?;
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
                                        main_clone.batch_insert_on_txn(conn, &edge_key, &[])?;
                                        main_clone.batch_insert_on_txn(
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
                                        main_clone.batch_upsert_lww_on_txn(
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
        self.storage
            .delete_vector(&self.db_name, uid)
            .map_err(|e| e.to_string())?;

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

impl RedbResolver {
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
        let mut uids = self.related_uids_cached(parent_uid, field_name, cache);
        let load_time = start.elapsed();

        if let Some(ref vec) = near_vector {
            let mut uid_dists = Vec::new();
            for uid in &uids {
                if let Some(Value::List(floats)) = self.resolve_cached(*uid, "embedding", cache) {
                    let embed: Vec<f64> = floats
                        .iter()
                        .filter_map(|v| match v {
                            Value::Number(n) => n.as_f64(),
                            _ => None,
                        })
                        .collect();

                    if embed.len() == vec.len() {
                        let dot: f64 = embed.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
                        let norm_a: f64 = embed.iter().map(|a| a * a).sum::<f64>().sqrt();
                        let norm_b: f64 = vec.iter().map(|b| b * b).sum::<f64>().sqrt();

                        if norm_a > 0.0 && norm_b > 0.0 {
                            let sim = dot / (norm_a * norm_b);
                            uid_dists.push((*uid, 1.0 - sim));
                        } else {
                            uid_dists.push((*uid, f64::MAX));
                        }
                    }
                }
            }

            uid_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            uids = uid_dists.into_iter().map(|(u, _)| u).collect();
        }

        if !filter.is_empty() {
            let mut filter_im = indexmap::IndexMap::new();
            for (k, v) in &filter {
                filter_im.insert(async_graphql::Name::new(k), v.clone());
            }
            uids.retain(|uid| self.check_filter_recursive_cached(*uid, &filter_im, cache));
        }

        if !sort.is_empty() {
            if let Some((field, direction)) = sort.iter().next() {
                let asc = match direction {
                    Value::String(s) => s == "ASC",
                    Value::Enum(n) => n.as_str() == "ASC",
                    _ => true,
                };
                self.sort_uids_by_field(&mut uids, field, asc, cache);
            }
        }

        if let Some(cursor_uid_str) = after {
            if let Ok(cursor_uid) = cursor_uid_str.parse::<u64>() {
                if let Some(pos) = uids.iter().position(|u| *u == cursor_uid) {
                    uids = uids.into_iter().skip(pos + 1).collect();
                }
            }
        }

        if let Some(skip_count) = offset {
            if skip_count > 0 {
                uids = uids.into_iter().skip(skip_count).collect();
            }
        }

        if let Some(limit) = first {
            uids.truncate(limit);
        }

        if let Some(cache) = cache {
            self.preload_objects_for_uids(&uids, cache);
        }

        let total = start.elapsed();
        if crate::debug_logging() && total.as_millis() > 10 {
            eprintln!(
                "[RESOLVER] resolve_list {}.{} parent={} base={} filter={} sort={} total={}ms load={}ms near_vector={}",
                self.db_name,
                field_name,
                parent_uid,
                uids.len(),
                !filter.is_empty(),
                !sort.is_empty(),
                total.as_millis(),
                load_time.as_millis(),
                near_vector.is_some(),
            );
        }

        Ok(uids)
    }

    fn scan_nodes_internal(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        sort: std::collections::HashMap<String, Value>,
        first: Option<usize>,
        after: Option<String>,
        offset: Option<usize>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        rrf_alpha: Option<f32>,
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
        cache: Option<&RequestCache>,
    ) -> Vec<u64> {
        let start = std::time::Instant::now();
        let mut uids = Vec::new();
        let mut filter_im = indexmap::IndexMap::new();
        for (k, v) in &filter {
            filter_im.insert(async_graphql::Name::new(k), v.clone());
        }

        let mut text_search: Option<(String, String, String, bool)> = None;
        for (field, val) in &filter {
            if let Value::Object(obj) = val {
                if let Some(Value::String(s)) = obj.get("allofterms") {
                    text_search = Some((field.clone(), "term".to_string(), s.clone(), true));
                    break;
                }
                if let Some(Value::String(s)) = obj.get("anyofterms") {
                    text_search = Some((field.clone(), "term".to_string(), s.clone(), false));
                    break;
                }
                if let Some(Value::String(s)) = obj.get("alloftext") {
                    text_search = Some((field.clone(), "fulltext".to_string(), s.clone(), true));
                    break;
                }
                if let Some(Value::String(s)) = obj.get("anyoftext") {
                    text_search = Some((field.clone(), "fulltext".to_string(), s.clone(), false));
                    break;
                }
            }
        }

        let has_text_search = text_search.is_some();
        let candidate_start = std::time::Instant::now();
        let candidate_set = if near_vector.is_none() && !has_text_search {
            self.get_candidates(type_name, &filter, uniques, query_metadata)
        } else {
            None
        };
        let candidate_time = candidate_start.elapsed();
        let used_candidates = candidate_set.is_some();

        if near_vector.is_none() && !has_text_search && !sort.is_empty() {
            if let Some((field, direction)) = sort.iter().next() {
                let asc = match direction {
                    Value::String(s) => s == "ASC",
                    Value::Enum(n) => n.as_str() == "ASC",
                    _ => true,
                };
                if let Some(sorted_uids) = self.sorted_index_scan(
                    type_name,
                    field,
                    asc,
                    &filter,
                    &filter_im,
                    first,
                    after.clone(),
                    offset,
                    candidate_set.as_ref(),
                    cache,
                ) {
                    return sorted_uids;
                }
            }
        }

        if let Some(ref vec) = near_vector {
            let k = first.unwrap_or(50) * 4;
            let search_results =
                if let Some((field, _strat, query, require_all)) = text_search.clone() {
                    self.search_hybrid(&query, &field, vec, k, require_all, rrf_alpha)
                } else {
                    self.search_vectors(vec, k)
                };

            for (uid, _dist) in search_results {
                if self.node_exists(type_name, uid)
                    && self.get_node_type(uid).as_deref() == Some(type_name)
                    && (filter.is_empty()
                        || self.check_filter_recursive_cached(uid, &filter_im, cache))
                {
                    uids.push(uid);
                }
            }
        } else if let Some((field, strat, query, require_all)) = text_search {
            let k = first.unwrap_or(50) * 4;
            let results = self.search_text_bm25(&query, &field, &strat, k, require_all, None, None);

            for (uid, _score) in results {
                if self.node_exists(type_name, uid)
                    && self.get_node_type(uid).as_deref() == Some(type_name)
                    && self.check_filter_recursive_cached(uid, &filter_im, cache)
                {
                    uids.push(uid);
                }
            }
        } else if let Some(candidates) = candidate_set.clone() {
            use rayon::prelude::*;

            let mut matched_uids: Vec<u64> = candidates
                .par_iter()
                .filter(|uid| {
                    filter.is_empty()
                        || self.check_filter_recursive_cached(**uid, &filter_im, cache)
                })
                .cloned()
                .collect();

            uids.append(&mut matched_uids);
            if sort.is_empty() {
                uids.sort();
            }
        } else {
            let prefix = Codec::encode_type_prefix(type_name);
            let needs_sorting = !sort.is_empty();
            let mut skipped = 0usize;
            let skip_target = offset.unwrap_or(0);

            let start_key = if !needs_sorting {
                if let Some(cursor) = after.clone() {
                    let uid = cursor.parse::<u64>().unwrap_or(0);
                    if uid == u64::MAX {
                        return vec![];
                    }
                    Codec::encode_type_index_key(type_name, uid + 1)
                } else {
                    prefix.clone()
                }
            } else {
                prefix.clone()
            };

            if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                let upper = crate::storage::redb_backend::compute_prefix_upper_bound(&prefix)
                    .expect("valid prefix bounds");
                for (key, _val) in main_ks.range(&start_key, &upper) {
                    if !key.starts_with(&prefix) {
                        break;
                    }
                    if key.len() < 8 {
                        continue;
                    }

                    let uid = BigEndian::read_u64(&key[key.len() - 8..]);
                    if !filter.is_empty()
                        && !self.check_filter_recursive_cached(uid, &filter_im, cache)
                    {
                        continue;
                    }

                    if !needs_sorting && near_vector.is_none() {
                        if skipped < skip_target {
                            skipped += 1;
                            continue;
                        }
                        uids.push(uid);
                        if let Some(limit) = first {
                            if uids.len() >= limit {
                                break;
                            }
                        }
                    } else {
                        uids.push(uid);
                    }
                }
            }
        }

        if !sort.is_empty() {
            if let Some((field, direction)) = sort.iter().next() {
                let asc = match direction {
                    Value::String(s) => s == "ASC",
                    _ => true,
                };
                self.sort_uids_by_field(&mut uids, field, asc, cache);
            }
        } else if near_vector.is_none() && used_candidates {
            uids.sort();
        }

        if let Some(cursor_uid_str) = after {
            if let Ok(cursor_uid) = cursor_uid_str.parse::<u64>() {
                if let Some(pos) = uids.iter().position(|u| *u == cursor_uid) {
                    uids = uids.into_iter().skip(pos + 1).collect();
                }
            }
        }

        if !(sort.is_empty() && near_vector.is_none() && candidate_set.is_none()) {
            if let Some(skip_count) = offset {
                if skip_count > 0 {
                    uids = uids.into_iter().skip(skip_count).collect();
                }
            }
        }

        if let Some(limit) = first {
            uids.truncate(limit);
        }

        if let Some(cache) = cache {
            self.preload_objects_for_uids(&uids, cache);
        }

        let total = start.elapsed();
        if crate::debug_logging() && total.as_millis() > 10 {
            eprintln!(
                "[RESOLVER] scan_nodes {}.{} filter={} sort={} first={:?} offset={:?} candidates={} candidate_ms={} result_count={} total_ms={} near_vector={} text_search={}",
                self.db_name,
                type_name,
                !filter.is_empty(),
                !sort.is_empty(),
                first,
                offset,
                used_candidates,
                candidate_time.as_millis(),
                uids.len(),
                total.as_millis(),
                near_vector.is_some(),
                has_text_search,
            );
        }

        uids
    }

    fn count_nodes_internal(
        &self,
        type_name: &str,
        filter: std::collections::HashMap<String, Value>,
        uniques: &[String],
        near_vector: Option<Vec<f64>>,
        rrf_alpha: Option<f32>,
        query_metadata: &std::collections::HashMap<
            String,
            crate::engine::resolver::QueryTypeMetadata,
        >,
        cache: Option<&RequestCache>,
    ) -> usize {
        let start = std::time::Instant::now();
        let mut filter_im = indexmap::IndexMap::new();
        for (k, v) in &filter {
            filter_im.insert(async_graphql::Name::new(k), v.clone());
        }

        let mut text_search: Option<(String, String, String, bool)> = None;
        for (field, val) in &filter {
            if let Value::Object(obj) = val {
                if let Some(Value::String(s)) = obj.get("allofterms") {
                    text_search = Some((field.clone(), "term".to_string(), s.clone(), true));
                    break;
                }
                if let Some(Value::String(s)) = obj.get("anyofterms") {
                    text_search = Some((field.clone(), "term".to_string(), s.clone(), false));
                    break;
                }
                if let Some(Value::String(s)) = obj.get("alloftext") {
                    text_search = Some((field.clone(), "fulltext".to_string(), s.clone(), true));
                    break;
                }
                if let Some(Value::String(s)) = obj.get("anyoftext") {
                    text_search = Some((field.clone(), "fulltext".to_string(), s.clone(), false));
                    break;
                }
            }
        }

        let has_text_search = text_search.is_some();
        let candidate_start = std::time::Instant::now();
        let candidate_set = if near_vector.is_none() && !has_text_search {
            self.get_candidates(type_name, &filter, uniques, query_metadata)
        } else {
            None
        };
        let candidate_time = candidate_start.elapsed();

        if let Some(ref vec) = near_vector {
            let search_results =
                if let Some((field, _strat, query, require_all)) = text_search.clone() {
                    self.search_hybrid(&query, &field, vec, 10_000, require_all, rrf_alpha)
                } else {
                    self.search_vectors(vec, 10_000)
                };

            let count = search_results
                .into_iter()
                .filter(|(uid, _)| {
                    self.node_exists(type_name, *uid)
                        && self.get_node_type(*uid).as_deref() == Some(type_name)
                        && (filter.is_empty()
                            || self.check_filter_recursive_cached(*uid, &filter_im, cache))
                })
                .count();
            if crate::debug_logging() && start.elapsed().as_millis() > 10 {
                eprintln!(
                    "[RESOLVER] count_nodes {}.{} candidates=false candidate_ms={} count={} total_ms={} near_vector=true text_search={}",
                    self.db_name,
                    type_name,
                    candidate_time.as_millis(),
                    count,
                    start.elapsed().as_millis(),
                    has_text_search,
                );
            }
            return count;
        }

        if let Some((field, strat, query, require_all)) = text_search {
            let count = self
                .search_text_bm25(&query, &field, &strat, 10_000, require_all, None, None)
                .into_iter()
                .filter(|(uid, _)| {
                    self.node_exists(type_name, *uid)
                        && self.get_node_type(*uid).as_deref() == Some(type_name)
                        && self.check_filter_recursive_cached(*uid, &filter_im, cache)
                })
                .count();
            if crate::debug_logging() && start.elapsed().as_millis() > 10 {
                eprintln!(
                    "[RESOLVER] count_nodes {}.{} candidates=false candidate_ms={} count={} total_ms={} near_vector=false text_search=true",
                    self.db_name,
                    type_name,
                    candidate_time.as_millis(),
                    count,
                    start.elapsed().as_millis(),
                );
            }
            return count;
        }

        if let Some(candidates) = candidate_set {
            let count = candidates
                .into_iter()
                .filter(|uid| {
                    filter.is_empty() || self.check_filter_recursive_cached(*uid, &filter_im, cache)
                })
                .count();
            if crate::debug_logging() && start.elapsed().as_millis() > 10 {
                eprintln!(
                    "[RESOLVER] count_nodes {}.{} candidates=true candidate_ms={} count={} total_ms={} near_vector=false text_search=false",
                    self.db_name,
                    type_name,
                    candidate_time.as_millis(),
                    count,
                    start.elapsed().as_millis(),
                );
            }
            return count;
        }

        let prefix = Codec::encode_type_prefix(type_name);
        if filter.is_empty() {
            if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
                return main_ks.count_prefix(&prefix).unwrap_or(0);
            }
            return 0;
        }

        let mut count = 0usize;
        if let Some((main_ks, _)) = self.storage.get_database(&self.db_name) {
            let upper = crate::storage::redb_backend::compute_prefix_upper_bound(&prefix)
                .expect("valid prefix bounds");
            for (key, _val) in main_ks.range(&prefix, &upper) {
                if !key.starts_with(&prefix) {
                    break;
                }
                if key.len() < 8 {
                    continue;
                }

                let uid = BigEndian::read_u64(&key[key.len() - 8..]);
                if self.check_filter_recursive_cached(uid, &filter_im, cache) {
                    count += 1;
                }
            }
        }

        if crate::debug_logging() && start.elapsed().as_millis() > 10 {
            eprintln!(
                "[RESOLVER] count_nodes {}.{} candidates=false candidate_ms={} count={} total_ms={} near_vector=false text_search=false",
                self.db_name,
                type_name,
                candidate_time.as_millis(),
                count,
                start.elapsed().as_millis(),
            );
        }

        count
    }
}

impl Resolver for RedbResolver {
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
        match self.storage.search_vectors(&self.db_name, query, k) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("Vector Search Error: {}", e);
                vec![]
            }
        }
    }

    fn search_hybrid(
        &self,
        text: &str,
        field: &str,
        vector: &[f64],
        k: usize,
        alpha: Option<f32>,
    ) -> Vec<(u64, f64)> {
        self.search_hybrid(text, field, vector, k, false, alpha)
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
            let db_name = self.db_name.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = storage.put_vector(&db_name, uid, vec_data) {
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
        rrf_alpha: Option<f32>,
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
            rrf_alpha,
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
        rrf_alpha: Option<f32>,
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
            rrf_alpha,
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
        rrf_alpha: Option<f32>,
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
            rrf_alpha,
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
        rrf_alpha: Option<f32>,
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
            rrf_alpha,
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
                        let db_name = self.db_name.clone();
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = storage.put_vector(&db_name, uid, vec_data) {
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
