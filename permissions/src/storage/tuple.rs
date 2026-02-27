use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub entity: String,
    pub id: String,
}

impl Subject {
    pub fn from_str(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.trim().split(':').collect();
        if parts.len() != 2 {
            None
        } else {
            Some(Subject {
                entity: parts[0].to_string(),
                id: parts[1].to_string(),
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationTuple {
    pub entity_type: String,
    pub entity_id: String,
    pub relation: String,
    pub subject_type: String,
    pub subject_id: String,
    pub subject_relation: Option<String>,
}

impl RelationTuple {
    pub fn into_subject(&self) -> Subject {
        Subject {
            entity: self.subject_type.clone(),
            id: if let Some(ref rel) = self.subject_relation {
                format!("{}#{}", self.subject_id, rel)
            } else {
                self.subject_id.clone()
            },
        }
    }
}
