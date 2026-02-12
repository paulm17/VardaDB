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
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct VardaClawConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub workers: usize,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct LLMConfig {
    pub openai_api_key: Option<String>,
    pub model_default: Option<String>,
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
