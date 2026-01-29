pub mod remote;
pub mod pusher;
pub mod oracle;

#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub enabled: bool,
    pub remote_url: Option<String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            remote_url: None,
        }
    }
}

pub struct SyncManager {
    config: SyncConfig,
    pusher: pusher::Pusher,
    #[allow(dead_code)] // Phase 3 stub
    remote: remote::Remote,
}

impl SyncManager {
    pub fn new(config: SyncConfig) -> Self {
        let remote = remote::Remote::new(config.remote_url.clone());
        let pusher = pusher::Pusher::new(remote.clone());
        
        Self {
            config,
            pusher,
            remote,
        }
    }

    pub async fn start(&self) {
        if !self.config.enabled {
            println!("Sync is DISABLED. Skipping background tasks.");
            return;
        }
        
        println!("Starting Sync Manager...");
        self.pusher.start().await;
    }
}
