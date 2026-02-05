use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
pub struct VardaConfig {
    pub server: ServerConfig,
    pub zenoh: ZenohConfig,
    #[serde(default = "default_jobs_config")]
    pub jobs: JobsConfig,
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
