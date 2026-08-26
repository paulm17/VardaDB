use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::Response,
};
use futures_util::TryStreamExt;
use tokio::time::sleep;
use tracing::info;

use crate::config::LLMConfig;

const MLX_SERVER_START_TIMEOUT: Duration = Duration::from_secs(10);
const MLX_SERVER_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MLX_SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const EMBEDDINGS_BATCH_SIZE: usize = 256;

static ACTIVE_MLX_ENGINES: std::sync::OnceLock<Mutex<Vec<Weak<MlxEngine>>>> =
    std::sync::OnceLock::new();
static MLX_EXIT_HOOK_REGISTERED: std::sync::Once = std::sync::Once::new();

pub struct MlxEngine {
    pub base_url: String,
    pub model: String,
    client: reqwest::Client,
    child: Mutex<Option<Child>>,
    config_path: PathBuf,
}

impl MlxEngine {
    pub fn start(config: LLMConfig) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        register_process_exit_hook();

        let server_path = resolve_mlx_server_path(&config);
        let bind = "127.0.0.1";
        let base_url = format!("http://{}:{}", bind, config.port);
        let config_path = write_mlx_server_config(&config, &bind, &server_path)?;

        let mut command = Command::new(&server_path);
        command
            .arg("--config")
            .arg(&config_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = command.spawn().map_err(|e| {
            format!(
                "failed to start llama server at {}: {}",
                server_path.display(),
                e
            )
        })?;

        if let Err(e) = wait_for_mlx_server(config.port) {
            cleanup_process(&mut child, &config_path);
            return Err(e.into());
        }

        info!(
            target: "vardadb::llm::proxy",
            base_url = base_url.as_str(),
            server_path = server_path.display().to_string(),
            "started managed llama server"
        );

        let engine = Arc::new(Self {
            base_url,
            model: config.model,
            client: reqwest::Client::builder().build()?,
            child: Mutex::new(Some(child)),
            config_path,
        });

        register_managed_engine(&engine);

        Ok(engine)
    }

    pub fn shutdown(&self) {
        if let Ok(mut child_guard) = self.child.lock() {
            if let Some(mut child) = child_guard.take() {
                shutdown_child(&mut child, &self.config_path);
            } else {
                let _ = fs::remove_file(&self.config_path);
            }
        } else {
            let _ = fs::remove_file(&self.config_path);
        }
    }
}

impl Drop for MlxEngine {
    fn drop(&mut self) {
        if let Ok(child_guard) = self.child.get_mut() {
            if let Some(child) = child_guard.as_mut() {
                shutdown_child(child, &self.config_path);
            } else {
                let _ = fs::remove_file(&self.config_path);
            }
        } else {
            let _ = fs::remove_file(&self.config_path);
        }
    }
}

fn cleanup_process(child: &mut Child, config_path: &PathBuf) {
    shutdown_child(child, config_path);
}

fn register_managed_engine(engine: &Arc<MlxEngine>) {
    let mutex = ACTIVE_MLX_ENGINES.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut list) = mutex.lock() {
        list.push(Arc::downgrade(engine));
    }
}

fn register_process_exit_hook() {
    MLX_EXIT_HOOK_REGISTERED.call_once(|| unsafe {
        libc::atexit(cleanup_on_process_exit);
    });
}

extern "C" fn cleanup_on_process_exit() {
    shutdown_all_managed_processes();
}

pub fn shutdown_all_managed_processes() {
    if let Some(mutex) = ACTIVE_MLX_ENGINES.get() {
        if let Ok(mut list) = mutex.lock() {
            for weak in list.drain(..) {
                if let Some(engine) = weak.upgrade() {
                    engine.shutdown();
                }
            }
        }
    }
}

fn shutdown_child(child: &mut Child, config_path: &PathBuf) {
    let already_exited = matches!(child.try_wait(), Ok(Some(_)));
    if !already_exited {
        #[cfg(unix)]
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }

        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < MLX_SERVER_SHUTDOWN_TIMEOUT => {
                    thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }
    let _ = fs::remove_file(config_path);
}

fn resolve_mlx_server_path(config: &LLMConfig) -> PathBuf {
    if let Some(path) = &config.llama_server_path {
        return PathBuf::from(path);
    }

    for candidate in [
        "../mlx-rs/target/release/llama-server",
        "../mlx-rs/target/debug/llama-server",
        "../mlx-rs/target/release/mlx-server",
        "../mlx-rs/target/debug/mlx-server",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return path;
        }
    }

    PathBuf::from("llama-server")
}

/// Locate an inherited mlx-rs config file, if one exists.
///
/// Discovery order:
/// 1. `MLX_RS_CONFIG` environment variable (explicit path)
/// 2. `config.toml` in the repo root of the resolved server binary
///    (e.g. `../mlx-rs/target/release/llama-server` -> `../mlx-rs/config.toml`)
fn discover_base_config(server_path: &Path) -> Option<(PathBuf, toml::Table)> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(explicit) = std::env::var("MLX_RS_CONFIG") {
        candidates.push(PathBuf::from(explicit));
    }

    // Binary at <repo>/target/<profile>/llama-server -> <repo>/config.toml
    if let Some(repo_dir) = server_path.ancestors().nth(2) {
        candidates.push(repo_dir.join("config.toml"));
    }
    candidates.push(PathBuf::from("../mlx-rs/config.toml"));

    for candidate in candidates {
        if let Ok(content) = fs::read_to_string(&candidate) {
            match toml::from_str::<toml::Table>(&content) {
                Ok(table) => return Some((candidate, table)),
                Err(e) => {
                    info!(
                        target: "vardadb::llm::proxy",
                        path = candidate.display().to_string(),
                        error = %e,
                        "ignoring invalid inherited server config"
                    );
                }
            }
        }
    }

    None
}

fn write_mlx_server_config(config: &LLMConfig, bind: &str, server_path: &Path) -> Result<PathBuf> {
    let (base_path, root_table) = match discover_base_config(server_path) {
        Some((path, table)) => (Some(path), table),
        None => (None, toml::Table::new()),
    };
    let mut root = toml::Value::Table(root_table);

    if base_path.is_some() {
        info!(
            target: "vardadb::llm::proxy",
            path = base_path.unwrap().display().to_string(),
            "inheriting mlx-rs server config"
        );
    }

    if !root.as_table().map(|t| t.contains_key("server")).unwrap_or(false) {
        root.as_table_mut()
            .expect("root is always a table")
            .insert("server".to_string(), toml::Value::Table(toml::map::Map::new()));
    }

    {
        let server = root
            .get_mut("server")
            .and_then(|v| v.as_table_mut())
            .expect("server section is guaranteed to be a table");

        // VardaDB always controls these keys.
        server.insert("bind".to_string(), toml::Value::String(bind.to_string()));
        server.insert("port".to_string(), toml::Value::Integer(config.port as i64));
        server.insert(
            "embeddings_batch_size".to_string(),
            toml::Value::Integer(EMBEDDINGS_BATCH_SIZE as i64),
        );

        if !server.contains_key("embedding") {
            // Not set by the inherited config: default embeddings on so
            // /v1/embeddings works with embedding models out of the box.
            server.insert("embedding".to_string(), toml::Value::Boolean(true));
        }

        if !config.model.is_empty() && config.model != "llama3" {
            server.insert(
                "model_path".to_string(),
                toml::Value::String(config.model.clone()),
            );
        }
    }

    if let Some(hf_token) = &config.huggingface.hf_token {
        let hf = root
            .as_table_mut()
            .expect("root is always a table")
            .entry("huggingface")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        hf.as_table_mut()
            .expect("huggingface section is a table")
            .insert("hf_token".to_string(), toml::Value::String(hf_token.clone()));
    }

    let content = toml::to_string_pretty(&root)?;

    let path = std::env::temp_dir().join(format!(
        "vardadb-mlx-server-{}-{}.toml",
        std::process::id(),
        config.port
    ));
    fs::write(&path, content)?;
    Ok(path)
}

fn wait_for_mlx_server(port: u16) -> Result<()> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
    let started = Instant::now();
    loop {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        if started.elapsed() >= MLX_SERVER_START_TIMEOUT {
            anyhow::bail!("llama server on port {port} did not become ready within 10 seconds");
        }
        thread::sleep(MLX_SERVER_POLL_INTERVAL);
    }
}

fn filter_request_headers(headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::new();
    for (name, value) in headers {
        if matches!(
            name.as_str(),
            "host" | "content-length" | "connection" | "transfer-encoding"
        ) {
            continue;
        }
        filtered.append(name.clone(), value.clone());
    }
    filtered
}

fn copy_response_headers(from: &HeaderMap, to: &mut HeaderMap) {
    for (name, value) in from {
        if matches!(
            name.as_str(),
            "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "content-length"
        ) {
            continue;
        }
        to.append(name.clone(), value.clone());
    }
}

async fn proxy_request(
    state: crate::ServerState,
    method: Method,
    path: &str,
    headers: HeaderMap,
    body: Option<Bytes>,
) -> Result<Response, (StatusCode, String)> {
    let engine = state.llama_server.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "MLX service not started".to_string(),
        )
    })?;

    let url = format!("{}{}", engine.base_url, path);
    let started = Instant::now();
    let mut request = engine
        .client
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("invalid proxy method: {}", e),
                )
            })?,
            &url,
        )
        .headers(filter_request_headers(&headers));

    if let Some(body) = body {
        request = request.body(body);
    }

    let upstream = request.send().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("llama server request failed: {}", e),
        )
    })?;

    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let upstream_headers = upstream.headers().clone();
    let stream = upstream.bytes_stream().map_err(std::io::Error::other);

    let mut response = Response::builder()
        .status(status)
        .body(Body::from_stream(stream))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {}", e),
            )
        })?;
    copy_response_headers(&upstream_headers, response.headers_mut());

    info!(
        target: "vardadb::llm::proxy",
        method = method.as_str(),
        path,
        status = status.as_u16(),
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "proxied mlx request"
    );

    Ok(response)
}

pub async fn models_handler(
    State(state): State<crate::ServerState>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    proxy_request(state, Method::GET, "/v1/models", headers, None).await
}

pub async fn load_handler(
    State(state): State<crate::ServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    proxy_request(state, Method::POST, "/llm/load", headers, Some(body)).await
}

pub async fn unload_handler(
    State(state): State<crate::ServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    proxy_request(state, Method::POST, "/llm/unload", headers, Some(body)).await
}

pub async fn embeddings_handler(
    State(state): State<crate::ServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    proxy_request(state, Method::POST, "/v1/embeddings", headers, Some(body)).await
}

pub async fn chat_handler(
    State(state): State<crate::ServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    proxy_request(
        state,
        Method::POST,
        "/v1/chat/completions",
        headers,
        Some(body),
    )
    .await
}

pub async fn wait_for_proxy_ready(state: Arc<MlxEngine>) -> bool {
    let started = Instant::now();
    while started.elapsed() < MLX_SERVER_START_TIMEOUT {
        if state
            .client
            .get(format!("{}/v1/models", state.base_url))
            .send()
            .await
            .is_ok()
        {
            return true;
        }
        sleep(MLX_SERVER_POLL_INTERVAL).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HuggingFaceConfig;

    fn test_config(model: &str, port: u16) -> LLMConfig {
        LLMConfig {
            provider: "mlx".to_string(),
            model: model.to_string(),
            draft_model: None,
            port,
            num_draft_tokens: 0,
            openai_api_key: None,
            llama_server_path: None,
            huggingface: HuggingFaceConfig {
                hf_token: Some("test-token".to_string()),
            },
        }
    }

    #[test]
    fn test_server_config_generation_and_inheritance() {
        // ── Scenario 1: no inherited config → defaults with embedding on ──
        let config = test_config(
            "ChristianAzinn/mxbai-embed-large-v1-gguf/mxbai-embed-large-v1.Q8_0.gguf",
            8080,
        );
        let path =
            write_mlx_server_config(&config, "127.0.0.1", Path::new("/nonexistent/llama-server"))
                .unwrap();
        let content = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).ok();
        assert!(content.contains("embedding = true"), "content: {content}");
        assert!(content.contains("port = 8080"));
        assert!(content.contains("embeddings_batch_size = 256"));
        assert!(content.contains("mxbai-embed-large-v1.Q8_0.gguf"));

        // ── Scenario 2: inherited config merged, VardaDB keys overridden ──
        let base = tempfile::NamedTempFile::new().unwrap();
        fs::write(
            base.path(),
            "[server]\nbind = \"0.0.0.0\"\nport = 1\nembedding = true\nn_ctx = 2048\n\n[llamacpp]\npooling = \"mean\"\n",
        )
        .unwrap();

        std::env::set_var("MLX_RS_CONFIG", base.path());
        let config = test_config("", 8080);
        let path =
            write_mlx_server_config(&config, "127.0.0.1", Path::new("/nonexistent/llama-server"));
        std::env::remove_var("MLX_RS_CONFIG");
        let path = path.unwrap();
        let content = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).ok();

        assert!(content.contains("n_ctx = 2048"), "inherited key lost: {content}");
        assert!(content.contains("pooling = \"mean\""));
        assert!(content.contains("embedding = true"));
        // VardaDB overrides bind/port even when inherited.
        assert!(content.contains("bind = \"127.0.0.1\""));
        assert!(content.contains("port = 8080"));
        // Empty model must not inject model_path.
        assert!(!content.contains("model_path"));
    }
}
