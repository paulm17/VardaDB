// src/storage/blob/r2_sync.rs
use crate::{config::VardaConfig, ServerState};
use opendal::{services::S3, Operator};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

pub struct R2SyncWorker {
    config: VardaConfig,
    state: Arc<ServerState>,
    operator: Option<Operator>,
}

impl R2SyncWorker {
    pub async fn new(config: &VardaConfig, state: Arc<ServerState>) -> Self {
        let operator = if let (Some(ak), Some(sk), Some(ep), Some(region), Some(bucket)) = (
            &config.r2.access_key,
            &config.r2.secret_key,
            &config.r2.endpoint_url,
            &config.r2.region,
            &config.r2.bucket,
        ) {
            let builder = S3::default()
                .root("/")
                .bucket(bucket)
                .endpoint(ep)
                .region(region)
                .access_key_id(ak)
                .secret_access_key(sk);

            match Operator::new(builder) {
                Ok(op) => {
                    tracing::info!("R2/S3 OpenDAL operator initialized successfully");
                    Some(op.finish())
                }
                Err(e) => {
                    tracing::error!("Failed to initialize R2 OpenDAL operator: {}", e);
                    None
                }
            }
        } else {
            tracing::warn!("R2 Settings incomplete in config. Backup will be disabled.");
            None
        };

        Self {
            config: config.clone(),
            state,
            operator,
        }
    }

    pub async fn start(&self) {
        if self.operator.is_none() {
            return;
        }

        // This worker loops indefinitely looking for UploadQueueEntry rows with status = 'PENDING'
        loop {
            // Wait first, to not immediately spin on startup
            sleep(Duration::from_secs(60)).await;

            let resolver = crate::bridge::sqlite_resolver::SqliteResolver::with_bus(
                self.state.storage.clone(),
                self.state.event_bus.clone(),
            );

            use crate::engine::resolver::Resolver;
            use async_graphql::{Name, Value};
            use indexmap::IndexMap;
            use std::collections::HashMap;
            use std::path::PathBuf;

            let mut eq_map = IndexMap::new();
            eq_map.insert(Name::new("eq"), Value::String("PENDING".to_string()));

            let mut filter = HashMap::new();
            filter.insert("status".to_string(), Value::Object(eq_map));

            let uids = resolver.scan_nodes(
                "UploadQueueEntry",
                filter,
                HashMap::new(),
                Some(100),
                None,
                None,
                &["id".to_string(), "tusId".to_string()],
                None,
            );

            if !uids.is_empty() {
                tracing::info!("R2 Sync Worker found {} pending uploads", uids.len());
            }

            for uid in uids {
                // Get fileRefId
                let file_ref_id = match resolver.resolve(uid, "fileRefId") {
                    Some(Value::String(s)) => s,
                    _ => continue,
                };

                // Find FileRef UID
                let file_uid = match resolver.find_uid("FileRef.id", &file_ref_id) {
                    Some(u) => u,
                    None => continue,
                };

                // Get contentHash
                let content_hash = match resolver.resolve(file_uid, "contentHash") {
                    Some(Value::String(s)) => s,
                    _ => continue,
                };

                if content_hash.len() < 2 {
                    continue;
                }

                let blobs_path = self
                    .config
                    .server
                    .blobs_path
                    .clone()
                    .unwrap_or_else(|| "varda_blobs".to_string());
                let prefix = &content_hash[0..2];
                let file_path = PathBuf::from(&blobs_path).join(prefix).join(&content_hash);

                if let Ok(bytes) = tokio::fs::read(&file_path).await {
                    let remote_path = format!("{}/{}", prefix, content_hash);

                    if let Some(op) = &self.operator {
                        match op.write(&remote_path, bytes).await {
                            Ok(_) => {
                                // Update Status to COMPLETED
                                let mut update_fields = HashMap::new();
                                update_fields.insert(
                                    "status".to_string(),
                                    Value::String("COMPLETED".to_string()),
                                );

                                let _ = resolver.update_node(
                                    "UploadQueueEntry",
                                    uid,
                                    update_fields,
                                    &["id".to_string(), "tusId".to_string()],
                                    &[],
                                    &HashMap::new(),
                                    None,
                                );
                                tracing::info!("Backed up {} to R2", content_hash);
                            }
                            Err(e) => tracing::error!("Failed to upload to R2: {}", e),
                        }
                    }
                } else {
                    tracing::warn!("Local file not found for R2 backup: {:?}", file_path);
                }
            }
        }
    }
}
