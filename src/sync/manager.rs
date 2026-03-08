use crate::bridge::sqlite_resolver::SqliteResolver;
use crate::config::ZenohConfig;
use crate::engine::resolver::Resolver;
use crate::sync::network_layer::NetworkLayer;
use crate::sync::reconciliation::SyncMessage;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use zenoh::config::Config;

pub struct SyncManager {
    network: Arc<NetworkLayer>,
    resolver: Arc<SqliteResolver>,
    prefix: String,
    schema: Arc<RwLock<Arc<crate::engine::schema::Schema>>>,
    cache: Arc<crate::engine::cache::QueryCache>,
    remote_append_path: Option<String>,
}

impl SyncManager {
    pub async fn new(
        resolver: Arc<SqliteResolver>,
        config: ZenohConfig,
        remote_append_path: Option<String>,
        schema: Arc<RwLock<Arc<crate::engine::schema::Schema>>>,
        cache: Arc<crate::engine::cache::QueryCache>,
    ) -> anyhow::Result<Self> {
        let mut z_config = Config::default();

        match config.mode.as_str() {
            "client" => {
                z_config
                    .insert_json5("mode/client", "true")
                    .map_err(|e| anyhow::anyhow!(e))?;
            }
            "peer" => {}
            _ => {
                eprintln!("Unknown Zenoh mode: {}. Defaulting to Peer.", config.mode);
            }
        }

        if !config.connect.is_empty() {
            let json = serde_json::to_string(&config.connect)?;
            z_config
                .insert_json5("connect/endpoints", &json)
                .map_err(|e| anyhow::anyhow!(e))?;
        }
        if !config.listen.is_empty() {
            let json = serde_json::to_string(&config.listen)?;
            z_config
                .insert_json5("listen/endpoints", &json)
                .map_err(|e| anyhow::anyhow!(e))?;
        }

        let network = Arc::new(NetworkLayer::new(z_config).await?);

        Ok(Self {
            network,
            resolver,
            prefix: config.prefix,
            schema,
            cache,
            remote_append_path,
        })
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        // Wait for fingerprints to be ready (background rebuild may be in progress)
        println!("SyncManager: Waiting for fingerprints...");
        self.resolver.storage.wait_for_fingerprints();
        println!("SyncManager: Fingerprints ready, starting sync!");

        let node_id = self.resolver.storage.node_id;

        // 1. Outbound Bridge
        let bus = self.resolver.subscribe_events();
        let receiver = bus.subscribe();
        self.network
            .start_bridge(node_id, receiver, self.prefix.clone())
            .await;

        // 2. Inbound Listener
        let resolver = self.resolver.clone();
        let cache_listener = self.cache.clone();
        self.network
            .start_listener(node_id, self.prefix.clone(), move |event| {
                if let Err(e) = resolver.apply_remote_mutation(event) {
                    eprintln!("Failed to apply remote mutation: {}", e);
                } else {
                    cache_listener.invalidate();
                }
            })
            .await?;

        // 3. Sync Gossip Loop (Every 5s)
        let resolver_gossip = self.resolver.clone();
        let network_gossip = self.network.clone();
        let prefix_gossip = self.prefix.clone();
        let schema_gossip = self.schema.clone();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(5));
            loop {
                ticker.tick().await;

                // A. Schema Sync Check (Every gossip tick)
                {
                    let schema = schema_gossip.read().await;
                    let current_sdl = schema.source_sdl();
                    // If we have default schema (Health only), request schema!
                    // Check if schema ONLY has Health type and no other user-defined types
                    // Note: We don't check for Mutation because async-graphql always generates it
                    let has_health = current_sdl.contains("Health");
                    let has_todo = current_sdl.contains("Todo");
                    let has_user = current_sdl.contains("User");
                    // Default schema = has Health, but no real user types
                    let is_default_schema = has_health && !has_todo && !has_user;

                    if crate::debug_logging() {
                        println!("DEBUG Schema Check: len={}, has_health={}, has_todo={}, has_user={}, is_default={}", 
                            current_sdl.len(), has_health, has_todo, has_user, is_default_schema);
                    }

                    if is_default_schema {
                        if crate::debug_logging() {
                            println!("DEBUG: Requesting schema from peers...");
                        }
                        let req = SyncMessage::RequestSchema;
                        if let Ok(payload) = serde_json::to_vec(&req) {
                            let key = format!("{}/sync/req_schema", prefix_gossip);
                            let _ = network_gossip.publish(&key, &payload).await;
                        }
                    }
                }

                // B. Data Sync
                match resolver_gossip.compute_fingerprint() {
                    Ok(fp) => {
                        if crate::debug_logging() {
                            println!("DEBUG: Sending Gossip. Local Count: {}", fp.count);
                        }
                        let key = format!("{}/sync/gossip", prefix_gossip);
                        let msg = SyncMessage::Gossip(fp);
                        if let Ok(payload) = serde_json::to_vec(&msg) {
                            if let Err(e) = network_gossip.publish(&key, &payload).await {
                                eprintln!("Gossip Publish Error: {}", e);
                            }
                        }
                    }
                    Err(e) => eprintln!("Fingerprint error: {}", e),
                }
            }
        });

        // 4. Sync Worker (Serialized Processing with Rate Limit)
        let (sync_tx, mut sync_rx) = tokio::sync::mpsc::channel::<SyncMessage>(1000);

        // Clones for the worker
        let resolver_worker = self.resolver.clone();
        let network_worker = self.network.clone();
        let prefix_worker = self.prefix.clone();
        let schema_worker = self.schema.clone();
        let cache_worker = self.cache.clone();
        // Get Node ID from Resolver -> Storage
        // self.resolver.storage is Arc<Storage>, Storage has public node_id?
        // Need to check if node_id is accessible. From viewed_file line 367 backend.rs, Storage struct has node_id field?
        // Let's assume yes or use accessor.
        // Checking backend.rs again... line 367 shown earlier didn't show struct def.
        // Assuming public field or method.
        let node_id = self.resolver.storage.node_id;

        tokio::spawn(async move {
            // Rate limit interval (200ms = 5 ops/sec) - Non-realtime
            let mut limiter = interval(Duration::from_millis(200));

            while let Some(msg) = sync_rx.recv().await {
                limiter.tick().await;

                // Process Message Logic (Compute Response) or Forward Request?
                // Wait, the msg in channel is INCOMING logic?
                // NO! The channel contains "SyncMessage::Request..." generated by US to be sent OUT.
                // Ah, the logic in previous step was mixed.
                // The PREVIOUS code spawned a task to PROCESS incoming message AND send response.
                // Wait.
                // The `start_sync_listener` puts INCOMING messages into the channel?
                // `self.network.start_sync_listener(..., move |msg| { tx.send(msg) })`
                // So `sync_rx` consumes INCOMING messages.

                // So inside `match msg`, we are processing INCOMING messages.
                // And we generate OUTGOING messages.
                // Where do we send OUTGOING messages?
                // `network_worker.publish(&key, &payload).await;`
                // We need to wrap payload in Envelope here!

                match msg {
                    // --- SCHEMA SYNC ---
                    SyncMessage::RequestSchema => {
                        let schema = schema_worker.read().await;
                        let sdl = schema.source_sdl();
                        let has_user_types = sdl.contains("Todo") || sdl.contains("User");
                        // println!("DEBUG: RequestSchema received...");
                        if has_user_types {
                            let resp = SyncMessage::SchemaResponse(sdl);
                            let envelope = crate::sync::reconciliation::SyncEnvelope {
                                sender: node_id,
                                message: resp,
                            };
                            if let Ok(payload) = serde_json::to_vec(&envelope) {
                                let key = format!("{}/sync/resp_schema", prefix_worker);
                                let _ = network_worker.publish(&key, &payload).await;
                            }
                        }
                    }
                    SyncMessage::SchemaResponse(new_sdl) => {
                        // ... Schema Apply Logic ...
                        let mut lock = schema_worker.write().await;
                        let current_sdl = lock.source_sdl();
                        if current_sdl != new_sdl {
                            println!("Sync: Received new schema. Applying...");
                            let new_resolver_instance = resolver_worker.as_ref().clone();
                            match crate::engine::schema::Schema::load_with_resolver(
                                &new_sdl,
                                new_resolver_instance,
                            ) {
                                Ok(new_schema) => {
                                    *lock = Arc::new(new_schema);
                                    let storage_path = "varda_db_data";
                                    let schema_file_path =
                                        format!("{}/current_schema.graphql", storage_path);
                                    if let Err(e) =
                                        tokio::fs::write(&schema_file_path, &new_sdl).await
                                    {
                                        eprintln!("Failed to persist schema: {}", e);
                                    }
                                    cache_worker.invalidate();
                                }
                                Err(e) => eprintln!("Sync: Failed to load received schema: {}", e),
                            }
                        }
                    }

                    // --- DATA SYNC ---
                    SyncMessage::Gossip(remote_fp) | SyncMessage::RangeResponse(remote_fp) => {
                        // println!("DEBUG: Received Gossip/Range. Remote Count: {}", remote_fp.count); // Removed spam
                        if let Ok(local_fp) = resolver_worker
                            .compute_fingerprint_range(&remote_fp.start, &remote_fp.end)
                        {
                            if local_fp.hash != remote_fp.hash {
                                const THRESHOLD: u64 = 100;
                                let mid = remote_fp.start.midpoint(&remote_fp.end);
                                let is_atomic = mid.millis <= remote_fp.start.millis;

                                if local_fp.count <= THRESHOLD
                                    || remote_fp.count <= THRESHOLD
                                    || is_atomic
                                {
                                    // Fetch Data
                                    // println!("DEBUG: Decision -> FETCH DATA"); // Reduced spam
                                    let req = SyncMessage::RequestData {
                                        start: remote_fp.start,
                                        end: remote_fp.end,
                                    };
                                    let envelope = crate::sync::reconciliation::SyncEnvelope {
                                        sender: node_id,
                                        message: req,
                                    };
                                    if let Ok(payload) = serde_json::to_vec(&envelope) {
                                        let key = format!("{}/sync/req_data", prefix_worker);
                                        let _ = network_worker.publish(&key, &payload).await;
                                    }
                                } else {
                                    // Split
                                    // println!("DEBUG: Decision -> SPLIT RANGE (Local: {}, Remote: {})", local_fp.count, remote_fp.count);
                                    let req1 = SyncMessage::RequestRange {
                                        start: remote_fp.start,
                                        end: mid,
                                    };
                                    let req2 = SyncMessage::RequestRange {
                                        start: mid,
                                        end: remote_fp.end,
                                    };

                                    let key = format!("{}/sync/req_range", prefix_worker);

                                    let env1 = crate::sync::reconciliation::SyncEnvelope {
                                        sender: node_id,
                                        message: req1,
                                    };
                                    if let Ok(p1) = serde_json::to_vec(&env1) {
                                        let _ = network_worker.publish(&key, &p1).await;
                                    }

                                    let env2 = crate::sync::reconciliation::SyncEnvelope {
                                        sender: node_id,
                                        message: req2,
                                    };
                                    if let Ok(p2) = serde_json::to_vec(&env2) {
                                        let _ = network_worker.publish(&key, &p2).await;
                                    }
                                }
                            }
                        }
                    }
                    SyncMessage::RequestRange { start, end } => {
                        if let Ok(fp) = resolver_worker.compute_fingerprint_range(&start, &end) {
                            let resp = SyncMessage::RangeResponse(fp);
                            let envelope = crate::sync::reconciliation::SyncEnvelope {
                                sender: node_id,
                                message: resp,
                            };
                            if let Ok(payload) = serde_json::to_vec(&envelope) {
                                let key = format!("{}/sync/resp_range", prefix_worker);
                                let _ = network_worker.publish(&key, &payload).await;
                            }
                        }
                    }
                    SyncMessage::RequestData { start, end } => {
                        if let Ok(items) = resolver_worker.get_history_range(&start, &end) {
                            let resp = SyncMessage::DataResponse(items);
                            let envelope = crate::sync::reconciliation::SyncEnvelope {
                                sender: node_id,
                                message: resp,
                            };
                            if let Ok(payload) = serde_json::to_vec(&envelope) {
                                let key = format!("{}/sync/resp_data", prefix_worker);
                                let _ = network_worker.publish(&key, &payload).await;
                            }
                        }
                    }
                    SyncMessage::DataResponse(items) => {
                        if crate::debug_logging() {
                            println!(
                                "DEBUG: Received DataResponse. Applying batch of {} items",
                                items.len()
                            );
                        }
                        if let Err(e) = resolver_worker.apply_batch(items) {
                            eprintln!("Sync: Failed to apply batch: {}", e);
                        } else {
                            cache_worker.invalidate();
                        }
                    }
                }
            }
        });

        // 5. Sync Listener (Producer)
        let listener_node_id = self.resolver.storage.node_id;
        self.network
            .start_sync_listener(self.prefix.clone(), listener_node_id, move |msg| {
                let tx = sync_tx.clone();
                tokio::spawn(async move {
                    if let Err(_e) = tx.send(msg).await {
                        // eprintln!("Sync Queue Error (Dropping Msg): {}", e);
                    }
                });
            })
            .await?;

        // 6. Backup Worker
        if let Some(path) = self.remote_append_path.clone() {
            let network_backup = self.network.clone();
            let prefix_backup = self.prefix.clone();

            println!("SyncManager: Starting Backup Worker (Path: {})", path);

            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                // Open file in append mode
                let file_res = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .await;

                match file_res {
                    Ok(mut f) => {
                        // Subscribe to all ops
                        let key = format!("{}/ops/**", prefix_backup);
                        match network_backup.subscribe(&key).await {
                            Ok(sub) => {
                                while let Ok(sample) = sub.recv_async().await {
                                    let payload = sample.payload().to_bytes();
                                    if let Err(e) = f.write_all(&payload).await {
                                        eprintln!("Backup Write Error: {}", e);
                                    }
                                    if let Err(e) = f.write_all(b"\n").await {
                                        eprintln!("Backup Write Error (Newline): {}", e);
                                    }
                                    if let Err(e) = f.flush().await {
                                        eprintln!("Backup Flush Error: {}", e);
                                    }
                                }
                            }
                            Err(e) => eprintln!("Backup Subscribe Error: {}", e),
                        }
                    }
                    Err(e) => eprintln!("Failed to open backup file '{}': {}", path, e),
                }
            });
        }

        println!("SyncManager: Started (Prefix: {}, Mode: Peer)", self.prefix);
        Ok(())
    }
}
