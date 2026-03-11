use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
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
const EMBEDDINGS_BATCH_SIZE: usize = 256;

pub struct MlxEngine {
    pub base_url: String,
    pub model: String,
    client: reqwest::Client,
    child: Mutex<Child>,
    config_path: PathBuf,
}

impl MlxEngine {
    pub fn start(config: LLMConfig) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let server_path = resolve_mlx_server_path(&config);
        let bind = format!("127.0.0.1:{}", config.port);
        let base_url = format!("http://{}", bind);
        let config_path = write_mlx_server_config(&config, &bind)?;

        let mut command = Command::new(&server_path);
        command
            .arg("--config")
            .arg(&config_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = command.spawn().map_err(|e| {
            format!(
                "failed to start mlx-server at {}: {}",
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
            "started managed mlx-server"
        );

        Ok(Arc::new(Self {
            base_url,
            model: config.model,
            client: reqwest::Client::builder().build()?,
            child: Mutex::new(child),
            config_path,
        }))
    }
}

impl Drop for MlxEngine {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.config_path);
    }
}

fn cleanup_process(child: &mut Child, config_path: &PathBuf) {
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_file(config_path);
}

fn resolve_mlx_server_path(config: &LLMConfig) -> PathBuf {
    if let Some(path) = &config.llama_server_path {
        return PathBuf::from(path);
    }

    let release = PathBuf::from("../mlx-rs/target/release/mlx-server");
    if release.exists() {
        return release;
    }

    let debug = PathBuf::from("../mlx-rs/target/debug/mlx-server");
    if debug.exists() {
        return debug;
    }

    PathBuf::from("mlx-server")
}

fn write_mlx_server_config(config: &LLMConfig, bind: &str) -> Result<PathBuf> {
    let mut content = format!(
        "[server]\nbind = \"{}\"\nembeddings_batch_size = {}\n",
        bind, EMBEDDINGS_BATCH_SIZE
    );

    if !config.model.is_empty() && config.model != "llama3" {
        content.push_str(&format!(
            "model_path = \"{}\"\n",
            config.model.replace('"', "\\\"")
        ));
    }

    if let Some(hf_token) = &config.huggingface.hf_token {
        content.push_str(&format!(
            "\n[huggingface]\nhf_token = \"{}\"\n",
            hf_token.replace('"', "\\\"")
        ));
    }

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
            anyhow::bail!("mlx-server on port {port} did not become ready within 10 seconds");
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
            format!("mlx-server request failed: {}", e),
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
