use std::sync::Arc;
use std::path::PathBuf;
use tokio::time::{sleep, Duration};
use crate::storage::backend::Storage;
use crate::config::VardaConfig;
use tracing::{info, error, debug};
use vardaclaw::agent::{Agent, AgentConfig as ClawAgentConfig};
use vardaclaw::config::{Config as ClawConfig};
use vardaclaw::memory::MemoryManager;
// use vardaclaw::agent::LLMProvider;

pub struct VardaClawRunner {
    #[allow(dead_code)]
    storage: Arc<Storage>,
    config: VardaConfig,
}

impl VardaClawRunner {
    pub fn new(storage: Arc<Storage>, config: VardaConfig) -> Self {
        Self { storage, config }
    }

    pub async fn run(self) {
        if !self.config.vardaclaw.enabled {
            info!("VardaClaw Runner disabled in config.");
            return;
        }

        info!("VardaClaw Runner started.");

        // Ensure workspace exists
        let storage_path = PathBuf::from(&self.config.server.storage_path);
        let claw_workspace = storage_path.join("vardaclaw_workspace");
        std::fs::create_dir_all(&claw_workspace).ok();
        
        // 1. Setup VardaClaw Config
        let mut claw_config = ClawConfig::default();
        claw_config.memory.workspace = claw_workspace.to_string_lossy().to_string();
        claw_config.agent.default_model = self.config.llm.model.clone();
        
        // Inject API Key from VardaDB config if present
        if let Some(key) = &self.config.llm.openai_api_key {
             claw_config.providers.openai = Some(vardaclaw::config::OpenAIConfig { 
                 api_key: key.clone(), 
                 base_url: "https://api.openai.com/v1".to_string() 
             });
        }

        // 2. Initialize MemoryManager
        let memory = match MemoryManager::new(&claw_config.memory) {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to initialize VardaClaw Memory: {}", e);
                return;
            }
        };

        // 3. Initialize Agent
        let agent_config = ClawAgentConfig {
            model: claw_config.agent.default_model.clone(),
            context_window: claw_config.agent.context_window,
            reserve_tokens: claw_config.agent.reserve_tokens,
        };

        let _agent = match Agent::new(agent_config, &claw_config, memory).await {
            Ok(a) => a,
            Err(e) => {
                error!("Failed to initialize VardaClaw Agent: {}", e);
                return;
            }
        };
        
        debug!("VardaClaw Agent initialized successfully.");

        loop {
            info!("VardaClaw Runner: Heartbeat tick.");
            
            // TODO: Implement actual background logic here.
            // For now, we just prove the loop runs and Agent is alive.
            // Example: Check for scheduled tasks in a specific queue, or maintain agent autonomy.
            
            sleep(Duration::from_secs(10)).await;
        }
    }
}
