pub mod engine;
pub mod storage;
pub mod codegen;
pub mod realtime;
pub mod bridge;
pub mod caching;
pub mod sync;


pub struct DummyResolver;

impl crate::engine::resolver::Resolver for DummyResolver {
    fn resolve(&self, _: u64, _: &str) -> Option<async_graphql::Value> {
        None
    }
    fn find_uid(&self, _: &str, _: &str) -> Option<u64> {
        None
    }
    fn create_node(
        &self,
        _: &str,
        _: std::collections::HashMap<String, async_graphql::Value>,
        _: &[String],
        _: &[crate::engine::resolver::InverseInfo],
        _: &std::collections::HashMap<String, Vec<String>>
    ) -> Result<u64, String> {
        Ok(0)
    }
    fn scan_nodes(&self, _: &str, _: std::collections::HashMap<String, async_graphql::Value>, _: std::collections::HashMap<String, async_graphql::Value>, _: Option<usize>, _: Option<String>) -> Vec<u64> { vec![] }
    fn update_node(&self, _: &str, _: u64, _: std::collections::HashMap<String, async_graphql::Value>, _: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }

    fn delete_node(&self, _: &str, _: u64, _: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }
    fn node_exists(&self, _: &str, _: u64) -> bool { false }
    fn get_node_type(&self, _: u64) -> Option<String> { None }
    fn subscribe_events(&self) -> crate::realtime::bus::EventBus { crate::realtime::bus::EventBus::new() }
}

use axum::{
    routing::{get, post},
    Router,
    response::{Html, IntoResponse},
};
use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::{CorsLayer, Any};
use crate::storage::backend::Storage;
use crate::bridge::fjall_resolver::FjallResolver;

use tokio::sync::RwLock;

#[derive(Clone)]
struct ServerState {
    schema: Arc<RwLock<Arc<crate::engine::schema::Schema>>>,
    storage: Arc<Storage>,
    cache: Arc<crate::engine::cache::QueryCache>,
}

pub async fn run(port: u16) {
    println!("VardaDB Engine v0.1.0 starting on port {}...", port);

    // 1. Initialize Storage
    let storage_path = "varda_db_data".to_string(); 
    let storage = Arc::new(Storage::new(&storage_path).expect("Failed to open storage"));
    println!("Storage initialized at ./{}", storage_path);

    // 2. Load Schema (from disk or default)
    let schema_file_path = format!("{}/current_schema.graphql", storage_path);
    let loaded_sdl = std::fs::read_to_string(&schema_file_path).ok();
    
    let (sdl, _is_default) = match loaded_sdl {
        Some(s) => {
            println!("Loaded persisted schema from {}", schema_file_path);
            (s, false)
        },
        None => {
             println!("No persisted schema found. Using default.");
             ("type Health { status: String }".to_string(), true)
        }
    };

    let resolver = FjallResolver::new(storage.clone());
    let initial_schema = crate::engine::schema::Schema::load_with_resolver(&sdl, resolver)
        .or_else(|e| {
            println!("Failed to load persisted schema: {}. Falling back to default.", e);
             let default_sdl = "type Health { status: String }";
             let blank_resolver = FjallResolver::new(storage.clone()); 
             crate::engine::schema::Schema::load_with_resolver(default_sdl, blank_resolver)
        })
        .expect("Failed to build schema");
    
    let state = ServerState {
        schema: Arc::new(RwLock::new(Arc::new(initial_schema))),
        storage: storage.clone(),
        cache: Arc::new(crate::engine::cache::QueryCache::new(100)), // Bounded LRU: 100 entries
    };

    // 3. Setup Routes
    let app = Router::new()
        .route("/graphql", post(graphql_handler).get(subscription_handler))
        .route("/playground", get(playground_handler))
        .route("/admin/schema", post(admin_schema_handler))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(state);

    // 4. Run Server
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await.expect("Failed to bind to address");
    println!("Server running at http://127.0.0.1:{}", port);
    println!("GraphiQL playground at http://127.0.0.1:{}/playground", port);
    println!("Admin Schema Endpoint at http://127.0.0.1:{}/admin/schema", port);
    
    axum::serve(listener, app).await.expect("Server error");
}

async fn graphql_handler(
    axum::extract::State(state): axum::extract::State<ServerState>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let schema = state.schema.read().await;
    let request = req.into_inner();
    
    // Check if Mutation (Simple check: contains "mutation")
    // A robust check would parse, but for this Demo/PoC string match is 99% effective.
    let query_string = request.query.clone(); // Clone to keep owned String for Cache Key
    let is_mutation = query_string.contains("mutation");

    if is_mutation {
        let resp = schema.execute(request).await;
        // Invalidate Cache on Mutation
        state.cache.invalidate();
        return resp.into();
    }

    // Read Path (Caching)
    let vars_str = serde_json::to_string(&request.variables).unwrap_or_default();
    
    if let Some(cached_json) = state.cache.get(&query_string, &vars_str) {
        // Return Cached Response
        if let Ok(resp) = serde_json::from_str::<async_graphql::Response>(&cached_json) {
            return resp.into();
        }
    }

    // Execute & Cache
    let resp = schema.execute(request).await;
    
    // Only cache if no errors? Readyset caches result regardless usually (snapshot).
    // But for us, let's cache successful queries.
    if resp.errors.is_empty() {
        if let Ok(json) = serde_json::to_string(&resp) {
            state.cache.put(&query_string, &vars_str, json);
        }
    }

    resp.into()
}

// ... subscription_handler ...

async fn admin_schema_handler(
    axum::extract::State(state): axum::extract::State<ServerState>,
    body: String,
) -> impl IntoResponse {
    println!("Received new schema update...");
    let resolver = FjallResolver::new(state.storage.clone());
    
    match crate::engine::schema::Schema::load_with_resolver(&body, resolver) {
        Ok(new_schema) => {
            let mut lock = state.schema.write().await;
            *lock = Arc::new(new_schema);
            
            // Persist Schema
            let storage_path = "varda_db_data"; // Consistent with run()
            let schema_file_path = format!("{}/current_schema.graphql", storage_path);
            if let Err(e) = tokio::fs::write(&schema_file_path, &body).await {
                eprintln!("Failed to persist schema to {}: {}", schema_file_path, e);
            } else {
                println!("Schema persisted to {}", schema_file_path);
            }

            println!("Schema updated successfully!");
            (axum::http::StatusCode::OK, "Schema updated".to_string())
        }
        Err(e) => {
            println!("Schema validation failed: {}", e);
            (axum::http::StatusCode::BAD_REQUEST, format!("Invalid Schema: {}", e))
        }
    }
}

async fn subscription_handler(
    axum::extract::State(state): axum::extract::State<ServerState>,
    headers: axum::http::HeaderMap,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    let schema_wrapper = state.schema.read().await.clone();
    let schema = schema_wrapper.inner().clone();

    let protocol_str = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|val| val.to_str().ok())
        .unwrap_or("graphql-transport-ws"); 
    
    // Fallback: simpler protocol lookup. 
    // async-graphql's WebSocketProtocols enum might be tricky to parse from string directly if no FromStr.
    // We will rely on ALL_WEBSOCKET_PROTOCOLS usually containing what we need.
    // Actually, WebSocket::new needs the negotiated protocol.
    // If we assume graphql-ws, we pass GraphQLWS.
    
    let protocol = if protocol_str.contains("graphql-transport-ws") {
        async_graphql::http::WebSocketProtocols::GraphQLWS
    } else {
        async_graphql::http::WebSocketProtocols::SubscriptionsTransportWS
    };
    
    upgrade
        .protocols(async_graphql::http::ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |socket| async move {
            use futures_util::{StreamExt, SinkExt, future, pin_mut};
            let (mut sink, stream) = socket.split();

            let stream = stream
                .take_while(|res| future::ready(res.is_ok()))
                .map(|res| res.unwrap())
                .filter_map(|msg| async move {
                    match msg {
                        axum::extract::ws::Message::Text(s) => Some(Vec::from(s.as_bytes())),
                        axum::extract::ws::Message::Binary(b) => Some(b.to_vec()),
                        _ => None,
                    }
                });

            let data_stream = async_graphql::http::WebSocket::new(schema, stream, protocol);
            pin_mut!(data_stream);

            while let Some(msg) = data_stream.next().await {
                match msg {
                    async_graphql::http::WsMessage::Text(s) => {
                        if sink.send(axum::extract::ws::Message::Text(s.into())).await.is_err() {
                            break;
                        }
                    }
                    async_graphql::http::WsMessage::Close(code, reason) => {
                         let _ = sink.send(axum::extract::ws::Message::Close(Some(axum::extract::ws::CloseFrame {
                            code,
                            reason: reason.into(),
                        }))).await;
                        break;
                    }
                }
            }
        })
}



async fn playground_handler() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}

pub fn build_schema(sdl: &str) -> Result<crate::engine::schema::Schema, String> {
    crate::engine::schema::Schema::load_from_sdl(sdl)
}
