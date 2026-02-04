use std::sync::Arc;
use crate::sync::network_layer::NetworkLayer;
use crate::bridge::fjall_resolver::FjallResolver;
use zenoh::config::Config;
use crate::engine::resolver::Resolver;
use crate::config::ZenohConfig;
use crate::sync::reconciliation::SyncMessage;
use tokio::time::{interval, Duration};
use tokio::sync::RwLock;

pub struct SyncManager {
    network: Arc<NetworkLayer>,
    resolver: Arc<FjallResolver>,
    prefix: String,
    schema: Arc<RwLock<Arc<crate::engine::schema::Schema>>>,
    cache: Arc<crate::engine::cache::QueryCache>,
}

impl SyncManager {
    pub async fn new(
        resolver: Arc<FjallResolver>,
        config: ZenohConfig,
        schema: Arc<RwLock<Arc<crate::engine::schema::Schema>>>,
        cache: Arc<crate::engine::cache::QueryCache>,
    ) -> anyhow::Result<Self> {
        let mut z_config = Config::default(); 
        
        match config.mode.as_str() {
            "client" => { z_config.insert_json5("mode/client", "true").map_err(|e| anyhow::anyhow!(e))?; },
            "peer" => {},
            _ => { eprintln!("Unknown Zenoh mode: {}. Defaulting to Peer.", config.mode); }
        }

        if !config.connect.is_empty() {
             let json = serde_json::to_string(&config.connect)?;
             z_config.insert_json5("connect/endpoints", &json).map_err(|e| anyhow::anyhow!(e))?;
        }
        if !config.listen.is_empty() {
             let json = serde_json::to_string(&config.listen)?;
             z_config.insert_json5("listen/endpoints", &json).map_err(|e| anyhow::anyhow!(e))?;
        }

        let network = Arc::new(NetworkLayer::new(z_config).await?);
        
        Ok(Self {
            network,
            resolver,
            prefix: config.prefix,
            schema,
            cache,
        })
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        // 1. Outbound Bridge
        let bus = self.resolver.subscribe_events(); 
        let receiver = bus.subscribe(); 
        self.network.start_bridge(receiver, self.prefix.clone()).await;
        
        // 2. Inbound Listener
        let resolver = self.resolver.clone();
        let cache_listener = self.cache.clone();
        self.network.start_listener(self.prefix.clone(), move |event| {
             if let Err(e) = resolver.apply_remote_mutation(event) {
                 eprintln!("Failed to apply remote mutation: {}", e);
             } else {
                 cache_listener.invalidate();
             }
        }).await?;

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
                    
                    println!("DEBUG Schema Check: len={}, has_health={}, has_todo={}, has_user={}, is_default={}", 
                        current_sdl.len(), has_health, has_todo, has_user, is_default_schema);
                    
                    if is_default_schema {
                        println!("DEBUG: Requesting schema from peers...");
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
                        println!("DEBUG: Sending Gossip. Local Count: {}", fp.count);
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

        // 4. Sync Listener (Anti-Entropy + Schema)
        let resolver_sync = self.resolver.clone();
        let network_sync = self.network.clone();
        let prefix_sync = self.prefix.clone();
        let schema_sync = self.schema.clone();
        let cache_sync = self.cache.clone();
        
        self.network.start_sync_listener(self.prefix.clone(), move |msg| {
             let resolver = resolver_sync.clone();
             let network = network_sync.clone();
             let prefix = prefix_sync.clone();
             let schema_lock = schema_sync.clone();
             let cache = cache_sync.clone();
             
             tokio::spawn(async move {
                 match msg {
                     // --- SCHEMA SYNC ---
                     SyncMessage::RequestSchema => {
                         let schema = schema_lock.read().await;
                         let sdl = schema.source_sdl();
                         // Only share schema if it has user-defined types (not just default Health)
                         let has_user_types = sdl.contains("Todo") || sdl.contains("User") || 
                             sdl.contains("Post") || sdl.contains("Article");
                         println!("DEBUG: RequestSchema received, has_user_types={}", has_user_types);
                         if has_user_types {
                             let resp = SyncMessage::SchemaResponse(sdl);
                             if let Ok(payload) = serde_json::to_vec(&resp) {
                                 let key = format!("{}/sync/resp_schema", prefix);
                                 let _ = network.publish(&key, &payload).await;
                             }
                         }
                     },
                     SyncMessage::SchemaResponse(new_sdl) => {
                         let mut lock = schema_lock.write().await;
                         let current_sdl = lock.source_sdl();
                         if current_sdl != new_sdl {
                             println!("Sync: Received new schema. Applying...");
                             // Create new resolver instance. self.resolver is Arc<FjallResolver>.
                             // FjallResolver is Clone (cheap, holds Arc<Storage>).
                             // We need to pass a concrete type T: Resolver.
                             // &FjallResolver implements Resolver? No, FjallResolver does.
                             // So we clone the struct.
                             let new_resolver_instance = resolver.as_ref().clone(); 
                             match crate::engine::schema::Schema::load_with_resolver(&new_sdl, new_resolver_instance) {
                                 Ok(new_schema) => {
                                     *lock = Arc::new(new_schema);
                                     // Persist
                                      let storage_path = "varda_db_data"; 
                                      let schema_file_path = format!("{}/current_schema.graphql", storage_path);
                                      if let Err(e) = tokio::fs::write(&schema_file_path, &new_sdl).await {
                                          eprintln!("Failed to persist schema: {}", e);
                                      } else {
                                          println!("Sync: Schema persisted to {}", schema_file_path);
                                      }
                                      cache.invalidate(); // Invalidate on Schema Change!
                                 },
                                 Err(e) => eprintln!("Sync: Failed to load received schema: {}", e),
                             }
                         }
                     },

                     // --- DATA SYNC ---
                     SyncMessage::Gossip(remote_fp) | SyncMessage::RangeResponse(remote_fp) => {
                         println!("DEBUG: Received Gossip/Range. Remote Count: {}", remote_fp.count);
                         // Check against local
                         if let Ok(local_fp) = resolver.compute_fingerprint_range(&remote_fp.start, &remote_fp.end) {
                             if local_fp.hash != remote_fp.hash {
                                 // println!("Sync: Mismatch in range count={}/{}", local_fp.count, remote_fp.count);
                                 // Decide: Split or Fetch
                                 const THRESHOLD: u64 = 100;
                                 let mid = remote_fp.start.midpoint(&remote_fp.end);
                                 
                                 // Check for granular limit (Infinite loop prevention)
                                 let is_atomic = mid.millis <= remote_fp.start.millis;

                                 if local_fp.count <= THRESHOLD || remote_fp.count <= THRESHOLD || is_atomic {
                                     // Fetch Data
                                     let reason = if is_atomic { "Atomic Range" } else { "Threshold" };
                                     println!("DEBUG: Decision -> FETCH DATA (Reason: {}, Local: {}, Remote: {})", reason, local_fp.count, remote_fp.count);
                                     
                                     let req = SyncMessage::RequestData { start: remote_fp.start, end: remote_fp.end };
                                     if let Ok(payload) = serde_json::to_vec(&req) {
                                         let key = format!("{}/sync/req_data", prefix);
                                         let _ = network.publish(&key, &payload).await;
                                     }
                                 } else {
                                     // Split
                                     println!("DEBUG: Decision -> SPLIT RANGE (Local: {}, Remote: {})", local_fp.count, remote_fp.count);
                                     // Left: Start to Mid
                                     let req1 = SyncMessage::RequestRange { start: remote_fp.start, end: mid };
                                     // Right: Mid to End
                                     let req2 = SyncMessage::RequestRange { start: mid, end: remote_fp.end };
                                     
                                     let key = format!("{}/sync/req_range", prefix);
                                     if let Ok(p1) = serde_json::to_vec(&req1) { let _ = network.publish(&key, &p1).await; }
                                     if let Ok(p2) = serde_json::to_vec(&req2) { let _ = network.publish(&key, &p2).await; }
                                 }
                             }
                         }
                     },
                     SyncMessage::RequestRange { start, end } => {
                         if let Ok(fp) = resolver.compute_fingerprint_range(&start, &end) {
                             let resp = SyncMessage::RangeResponse(fp);
                             if let Ok(payload) = serde_json::to_vec(&resp) {
                                 let key = format!("{}/sync/resp_range", prefix);
                                 let _ = network.publish(&key, &payload).await;
                             }
                         }
                     },
                     SyncMessage::RequestData { start, end } => {
                         if let Ok(items) = resolver.get_history_range(&start, &end) {
                             println!("DEBUG: Handling RequestData. Found {} items. Sending Response.", items.len());
                             let resp = SyncMessage::DataResponse(items);
                             if let Ok(payload) = serde_json::to_vec(&resp) {
                                 let key = format!("{}/sync/resp_data", prefix);
                                 let _ = network.publish(&key, &payload).await;
                             }
                         }
                     },
                     SyncMessage::DataResponse(items) => {
                         println!("DEBUG: Received DataResponse. Applying batch of {} items", items.len());
                         if let Err(e) = resolver.apply_batch(items) {
                             eprintln!("Sync: Failed to apply batch: {}", e);
                         } else {
                             cache.invalidate(); // Invalidate on Sync Data!
                         }
                     }
                 }
             });
        }).await?;
        
        println!("SyncManager: Started (Prefix: {}, Mode: Peer)", self.prefix);
        Ok(())
    }
}
