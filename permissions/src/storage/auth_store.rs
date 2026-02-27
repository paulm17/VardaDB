use fjall::Keyspace;

use crate::storage::tuple::{RelationTuple, Subject};
use crate::storage::attribute::AttrValue;

/// Fjall-backed storage for AuthZ Data
#[derive(Clone)]
pub struct AuthStore {
    pub tuples: Keyspace,
    pub attributes: Keyspace,
}

impl AuthStore {
    pub fn new(tuples: Keyspace, attributes: Keyspace) -> Self {
        Self {
            tuples,
            attributes,
        }
    }

    /// Tuple Key: {entity_type}\x00{entity_id}\x00{relation}\x00{subject_type}\x00{subject_id}\x00{subject_relation}
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

    /// Tuple Prefix: {entity_type}\x00{entity_id}\x00{relation}\x00
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

    /// Tuple Entity Prefix: {entity_type}\x00{entity_id}\x00
    pub fn build_entity_prefix(entity_type: &str, entity_id: &str) -> Vec<u8> {
        let mut key = String::new();
        key.push_str(entity_type);
        key.push('\x00');
        key.push_str(entity_id);
        key.push('\x00');
        key.into_bytes()
    }

    /// Attribute Key: {entity_type}\x00{entity_id}\x00{attribute}
    pub fn build_attr_key(entity_type: &str, entity_id: &str, attribute: &str) -> Vec<u8> {
        let mut key = String::new();
        key.push_str(entity_type);
        key.push('\x00');
        key.push_str(entity_id);
        key.push('\x00');
        key.push_str(attribute);
        key.into_bytes()
    }

    pub fn insert_tuple(&self, tuple: &RelationTuple) -> fjall::Result<()> {
        let key = Self::build_tuple_key(tuple);
        // Only writing keys. Value can be empty or store snap token metadata later.
        self.tuples.insert(key, b"")
    }

    pub fn insert_attribute(&self, entity_type: &str, entity_id: &str, attribute: &str, value: &AttrValue) -> fjall::Result<()> {
        let key = Self::build_attr_key(entity_type, entity_id, attribute);
        let val = serde_json::to_vec(value).unwrap();
        self.attributes.insert(key, val)
    }

    pub fn get_subjects(&self, entity_type: &str, entity_id: &str, relation: &str) -> Vec<Subject> {
        let prefix = Self::build_tuple_prefix(entity_type, entity_id, relation);
        let iter = self.tuples.prefix(prefix);
        
        let mut subjects = Vec::new();
        for item in iter {
            if let Ok((k, _)) = item.into_inner() {
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
                        }
                    });
                }
            }
        }
        subjects
    }

    pub fn get_attribute(&self, entity_type: &str, entity_id: &str, attribute: &str) -> Option<AttrValue> {
        let key = Self::build_attr_key(entity_type, entity_id, attribute);
        if let Ok(Some(item)) = self.attributes.get(&key) {
            serde_json::from_slice(&*item).ok()
        } else {
            None
        }
    }

    pub fn get_all_tuples_for_entity(&self, entity_type: &str, entity_id: &str) -> Vec<RelationTuple> {
        let prefix = Self::build_entity_prefix(entity_type, entity_id);
        let iter = self.tuples.prefix(prefix);
        
        let mut tuples = Vec::new();
        for item in iter {
            if let Ok((k, _)) = item.into_inner() {
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
        }
        tuples
    }

    pub fn get_all_for_target(&self, target_type: &str, match_subject_entity: &str, match_subject_id: &str) -> Vec<Subject> {
        // Reverse lookup: Given a user, find instances of target_type where they have a relation.
        // Since we don't have a reverse index yet (per spec Phase 3), we do a full scan of tuples for target_type.
        // NOTE: This could be expensive on large datasets.
        
        // Target prefix: {target_type}\x00
        let mut prefix = String::new();
        prefix.push_str(target_type);
        prefix.push('\x00');
        
        let iter = self.tuples.prefix(prefix.as_bytes());
        let mut results = Vec::new();

        for item in iter {
             if let Ok((k, _)) = item.into_inner() {
                let key_str = String::from_utf8_lossy(&k);
                let parts: Vec<&str> = key_str.split('\x00').collect();
                if parts.len() >= 5 {
                    let subject_type = parts[3];
                    let subject_id = parts[4];
                    let entity_id = parts[1];
                     // Assuming simple match for now. No subject relation in reverse lookup yet.
                    if subject_type == match_subject_entity && subject_id == match_subject_id {
                         results.push(Subject {
                             entity: target_type.to_string(),
                             id: entity_id.to_string(),
                         });
                    }
                }
             }
        }
        
        // Deduplicate
        let mut unique_results = Vec::new();
        for r in results {
            if !unique_results.contains(&r) {
                unique_results.push(r);
            }
        }
        unique_results
    }
}
