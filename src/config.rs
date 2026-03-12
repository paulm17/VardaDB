use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
pub struct VardaConfig {
    pub server: ServerConfig,
    pub zenoh: ZenohConfig,
    #[serde(default)]
    pub remote_append: RemoteAppendConfig,
    #[serde(default)]
    pub llm: LLMConfig,
    #[serde(default)]
    pub r2: R2Config,
    pub auth: Option<auth::config::AuthConfig>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct R2Config {
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub endpoint_url: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
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
    #[serde(default)]
    pub huggingface: HuggingFaceConfig,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct HuggingFaceConfig {
    pub hf_token: Option<String>,
}

fn default_llm_provider() -> String {
    "mlx".to_string()
}
fn default_llm_model() -> String {
    "".to_string()
}
fn default_llm_port() -> u16 {
    8080
}
fn default_draft_tokens() -> usize {
    5
}

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
            huggingface: HuggingFaceConfig::default(),
        }
    }
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
            llm: LLMConfig::default(),
            r2: R2Config::default(),
            auth: None, // Auth is disabled by default
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
