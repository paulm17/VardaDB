// src/storage/blob/info.rs
use std::sync::Arc;
use crate::storage::backend::Storage;
use super::{errors::VardaStorageError, file_info::FileInfo};

#[async_trait::async_trait]
pub trait InfoStorage: Send + Sync {
    async fn prepare(&self) -> Result<(), VardaStorageError>;
    async fn set_info(&self, file_info: &FileInfo, create: bool) -> Result<(), VardaStorageError>;
    async fn get_info(&self, file_id: &str) -> Result<FileInfo, VardaStorageError>;
    async fn remove_info(&self, file_id: &str) -> Result<(), VardaStorageError>;
}

pub struct VardaInfoStorage {
    pub storage: Arc<Storage>,
}

impl VardaInfoStorage {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }
    
    fn key(&self, file_id: &str) -> String {
        format!("tus:info:{}", file_id)
    }
}

#[async_trait::async_trait]
impl InfoStorage for VardaInfoStorage {
    async fn prepare(&self) -> Result<(), VardaStorageError> {
        Ok(())
    }

    async fn set_info(&self, file_info: &FileInfo, _create: bool) -> Result<(), VardaStorageError> {
        let key = self.key(&file_info.id);
        let bytes = bincode::serialize(file_info)
            .map_err(|e| VardaStorageError::StorageError(e.to_string()))?;
            
        self.storage.sys_keyspace.insert(&key, bytes)
            .map_err(|e| VardaStorageError::StorageError(e.to_string()))?;
            
        Ok(())
    }

    async fn get_info(&self, file_id: &str) -> Result<FileInfo, VardaStorageError> {
        let key = self.key(file_id);
            
        if let Some(bytes) = self.storage.sys_keyspace.get(&key).map_err(|e| VardaStorageError::StorageError(e.to_string()))? {
            let file_info: FileInfo = bincode::deserialize(&bytes)
                .map_err(|e| VardaStorageError::StorageError(e.to_string()))?;
            Ok(file_info)
        } else {
            Err(VardaStorageError::FileNotFound)
        }
    }

    async fn remove_info(&self, file_id: &str) -> Result<(), VardaStorageError> {
        let key = self.key(file_id);
            
        self.storage.sys_keyspace.remove(&key)
            .map_err(|e| VardaStorageError::StorageError(e.to_string()))?;
            
        Ok(())
    }
}
