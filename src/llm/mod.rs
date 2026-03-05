use mlx_lm::{load_model, GenerationPipeline, Sampler, CausalLM, ChatTemplate, ChatTemplateOptions, Message as LmMessage};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use crate::config::LLMConfig;
use std::sync::Arc;
use axum::{extract::State, Json};
use serde::Deserialize;

pub struct MlxEngine {
    pub base_url: String,
    pub model: String,
    request_tx: mpsc::Sender<EngineRequest>,
}

pub enum EngineRequest {
    Chat {
        messages: Vec<serde_json::Value>,
        max_tokens: Option<usize>,
        temperature: f32,
        thinking: bool,
        stream: bool,
        respond_to: oneshot::Sender<Result<String, String>>,
    },
    Load {
        model_path: String,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    Unload {
        respond_to: oneshot::Sender<Result<(), String>>,
    }
}

impl MlxEngine {
    pub fn start(config: LLMConfig) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let port = config.port;
        let model_from_config = config.model.clone();

        let (tx, mut rx) = mpsc::channel::<EngineRequest>(32);

        // Spawn blocking thread to handle MLX generations synchronously without blocking tokio 
        std::thread::spawn(move || {
            let mut current_model: Option<(Box<dyn CausalLM>, mlx_lm::Tokenizer, PathBuf)> = None;

            let release_model = |model_opt: &mut Option<(Box<dyn CausalLM>, mlx_lm::Tokenizer, PathBuf)>| {
                let _ = mlx_core::Stream::new_gpu_default().synchronize();
                let _ = mlx_core::Stream::new_cpu_default().synchronize();
                if let Some((mut model, _tokenizer, _path)) = model_opt.take() {
                    model.clear_cache();
                    drop(model);
                }
                mlx_core::metal::set_cache_limit(0);
                mlx_core::metal::clear_cache();
                mlx_core::metal::clear_compile_cache();
                let _ = mlx_core::Stream::new_gpu_default().synchronize();
                let _ = mlx_core::Stream::new_cpu_default().synchronize();
            };

            // Optional startup load
            if !model_from_config.is_empty() &&  model_from_config != "llama3" {
                let path = PathBuf::from(&model_from_config);
                if path.exists() {
                     println!("Starting native mlx-lm engine with startup model {}...", model_from_config);
                     match load_model(&path) {
                        Ok(r) => {
                             current_model = Some((r.0, r.1, path));
                             println!("Native mlx-lm engine ready.");
                        },
                        Err(e) => {
                            eprintln!("Failed to load startup MLX model: {}", e);
                        }
                    };
                }
            } else {
                 println!("Native mlx-lm engine started (No model loaded yet).");
            }

            while let Some(req) = rx.blocking_recv() {
                match req {
                    EngineRequest::Chat { messages, max_tokens, temperature, thinking, respond_to, .. } => {
                        if let Some((ref mut model, ref tokenizer, ref model_dir)) = current_model {
                            // Convert JSON messages to LmMessage
                            let mut msgs = Vec::with_capacity(messages.len());
                            for msg in &messages {
                                if let (Some(role), Some(content)) = (msg.get("role").and_then(|v| v.as_str()), msg.get("content").and_then(|v| v.as_str())) {
                                    match role {
                                        "system" => msgs.push(LmMessage::system(content)),
                                        "assistant" => msgs.push(LmMessage::assistant(content)),
                                        _ => msgs.push(LmMessage::user(content)),
                                    }
                                }
                            }
                            
                            let options = ChatTemplateOptions {
                                add_generation_prompt: true,
                                continue_final_message: false,
                                enable_thinking: thinking,
                            };
                            
                            let prompt = if let Ok(template) = ChatTemplate::from_model_dir(model_dir) {
                                template.apply(&msgs, &options).unwrap_or_else(|_| ChatTemplate::chatml().apply(&msgs, &options).unwrap_or_default())
                            } else {
                                ChatTemplate::chatml().apply(&msgs, &options).unwrap_or_else(|_| ChatTemplate::qwen35().apply(&msgs, &options).unwrap_or_default())
                            };
            
                            let sampler = Sampler::new(temperature, 1.0);
                            let mut pipeline = GenerationPipeline::new(model.as_mut(), tokenizer.clone(), sampler);
                            
                            match pipeline.generate_with_metrics(&prompt, max_tokens, |_token, _piece| {}) {
                                Ok((mut text, _metrics)) => {
                                    if !thinking {
                                        if let Some(start) = text.find("<think>") {
                                            if let Some(end) = text.find("</think>") {
                                                let end_tag_len = "</think>".len();
                                                let mut new_text = String::with_capacity(text.len());
                                                new_text.push_str(&text[..start]);
                                                let mut rest = &text[end + end_tag_len..];
                                                
                                                // remove trailing blank lines immediately following the tag
                                                while rest.starts_with('\n') {
                                                    rest = &rest[1..];
                                                }
                                                new_text.push_str(rest);
                                                text = new_text;
                                            }
                                        }
                                    }
                                    let _ = respond_to.send(Ok(text));
                                }
                                Err(e) => {
                                    let _ = respond_to.send(Err(e.to_string()));
                                }
                            }
                        } else {
                            let _ = respond_to.send(Err("No MLX model is currently loaded.".to_string()));
                        }
                    },
                    EngineRequest::Load { model_path, respond_to } => {
                       let path = PathBuf::from(&model_path);
                       if !path.exists() {
                            let _ = respond_to.send(Err(format!("Model path does not exist: {}", model_path)));
                            continue;
                       }
                       println!("Loading new MLX model from {}...", model_path);
                       match load_model(&path) {
                            Ok(r) => {
                                 release_model(&mut current_model);
                                 current_model = Some((r.0, r.1, path));
                                 println!("Model loaded successfully.");
                                 let _ = respond_to.send(Ok(()));
                            },
                            Err(e) => {
                                let err_msg = format!("Failed to load MLX model {}: {}", model_path, e);
                                eprintln!("{}", err_msg);
                                let _ = respond_to.send(Err(err_msg));
                            }
                        };
                    },
                    EngineRequest::Unload { respond_to } => {
                        release_model(&mut current_model);
                        println!("MLX model unloaded successfully.");
                        let _ = respond_to.send(Ok(()));
                    }
                }
            }
        });

        let server = Arc::new(Self {
            base_url: format!("http://localhost:{}", port),
            model: config.model.clone(),
            request_tx: tx,
        });

        Ok(server)
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<serde_json::Value>,
    pub max_tokens: Option<usize>,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default)]
    pub thinking: bool,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub model: Option<String>,
}

fn default_temperature() -> f32 { 0.7 }

#[derive(Deserialize)]
pub struct LoadRequest {
    pub model_path: String,
}

pub async fn load_handler(
    state: State<crate::ServerState>,
    Json(payload): Json<LoadRequest>,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    let mlx_engine = state.llama_server.clone();
    
    if mlx_engine.is_none() {
        return Err((axum::http::StatusCode::SERVICE_UNAVAILABLE, "LLM engine not started".to_string()));
    }
    let engine = mlx_engine.unwrap();

    let (tx, rx) = tokio::sync::oneshot::channel();
    
    let req = EngineRequest::Load {
        model_path: payload.model_path,
        respond_to: tx,
    };

    if let Err(e) = engine.request_tx.send(req).await {
         return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to send request to MLX engine: {}", e)));
    }

    match rx.await {
        Ok(Ok(_)) => {
            let response_json = serde_json::json!({ "ok": true });
            Ok(axum::response::Response::builder()
                .status(axum::http::StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(response_json.to_string()))
                .unwrap())
        }
        Ok(Err(text)) => Err((axum::http::StatusCode::BAD_REQUEST, format!("MLX Load Error: {}", text))),
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Channel error waiting for MLX engine: {}", e)))
    }
}

pub async fn unload_handler(
    state: State<crate::ServerState>,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    let mlx_engine = state.llama_server.clone();
    
    if mlx_engine.is_none() {
        return Err((axum::http::StatusCode::SERVICE_UNAVAILABLE, "LLM engine not started".to_string()));
    }
    let engine = mlx_engine.unwrap();

    let (tx, rx) = tokio::sync::oneshot::channel();
    
    let req = EngineRequest::Unload { respond_to: tx };

    if let Err(e) = engine.request_tx.send(req).await {
         return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to send request to MLX engine: {}", e)));
    }

    match rx.await {
        Ok(Ok(_)) => {
            let response_json = serde_json::json!({ "ok": true });
            Ok(axum::response::Response::builder()
                .status(axum::http::StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(response_json.to_string()))
                .unwrap())
        }
        Ok(Err(text)) => Err((axum::http::StatusCode::BAD_REQUEST, format!("MLX Unload Error: {}", text))),
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Channel error waiting for MLX engine: {}", e)))
    }
}

pub async fn chat_handler(
    state: State<crate::ServerState>,
    Json(payload): Json<ChatRequest>,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    let mlx_engine = state.llama_server.clone();
    
    if mlx_engine.is_none() {
        return Err((axum::http::StatusCode::SERVICE_UNAVAILABLE, "LLM engine not started".to_string()));
    }
    let engine = mlx_engine.unwrap();

    let (tx, rx) = tokio::sync::oneshot::channel();
    
    let req = EngineRequest::Chat {
        messages: payload.messages,
        max_tokens: payload.max_tokens,
        temperature: payload.temperature,
        thinking: payload.thinking,
        stream: false, // Stream handling needs a stream channel, simple oneshot for now
        respond_to: tx,
    };

    if let Err(e) = engine.request_tx.send(req).await {
         return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to send request to MLX engine: {}", e)));
    }

    match rx.await {
        Ok(Ok(text)) => {
            let response_json = serde_json::json!({
                "id": "chatcmpl-local",
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": text
                    },
                    "finish_reason": "stop"
                }]
            });
            
            Ok(axum::response::Response::builder()
                .status(axum::http::StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(response_json.to_string()))
                .unwrap())
        }
        Ok(Err(text)) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("MLX Generation Error: {}", text))),
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Channel error waiting for MLX engine: {}", e)))
    }
}
