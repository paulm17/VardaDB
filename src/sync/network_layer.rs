use zenoh::{config::Config, Session}; // Removed prelude
use anyhow::Result;
use std::sync::Arc;
// use tokio::sync::broadcast; // Unused for now
use crate::realtime::bus::MutationEvent;

pub struct NetworkLayer {
    session: Arc<Session>,
    // We might need an outbound sender if we want to channel events to it
    // But for now, we'll expose methods to publish directly.
}

impl NetworkLayer {
    pub async fn new(config: Config) -> Result<Self> {
        let session = zenoh::open(config).await.map_err(|e| anyhow::anyhow!(e))?;
        // Zenoh session is already Arc-like internally or can be cloned cheapily? 
        // 1.0+ Session handles handles its own lifecycle.
        let session = Arc::new(session);
        
        Ok(Self {
            session,
        })
    }

    pub async fn publish(&self, key: &str, payload: &[u8]) -> Result<()> {
        self.session.put(key, payload).await.map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }

    pub async fn subscribe(&self, key: &str) -> Result<zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>> {
        let sub = self.session.declare_subscriber(key).await.map_err(|e| anyhow::anyhow!(e))?;
        Ok(sub)
    }

    pub async fn close(&self) -> Result<()> {
        self.session.close().await.map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }

    pub async fn start_bridge(&self, node_id: u64, mut receiver: tokio::sync::broadcast::Receiver<MutationEvent>, prefix: String) {
        let session = self.session.clone();
        tokio::spawn(async move {
            println!("Element: Zenoh Bridge Started (Prefix: {})", prefix);
            use crate::realtime::bus::MutationSource;
            while let Ok(mut event) = receiver.recv().await {
                if event.source != MutationSource::Local {
                    continue; 
                }
                
                // Rewrite source to Remote before publishing
                // This ensures that:
                // 1. Peers see it as Remote (and process it)
                // 2. We see it as Remote (and ignore it due to dedicated logic in listener)
                event.source = MutationSource::Remote;
                event.node_id = node_id;
                
                let key = format!("{}/{}/{}", prefix, event.type_name, event.uid);
                
                match serde_json::to_vec(&event) {
                    Ok(payload) => {
                        if let Err(e) = session.put(&key, payload).await {
                             eprintln!("Zenoh Put Error: {:?}", e);
                        }
                    }
                    Err(e) => eprintln!("Serialization Error: {:?}", e),
                }
            }
        });
    }
    
    pub async fn start_listener<F>(&self, node_id: u64, prefix: String, callback: F) -> Result<()> 
    where F: Fn(MutationEvent) + Send + Sync + 'static 
    {
        let key = format!("{}/**", prefix);
        let subscriber = self.session.declare_subscriber(&key).await.map_err(|e| anyhow::anyhow!(e))?;
        
        tokio::spawn(async move {
            println!("Element: Zenoh Listener Started (Key: {})", key);
            while let Ok(sample) = subscriber.recv_async().await {
                 let key_str = sample.key_expr().as_str();
                 if crate::debug_logging() {
                     println!("Zenoh Listener Raw Key: {}", key_str);
                 }
                 if key_str.contains("/sync/") {
                     continue;
                 }
                 let payload = sample.payload().to_bytes();
                 match serde_json::from_slice::<MutationEvent>(&payload) {
                     Ok(event) => {
                         if crate::debug_logging() {
                             println!("Zenoh: Received Event (Src: {:?}, Type: {}, UID: {}, NodeId: {})", event.source, event.type_name, event.uid, event.node_id);
                         }
                         if event.node_id == node_id {
                             if crate::debug_logging() {
                                 println!("Zenoh: Ignoring self-published event.");
                             }
                             continue;
                         }
                         if event.source == crate::realtime::bus::MutationSource::Local {
                             if crate::debug_logging() {
                                 println!("Zenoh: Ignoring Local Event execution loop.");
                             }
                             continue;
                         }
                         callback(event);
                     },
                     Err(e) => {
                         eprintln!("Zenoh: Deserialization Error: {} (Payload Len: {})", e, payload.len());
                         if let Ok(s) = std::str::from_utf8(&payload) {
                             eprintln!("Zenoh: Payload: {}", s);
                         }
                     }
                 }
            }
        });
        Ok(())
    }

    pub async fn start_sync_listener<F>(&self, prefix: String, node_id: u64, callback: F) -> Result<()> 
    where F: Fn(crate::sync::reconciliation::SyncMessage) + Send + Sync + 'static 
    {
        let key = format!("{}/sync/**", prefix);
        let subscriber = self.session.declare_subscriber(&key).await.map_err(|e| anyhow::anyhow!(e))?;
        
        tokio::spawn(async move {
            println!("Element: Zenoh Sync Listener Started (Key: {}, Filter Self: {})", key, node_id);
            while let Ok(sample) = subscriber.recv_async().await {
                 let payload = sample.payload().to_bytes();
                 if let Ok(envelope) = serde_json::from_slice::<crate::sync::reconciliation::SyncEnvelope>(&payload) {
                      if envelope.sender != node_id {
                          callback(envelope.message);
                      }
                 } else {
                     // Fallback for legacy/other messages? Or just ignore.
                     // eprintln!("Sync Listener: Failed to deserialize Envelope");
                 }
            }
        });
        Ok(())
    }
}
