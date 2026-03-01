use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
pub struct VardaConfig {
    pub server: ServerConfig,
    pub zenoh: ZenohConfig,
    #[serde(default)]
    pub remote_append: RemoteAppendConfig,
    #[serde(default = "default_jobs_config")]
    pub jobs: JobsConfig,
    #[serde(default)]
    pub llm: LLMConfig,
    #[serde(default)]
    pub vardaclaw: VardaClawConfig,
    #[serde(default)]
    pub r2: R2Config,
    pub auth: Option<auth::config::AuthConfig>,
    #[serde(default)]
    pub planner: PlannerConfig,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PlannerConfig {
    #[serde(default = "default_planner_enabled")]
    pub enabled: bool,
    #[serde(default = "default_planner_mode")]
    pub mode: String,
    #[serde(default = "default_planner_max_depth")]
    pub max_depth: usize,
    #[serde(default = "default_planner_max_estimated_cost")]
    pub max_estimated_cost: f64,
    #[serde(default = "default_planner_max_actual_cost")]
    pub max_actual_cost: f64,
    #[serde(default = "default_planner_default_list_size")]
    pub default_list_size: i32,
}

fn default_planner_enabled() -> bool { true }
fn default_planner_mode() -> String { "enforce".to_string() }
fn default_planner_max_depth() -> usize { 15 }
fn default_planner_max_estimated_cost() -> f64 { 1000.0 }
fn default_planner_max_actual_cost() -> f64 { 2000.0 }
fn default_planner_default_list_size() -> i32 { 20 }

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            enabled: default_planner_enabled(),
            mode: default_planner_mode(),
            max_depth: default_planner_max_depth(),
            max_estimated_cost: default_planner_max_estimated_cost(),
            max_actual_cost: default_planner_max_actual_cost(),
            default_list_size: default_planner_default_list_size(),
        }
    }
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct R2Config {
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub endpoint_url: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct VardaClawConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub workers: usize,
}

#[derive(Deserialize, Debug, Clone)]
pub struct LLMConfig {
    #[serde(default = "default_llm_provider")]
    pub provider: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    pub draft_model: Option<String>,
    #[serde(default = "default_llm_port")]
    pub port: u16,
    #[serde(default = "default_draft_tokens")]
    pub num_draft_tokens: usize,
    pub openai_api_key: Option<String>,
    pub llama_server_path: Option<String>,
}

fn default_llm_provider() -> String { "ollama".to_string() }
fn default_llm_model() -> String { "llama3".to_string() }
fn default_llm_port() -> u16 { 11434 }
fn default_draft_tokens() -> usize { 5 }

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            provider: default_llm_provider(),
            model: default_llm_model(),
            draft_model: None,
            port: default_llm_port(),
            num_draft_tokens: default_draft_tokens(),
            openai_api_key: None,
            llama_server_path: None,
        }
    }
}

fn default_jobs_config() -> JobsConfig {
    JobsConfig { workers: default_workers() }
}

#[derive(Deserialize, Debug, Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub storage_path: String,
    pub schema_path: Option<String>,
    pub node_id: Option<u64>,
    #[serde(default)]
    pub is_mcp: bool,
    pub blobs_path: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ZenohConfig {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub connect: Vec<String>,
    #[serde(default)]
    pub listen: Vec<String>,
    #[serde(default = "default_prefix")]
    pub prefix: String,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct RemoteAppendConfig {
    pub path: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct JobsConfig {
    #[serde(default = "default_workers")]
    pub workers: usize,
}

fn default_workers() -> usize {
    2
}

fn default_mode() -> String {
    "peer".to_string()
}

fn default_prefix() -> String {
    "varda/ops".to_string()
}

impl VardaConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: VardaConfig = toml::from_str(&content)?;
        Ok(config)
    }
}

impl Default for VardaConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            zenoh: ZenohConfig::default(),
            remote_append: RemoteAppendConfig::default(),
            jobs: JobsConfig::default(),
            llm: LLMConfig::default(),
            vardaclaw: VardaClawConfig::default(),
            r2: R2Config::default(),
            auth: None, // Auth is disabled by default
            planner: PlannerConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8000,
            storage_path: "varda_db_data".to_string(),
            schema_path: None,
            node_id: None,
            is_mcp: false,
            blobs_path: None,
        }
    }
}

impl Default for ZenohConfig {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            connect: vec![],
            listen: vec![],
            prefix: default_prefix(),
        }
    }
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            workers: default_workers(),
        }
    }
}
