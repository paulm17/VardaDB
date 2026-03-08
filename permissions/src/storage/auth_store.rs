use crate::storage::attribute::AttrValue;
use crate::storage::tuple::{RelationTuple, Subject};
use std::sync::Arc;

/// A key-value store trait that abstracts the underlying storage engine.
pub trait KvStore: Send + Sync {
    fn kv_insert(&self, key: &[u8], value: &[u8]) -> Result<(), String>;
    fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String>;
    fn kv_remove(&self, key: &[u8]) -> Result<(), String>;
    fn kv_prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)>;
}

/// Storage for AuthZ Data (Tuples and Attributes)
#[derive(Clone)]
pub struct AuthStore {
    pub tuples: Arc<dyn KvStore>,
    pub attributes: Arc<dyn KvStore>,
}

impl AuthStore {
    pub fn new(tuples: Arc<dyn KvStore>, attributes: Arc<dyn KvStore>) -> Self {
        Self { tuples, attributes }
    }

    pub fn build_tuple_key(tuple: &RelationTuple) -> Vec<u8> {
        let mut key = String::new();
        key.push_str(&tuple.entity_type);
        key.push('\x00');
        key.push_str(&tuple.entity_id);
        key.push('\x00');
        key.push_str(&tuple.relation);
        key.push('\x00');
        key.push_str(&tuple.subject_type);
        key.push('\x00');
        key.push_str(&tuple.subject_id);
        key.push('\x00');
        if let Some(ref sr) = tuple.subject_relation {
            key.push_str(sr);
        }
        key.into_bytes()
    }

    pub fn build_tuple_prefix(entity_type: &str, entity_id: &str, relation: &str) -> Vec<u8> {
        let mut key = String::new();
        key.push_str(entity_type);
        key.push('\x00');
        key.push_str(entity_id);
        key.push('\x00');
        key.push_str(relation);
        key.push('\x00');
        key.into_bytes()
    }

    pub fn build_entity_prefix(entity_type: &str, entity_id: &str) -> Vec<u8> {
        let mut key = String::new();
        key.push_str(entity_type);
        key.push('\x00');
        key.push_str(entity_id);
        key.push('\x00');
        key.into_bytes()
    }

    pub fn build_attr_key(entity_type: &str, entity_id: &str, attribute: &str) -> Vec<u8> {
        let mut key = String::new();
        key.push_str(entity_type);
        key.push('\x00');
        key.push_str(entity_id);
        key.push('\x00');
        key.push_str(attribute);
        key.into_bytes()
    }

    pub fn insert_tuple(&self, tuple: &RelationTuple) -> Result<(), String> {
        let key = Self::build_tuple_key(tuple);
        self.tuples.kv_insert(&key, b"")
    }

    pub fn insert_attribute(
        &self,
        entity_type: &str,
        entity_id: &str,
        attribute: &str,
        value: &AttrValue,
    ) -> Result<(), String> {
        let key = Self::build_attr_key(entity_type, entity_id, attribute);
        let val = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        self.attributes.kv_insert(&key, &val)
    }

    pub fn get_subjects(&self, entity_type: &str, entity_id: &str, relation: &str) -> Vec<Subject> {
        let prefix = Self::build_tuple_prefix(entity_type, entity_id, relation);
        let mut subjects = Vec::new();
        for (k, _) in self.tuples.kv_prefix(&prefix) {
            let key_str = String::from_utf8_lossy(&k);
            let parts: Vec<&str> = key_str.split('\x00').collect();
            if parts.len() >= 5 {
                let subject_type = parts[3];
                let subject_id = parts[4];
                let subject_relation = if parts.len() > 5 && !parts[5].is_empty() {
                    Some(parts[5].to_string())
                } else {
                    None
                };
                subjects.push(Subject {
                    entity: subject_type.to_string(),
                    id: if let Some(sr) = subject_relation {
                        format!("{}#{}", subject_id, sr)
                    } else {
                        subject_id.to_string()
                    },
                });
            }
        }
        subjects
    }

    pub fn get_attribute(
        &self,
        entity_type: &str,
        entity_id: &str,
        attribute: &str,
    ) -> Option<AttrValue> {
        let key = Self::build_attr_key(entity_type, entity_id, attribute);
        if let Ok(Some(item)) = self.attributes.kv_get(&key) {
            serde_json::from_slice(&item).ok()
        } else {
            None
        }
    }

    pub fn get_all_tuples_for_entity(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Vec<RelationTuple> {
        let prefix = Self::build_entity_prefix(entity_type, entity_id);
        let mut tuples = Vec::new();
        for (k, _) in self.tuples.kv_prefix(&prefix) {
            let key_str = String::from_utf8_lossy(&k);
            let parts: Vec<&str> = key_str.split('\x00').collect();
            if parts.len() >= 5 {
                tuples.push(RelationTuple {
                    entity_type: parts[0].to_string(),
                    entity_id: parts[1].to_string(),
                    relation: parts[2].to_string(),
                    subject_type: parts[3].to_string(),
                    subject_id: parts[4].to_string(),
                    subject_relation: if parts.len() > 5 && !parts[5].is_empty() {
                        Some(parts[5].to_string())
                    } else {
                        None
                    },
                });
            }
        }
        tuples
    }

    pub fn get_all_for_target(
        &self,
        target_type: &str,
        match_subject_entity: &str,
        match_subject_id: &str,
    ) -> Vec<Subject> {
        let mut prefix = String::new();
        prefix.push_str(target_type);
        prefix.push('\x00');
        let mut results = Vec::new();
        for (k, _) in self.tuples.kv_prefix(prefix.as_bytes()) {
            let key_str = String::from_utf8_lossy(&k);
            let parts: Vec<&str> = key_str.split('\x00').collect();
            if parts.len() >= 5 {
                let subject_type = parts[3];
                let subject_id = parts[4];
                let entity_id = parts[1];
                if subject_type == match_subject_entity && subject_id == match_subject_id {
                    results.push(Subject {
                        entity: target_type.to_string(),
                        id: entity_id.to_string(),
                    });
                }
            }
        }
        let mut unique_results = Vec::new();
        for r in results {
            if !unique_results.contains(&r) {
                unique_results.push(r);
            }
        }
        unique_results
    }
}
