use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;
use crate::config::LLMConfig;
use std::sync::Arc;
use axum::{extract::State, Json};
use serde::Deserialize;

pub struct LlamaServer {
    _process: Option<Child>,
    pub base_url: String,
    pub model: String,
}

impl LlamaServer {
    pub fn start(config: LLMConfig) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let port = config.port;
        
        let server_bin = config.llama_server_path.clone().unwrap_or_else(|| "llama-server".to_string());

        let model_path = if std::path::Path::new(&config.model).exists() {
             std::fs::canonicalize(&config.model)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or(config.model.clone())
        } else {
            // Warn but proceed? Or assume relative?
            config.model.clone()
        };

        let mut args = vec![
            "--model".to_string(),
            model_path,
            "--port".to_string(),
            port.to_string(),
            "--ctx-size".to_string(),
            "8192".to_string(), // Default context size
            "--n-gpu-layers".to_string(),
            "99".to_string(), // Try to offload all to GPU (M-series Mac)
        ];

        // Add draft model args if provided
        if let Some(ref draft_model) = config.draft_model {
            args.push("--model-draft".to_string());
             let draft_arg = if std::path::Path::new(draft_model).exists() {
                 std::fs::canonicalize(draft_model)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or(draft_model.clone())
            } else {
                draft_model.clone()
            };
            args.push(draft_arg);
            
            args.push("--draft".to_string());
            args.push(config.num_draft_tokens.to_string());
            
            // Offload draft model to GPU for efficient speculative decoding
            args.push("--n-gpu-layers-draft".to_string());
            args.push("99".to_string());
        }

        println!("Starting llama-server with model {} on port {}...", config.model, port);

        let process = Command::new(&server_bin)
            .args(&args)
            .stdout(if crate::debug_logging() { Stdio::inherit() } else { Stdio::null() })
            .stderr(if crate::debug_logging() { Stdio::inherit() } else { Stdio::null() })
            .spawn()?;

        let server = Arc::new(Self {
            _process: Some(process),
            base_url: format!("http://localhost:{}", port),
            model: config.model.clone(),
        });

        // Background health check
        let check_server = server.clone();
        tokio::spawn(async move {
             if let Err(e) = check_server.wait_until_ready(300).await {
                   eprintln!("llama-server failed to start or timed out: {}", e);
             } else {
                 println!("llama-server is ready.");
             }
        });

        Ok(server)
    }

    pub async fn wait_until_ready(
        &self,
        timeout_secs: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        let url = format!("{}/health", self.base_url); // llama-server has /health
        let deadline = Duration::from_secs(timeout_secs);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > deadline {
                return Err(format!("llama-server timed out after {}s.", timeout_secs).into());
            }

            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => sleep(Duration::from_millis(1000)).await,
            }
        }
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<serde_json::Value>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default)]
    pub stream: bool,
}

fn default_max_tokens() -> u32 { 512 }
fn default_temperature() -> f32 { 0.7 }

pub async fn chat_handler(
    state: State<crate::ServerState>,
    Json(payload): Json<ChatRequest>,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    let llm_config = &state.llm_config;
    
    // Determine target URL based on provider
    let (target_url, model) = if llm_config.provider == "ollama" || llm_config.provider == "local" || llm_config.provider == "llama" {
         (format!("http://localhost:{}/v1/chat/completions", llm_config.port), llm_config.model.clone())
    } else if llm_config.provider == "openai" {
         ("https://api.openai.com/v1/chat/completions".to_string(), llm_config.model.clone())
    } else {
        return Err((axum::http::StatusCode::BAD_REQUEST, format!("Unknown provider: {}", llm_config.provider)));
    };

    let client = reqwest::Client::new();
    
    // Construct request
    let mut request_builder = client.post(&target_url)
        .json(&serde_json::json!({
            "model": model, 
            "messages": payload.messages,
            "max_tokens": payload.max_tokens,
            "temperature": payload.temperature,
            "stream": payload.stream // Pass through stream flag
        }));
        
    if let Some(api_key) = &llm_config.openai_api_key {
        request_builder = request_builder.header("Authorization", format!("Bearer {}", api_key));
    }

    let response = request_builder
        .send()
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, format!("LLM Provider unreachable: {}", e)))?;

     if !response.status().is_success() {
         let status = response.status();
         let error_text = response.text().await.unwrap_or_default();
         return Err((status, format!("LLM Error: {}", error_text)));
    }

    // Proxy the response body directly
    // Ideally we stream it if payload.stream is true
    let body = axum::body::Body::from_stream(response.bytes_stream()); // This streams the response chunks
    
    Ok(axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("Content-Type", if payload.stream { "text/event-stream" } else { "application/json" })
        .body(body)
        .unwrap())
}
