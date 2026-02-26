// src/storage/blob/routes/mod.rs
pub mod headers;
pub mod handlers;
pub mod hashes;

use axum::{
    routing::{delete, get, head, patch, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use dashmap::DashMap;

use crate::storage::blob::{
    data::{DataStorage, VardaDataStorage},
    info::{InfoStorage, VardaInfoStorage},
};

pub struct BlobState {
    pub info_storage: Arc<dyn InfoStorage>,
    pub data_storage: Arc<dyn DataStorage>,
    pub upload_locks: Arc<DashMap<String, Arc<Mutex<()>>>>,
    pub server_state: Arc<crate::ServerState>,
}

impl BlobState {
    pub async fn new(config: &crate::config::VardaConfig, server_state: Arc<crate::ServerState>) -> Result<Self, super::errors::VardaStorageError> {
        let storage = server_state.storage.clone();
        let info = VardaInfoStorage::new(storage.clone());
        info.prepare().await?;
        
        let data = VardaDataStorage::new(config);
        data.prepare().await?;
        
        Ok(Self {
            info_storage: Arc::new(info),
            data_storage: Arc::new(data),
            upload_locks: Arc::new(DashMap::new()),
            server_state,
        })
    }
}

pub fn router(state: Arc<BlobState>) -> Router {
    Router::new()
        .route("/", post(handlers::create_file))
        .route("/{id}", get(handlers::get_file))
        .route("/{id}", head(handlers::file_status))
        .route("/{id}", patch(handlers::upload_chunk))
        .route("/{id}", delete(handlers::delete_file))
        .route("/hash/{hash}", get(handlers::get_blob_by_hash))
        .with_state(state)
}
