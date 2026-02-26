// src/storage/blob/data.rs
use axum::{body::Body, response::Response};
use bytes::Bytes;
use std::path::{PathBuf, Path};
use tokio::io::AsyncWriteExt;

use crate::config::VardaConfig;
use super::{errors::VardaStorageError, file_info::FileInfo};

#[async_trait::async_trait]
pub trait DataStorage: Send + Sync {
    async fn prepare(&self) -> Result<(), VardaStorageError>;
    async fn get_contents(&self, file_info: &FileInfo) -> Result<Response<Body>, VardaStorageError>;
    async fn add_bytes(&self, file_info: &mut FileInfo, bytes: Bytes) -> Result<(), VardaStorageError>;
    async fn create_file(&self, file_info: &mut FileInfo) -> Result<String, VardaStorageError>;
    async fn concat_files(&self, file_info: &FileInfo, parts_info: Vec<FileInfo>) -> Result<(), VardaStorageError>;
    async fn remove_file(&self, file_info: &FileInfo) -> Result<(), VardaStorageError>;
    async fn finalize(&self, upload_id: &str, staging_path: &Path) -> Result<(PathBuf, String), VardaStorageError>;
}

pub struct VardaDataStorage {
    pub blobs_dir: PathBuf,
}

impl VardaDataStorage {
    pub fn new(config: &VardaConfig) -> Self {
        let path = config.server.blobs_path.clone().unwrap_or_else(|| "varda_blobs".to_string());
        Self {
            blobs_dir: PathBuf::from(path),
        }
    }

    fn staging_path(&self, upload_id: &str) -> PathBuf {
        self.blobs_dir.join(".staging").join(upload_id)
    }

    fn cas_path(&self, content_hash: &str) -> PathBuf {
        let prefix = &content_hash[0..2];
        self.blobs_dir.join(prefix).join(content_hash)
    }
}

#[async_trait::async_trait]
impl DataStorage for VardaDataStorage {
    async fn prepare(&self) -> Result<(), VardaStorageError> {
        tokio::fs::create_dir_all(&self.blobs_dir).await?;
        tokio::fs::create_dir_all(self.blobs_dir.join(".staging")).await?;
        Ok(())
    }

    async fn get_contents(&self, file_info: &FileInfo) -> Result<Response<Body>, VardaStorageError> {
        let path = if let Some(hash) = &file_info.content_hash {
            self.cas_path(hash)
        } else if let Some(staging) = &file_info.path {
            PathBuf::from(staging)
        } else {
            return Err(VardaStorageError::FileNotFound);
        };

        let file = tokio::fs::File::open(&path).await?;
        let stream = tokio_util::io::ReaderStream::new(file);
        let body = Body::from_stream(stream);
        
        // Simple Content-Type fallback. In a real scenario, evaluate mime type based on file info
        let response = Response::builder()
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .map_err(|e| VardaStorageError::StorageError(e.to_string()))?;
            
        Ok(response)
    }

    async fn add_bytes(&self, file_info: &mut FileInfo, bytes: Bytes) -> Result<(), VardaStorageError> {
        let path = file_info.path.as_ref().ok_or(VardaStorageError::FileNotFound)?;
        
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await?;
            
        file.write_all(&bytes).await?;
        file.flush().await?;
        
        file_info.offset += bytes.len();
        Ok(())
    }

    async fn create_file(&self, file_info: &mut FileInfo) -> Result<String, VardaStorageError> {
        let path = self.staging_path(&file_info.id);
        tokio::fs::File::create(&path).await?;
        
        let path_str = path.to_string_lossy().into_owned();
        file_info.path = Some(path_str.clone());
        Ok(path_str)
    }

    async fn concat_files(&self, file_info: &FileInfo, parts_info: Vec<FileInfo>) -> Result<(), VardaStorageError> {
        let final_path = self.staging_path(&file_info.id);
        let mut final_file = tokio::fs::File::create(&final_path).await?;
        
        for part in parts_info {
            let part_path = part.path.as_ref().ok_or(VardaStorageError::FileNotFound)?;
            let mut part_file = tokio::fs::File::open(part_path).await?;
            tokio::io::copy(&mut part_file, &mut final_file).await?;
        }
        
        final_file.flush().await?;
        Ok(())
    }

    async fn remove_file(&self, file_info: &FileInfo) -> Result<(), VardaStorageError> {
        if let Some(path) = &file_info.path {
            let _ = tokio::fs::remove_file(path).await;
        }
        // In this TUS scope, if it's already in CAS, we rely on Graph metadata to "delete" it
        // Or eventually move to .orphans/. But for incomplete staging files, we just rm them.
        Ok(())
    }

    async fn finalize(&self, _upload_id: &str, staging_path: &Path) -> Result<(PathBuf, String), VardaStorageError> {
        let path = staging_path.to_owned();
        let hash = tokio::task::spawn_blocking(move || {
            let mut file = std::fs::File::open(&path)?;
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher)?;
            Ok::<String, std::io::Error>(hasher.finalize().to_hex().to_string())
        })
        .await
        .map_err(|e| VardaStorageError::StorageError(e.to_string()))??;

        let dest = self.cas_path(&hash);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(staging_path, &dest).await?;
        
        Ok((dest, hash))
    }
}
