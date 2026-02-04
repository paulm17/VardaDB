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

    pub async fn start_bridge(&self, mut receiver: tokio::sync::broadcast::Receiver<MutationEvent>, prefix: String) {
        let session = self.session.clone();
        tokio::spawn(async move {
            println!("Element: Zenoh Bridge Started (Prefix: {})", prefix);
            use crate::realtime::bus::MutationSource;
            while let Ok(event) = receiver.recv().await {
                if event.source != MutationSource::Local {
                    continue; 
                }
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
    
    pub async fn start_listener<F>(&self, prefix: String, callback: F) -> Result<()> 
    where F: Fn(MutationEvent) + Send + Sync + 'static 
    {
        let key = format!("{}/**", prefix);
        let subscriber = self.session.declare_subscriber(&key).await.map_err(|e| anyhow::anyhow!(e))?;
        
        tokio::spawn(async move {
            println!("Element: Zenoh Listener Started (Key: {})", key);
            while let Ok(sample) = subscriber.recv_async().await {
                 let key_str = sample.key_expr().as_str();
                 println!("Zenoh Listener Raw Key: {}", key_str);
                 if key_str.contains("/sync/") {
                     continue;
                 }
                 let payload = sample.payload().to_bytes();
                 match serde_json::from_slice::<MutationEvent>(&payload) {
                     Ok(event) => {
                         println!("Zenoh: Received Event (Src: {:?}, Type: {}, UID: {})", event.source, event.type_name, event.uid);
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

    pub async fn start_sync_listener<F>(&self, prefix: String, callback: F) -> Result<()> 
    where F: Fn(crate::sync::reconciliation::SyncMessage) + Send + Sync + 'static 
    {
        let key = format!("{}/sync/**", prefix);
        let subscriber = self.session.declare_subscriber(&key).await.map_err(|e| anyhow::anyhow!(e))?;
        
        tokio::spawn(async move {
            println!("Element: Zenoh Sync Listener Started (Key: {})", key);
            while let Ok(sample) = subscriber.recv_async().await {
                 let payload = sample.payload().to_bytes();
                 if let Ok(msg) = serde_json::from_slice::<crate::sync::reconciliation::SyncMessage>(&payload) {
                      callback(msg);
                 }
            }
        });
        Ok(())
    }
}
