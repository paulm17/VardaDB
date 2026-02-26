// src/storage/blob/file_info.rs
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Information about a TUS file upload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub id: String,
    pub offset: usize,
    pub length: Option<usize>,
    pub path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub deferred_size: bool,
    pub is_partial: bool,
    pub is_final: bool,
    pub parts: Option<Vec<String>>,
    pub storage: String,
    pub metadata: HashMap<String, String>,
    pub content_hash: Option<String>, // BLAKE3 hex hash after finalization
}

impl FileInfo {
    pub fn new(
        file_id: &str,
        length: Option<usize>,
        path: Option<String>,
        storage: String,
        initial_metadata: Option<HashMap<String, String>>,
    ) -> Self {
        let id = String::from(file_id);
        let deferred_size = length.is_none();
        let metadata = initial_metadata.unwrap_or_default();

        Self {
            id,
            path,
            length,
            storage,
            metadata,
            deferred_size,
            offset: 0,
            is_final: false,
            is_partial: false,
            parts: None,
            created_at: Utc::now(),
            content_hash: None,
        }
    }

    pub fn get_filename(&self) -> &str {
        self.metadata.get("filename").unwrap_or(&self.id)
    }

    pub fn get_metadata_string(&self) -> Option<String> {
        let mut pairs: Vec<String> = self
            .metadata
            .iter()
            .map(|(k, v)| {
                use base64::{Engine as _, engine::general_purpose};
                format!("{} {}", k, general_purpose::STANDARD.encode(v))
            })
            .collect();
            
        if pairs.is_empty() {
            None
        } else {
            pairs.sort();
            Some(pairs.join(","))
        }
    }
}
