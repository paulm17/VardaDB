pub mod engine;
pub mod storage;
pub mod codegen;
pub mod realtime;
pub mod bridge;
pub mod caching;
pub mod sync;
pub mod config;
pub mod worker;
pub mod vardaclaw_runner;
pub mod cli;
pub mod defaults;
pub mod server;

pub mod repl;
pub mod observability;
pub use jobs; 
// pub mod vardajobs; // Refactored to external crate


pub struct DummyResolver;

impl crate::engine::resolver::Resolver for DummyResolver {
    fn resolve(&self, _: u64, _: &str) -> Option<async_graphql::Value> {
        None
    }
    fn find_uid(&self, _: &str, _: &str) -> Option<u64> {
        None
    }
    fn create_node(&self, _type: &str, _fields: std::collections::HashMap<String, async_graphql::Value>, _uniques: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>, _: Option<&crate::engine::resolver::VectorConfig>) -> Result<u64, String> {
        Ok(0)
    }
    fn scan_nodes(&self, _: &str, _: std::collections::HashMap<String, async_graphql::Value>, _: std::collections::HashMap<String, async_graphql::Value>, _: Option<usize>, _: Option<String>, _: &[String], _: Option<Vec<f64>>) -> Vec<u64> { vec![] }
    fn resolve_list(&self, _: u64, _: &str, _: std::collections::HashMap<String, async_graphql::Value>, _: std::collections::HashMap<String, async_graphql::Value>, _: Option<usize>, _: Option<String>, _: Option<Vec<f64>>) -> Result<Vec<u64>, String> {
        Ok(vec![])
    }
    fn update_node(&self, _: &str, _: u64, _: std::collections::HashMap<String, async_graphql::Value>, _: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>, _: Option<&crate::engine::resolver::VectorConfig>) -> Result<(), String> { Ok(()) }

    fn delete_node(&self, _: &str, _: u64, _: &[String], _: &[crate::engine::resolver::InverseInfo], _: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> { Ok(()) }
    fn node_exists(&self, _: &str, _: u64) -> bool { false }
    fn get_node_type(&self, _: u64) -> Option<String> { None }
    fn subscribe_events(&self) -> crate::realtime::bus::EventBus { crate::realtime::bus::EventBus::new() }
    fn search_vectors(&self, _: &[f64], _: usize) -> Vec<(u64, f64)> { vec![] }
    fn search_hybrid(&self, _: &str, _: &str, _: &[f64], _: usize) -> Vec<(u64, f64)> { vec![] }
    fn flush(&self) -> Result<(), String> { Ok(()) }
    fn compact(&self) -> Result<u64, String> { Ok(0) }
    fn needs_compaction(&self) -> bool { false }
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
use crate::realtime::bus::EventBus;
use metrics::{counter, histogram};
use tracing::info;
use tokio::sync::RwLock;


#[derive(Clone)]
struct ServerState {
    // Map<db_name, Arc<RwLock<Arc<crate::engine::schema::Schema>>>>
    schemas: Arc<dashmap::DashMap<String, Arc<RwLock<Arc<crate::engine::schema::Schema>>>>>, 
    storage: Arc<Storage>,
    cache: Arc<crate::engine::cache::QueryCache>,
    event_bus: EventBus,
    storage_path: std::path::PathBuf,
}

impl axum::extract::FromRef<ServerState> for crate::server::management::ManagementState {
    fn from_ref(state: &ServerState) -> Self {
        Self {
            storage: state.storage.clone(),
            schemas: state.schemas.clone(),
            event_bus: state.event_bus.clone(),
            storage_path: state.storage_path.clone(),
        }
    }
}

impl axum::extract::FromRef<ServerState> for crate::observability::router::ObsState {
    fn from_ref(state: &ServerState) -> Self {
        Self {
            storage: state.storage.clone(),
        }
    }
}

pub async fn run(config: crate::config::VardaConfig) {
    let port = config.server.port;
    println!("VardaDB Engine v0.1.0 starting on port {}...", port);

    // 1. Initialize Storage
    let storage_path = config.server.storage_path.clone(); 
    let storage = Arc::new(Storage::new(&storage_path, config.server.node_id).expect("Failed to open storage"));

    println!("Storage initialized at ./{}", storage_path);

    // 1.5 Initialize Observability (Metrics + Tracing)
    crate::observability::init(storage.clone());
    
    // Setup Tracing Subscriber with Storage Backend
    // let trace_layer = crate::observability::backend::VardaTraceLayer::new(storage.clone());
    // use tracing_subscriber::prelude::*;
    // let registry = tracing_subscriber::registry().with(trace_layer);
    // registry.init(); // This sets the global default. Might panic if called twice (e.g. tests)
    
    info!("Observability initialized (Metrics + Traces in sorted keyspaces)");

    // 2. Load Schema (from config path or default)
    // If config has schema_path, use it. Else default to storage/current_schema.graphql
    let schema_file_path = config.server.schema_path.clone()
        .unwrap_or_else(|| format!("{}/current_schema.graphql", storage_path));
    
    let loaded_sdl = std::fs::read_to_string(&schema_file_path).ok();
    
    let (sdl, _is_default) = match loaded_sdl {
        Some(s) => {
            println!("Loaded persisted schema from {}", schema_file_path);
            (s, false)
        },
        None => {
             println!("No persisted schema found at {}. Using default.", schema_file_path);
             ("type Health { status: String }".to_string(), true)
        }
    };

    // Create a shared EventBus that will be used by all resolvers
    let shared_event_bus = EventBus::new();

    let resolver = FjallResolver::with_bus(storage.clone(), shared_event_bus.clone());

    let initial_schema = crate::engine::schema::Schema::load_with_resolver(&sdl, resolver.clone())
        .or_else(|e| {
            println!("Failed to load persisted schema: {}. Falling back to default.", e);
             let default_sdl = "type Health { status: String }";
             let blank_resolver = FjallResolver::with_bus(storage.clone(), shared_event_bus.clone()); 
             crate::engine::schema::Schema::load_with_resolver(default_sdl, blank_resolver)
        })
        .expect("Failed to build schema");

    let schemas = Arc::new(dashmap::DashMap::new());
    schemas.insert("default".to_string(), Arc::new(RwLock::new(Arc::new(initial_schema))));

    let state = ServerState {
        schemas,
        storage: storage.clone(),
        cache: Arc::new(crate::engine::cache::QueryCache::new(100)), // Bounded LRU: 100 entries
        event_bus: shared_event_bus.clone(),
        storage_path: std::path::PathBuf::from(&config.server.storage_path),
    };

    // Start Anti-Gravity (Zenoh) Sync
    let sync_resolver = std::sync::Arc::new(resolver.clone());
    let zenoh_config = config.zenoh.clone();
    let remote_append_path = config.remote_append.path.clone();
    let sync_schema = state.schemas.get("default").expect("Default schema missing").clone();
    let sync_cache = state.cache.clone();
    
    tokio::spawn(async move {
         println!("Initializing Zenoh Sync...");
         match crate::sync::manager::SyncManager::new(sync_resolver, zenoh_config, remote_append_path, sync_schema, sync_cache).await {
             Ok(manager) => {
                 if let Err(e) = manager.start().await {
                     eprintln!("SyncManager Error: {}", e);
                 }
             },
             Err(e) => eprintln!("Failed to initialize SyncManager: {}", e),
         }
    });

    // Start Job Workers
    let worker_count = config.jobs.workers.min(10); // Enforce max 10
    println!("Starting {} Job Workers...", worker_count);
    
    // Set concurrency limit on queue to match workers (or higher? default 100 is fine)
    // storage.system_queue.set_concurrency_limit(worker_count * 5); 
    
    // Register System Heartbeat (Run every 10 seconds)
    // Expression: sec min hour day_of_month month day_of_week year(opt)
    if let Err(e) = storage.system_queue.register_cron(
        "heartbeat".to_string(), 
        "0/10 * * * * * *".to_string(), 
        "system_queue".to_string(), 
        b"HEARTBEAT".to_vec()
    ) {
        eprintln!("Failed to register heartbeat cron: {}", e);
    }

    // Initialize LLM Gateway - MOVED TO VARDACLAW
    // The native worker is now "dumb" and doesn't need LLM/Skills.
    
    for i in 0..worker_count {
        let worker = crate::worker::Worker::new(storage.clone(), i);
        tokio::spawn(async move {
            worker.run().await;
        });
    }

    // Start VardaClaw Background Runner
    let claw_runner = crate::vardaclaw_runner::VardaClawRunner::new(storage.clone(), config.clone());
    tokio::spawn(async move {
        claw_runner.run().await;
    });

    // 3. Setup Routes
    let mgmt_routes = crate::server::management::routes::<ServerState>()
        .merge(crate::observability::router::routes::<ServerState>());

    let app = Router::new()
        .route("/graphql", post(graphql_handler).get(subscription_handler))
        .route("/playground", get(playground_handler))
        .route("/admin/schema", post(admin_schema_handler))
        .nest("/_mgmt", mgmt_routes)
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(state.clone());

    // 4. Run Server
    if config.server.is_mcp {
        // Run MCP Server
        // Use default schema for MCP for now
        let mcp_schema = state.schemas.get("default").expect("Default schema missing").value().clone();
        
        let mcp_resolver = Box::new(FjallResolver::with_bus(storage.clone(), shared_event_bus.clone()));
        
        // We can use the concrete resolver we created earlier:
        // let mcp_resolver = Box::new(resolver.clone()); // FjallResolver implements Clone? Let's assume so or check.
        // Looking at `src/bridge/fjall_resolver.rs`, it derives Clone.
        
        let mcp_server = crate::bridge::mcp::MCPServer::new(mcp_schema, mcp_resolver);
        
        if let Err(e) = mcp_server.run_stdio_server().await {
            eprintln!("MCP Server Error: {}", e);
        }
    } else {
        // Run HTTP Server
        let addr = format!("0.0.0.0:{}", port);
        let listener = TcpListener::bind(&addr).await.expect("Failed to bind to address");
        println!("Server running at http://127.0.0.1:{}", port);
        println!("GraphiQL playground at http://127.0.0.1:{}/playground", port);
        println!("Admin Schema Endpoint at http://127.0.0.1:{}/admin/schema", port);
        
        axum::serve(listener, app).await.expect("Server error");
    }
}

async fn graphql_handler(
    headers: axum::http::HeaderMap,
    axum::extract::State(state): axum::extract::State<ServerState>,
    req: GraphQLRequest,

) -> GraphQLResponse {
    let start = std::time::Instant::now();
    counter!("graphql_requests_total").increment(1);
    
    // Extract Trace Context? (If we were using OTel HTTP prop, but we are embedded)
    
    // Span is automatically created by `instrument` if we add it, but we can also manually trace.
    // Let's use the macro on the function or manually enter a span.
    // Since we are inside the handler, let's wrap the logic in a span.
    let span = tracing::info_span!("graphql_request", method = "POST");
    let _enter = span.enter();
    let db_name = headers
        .get("x-varda-db")
        .and_then(|val| val.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string());

    let schema = if let Some(s) = state.schemas.get(&db_name) {
        s.read().await.clone()
    } else {
        // Lazy Load / Create Schema for DB
        // Check if DB exists in Storage
        if state.storage.get_database(&db_name).is_some() {
             println!("Lazy loading schema for database: {}", db_name);
             let resolver = FjallResolver::new(state.storage.clone(), &db_name);
             
             let db_schema_path = format!("{}/{}_schema.graphql", "varda_db_data", db_name);
             let sdl = std::fs::read_to_string(&db_schema_path).unwrap_or_else(|_| "type Health { status: String }".to_string());
             
             let new_schema = crate::engine::schema::Schema::load_with_resolver(&sdl, resolver)
                .expect("Failed to load lazy schema");
             let arc_schema = Arc::new(RwLock::new(Arc::new(new_schema)));
             state.schemas.insert(db_name.clone(), arc_schema.clone());
             let x = arc_schema.read().await.clone();
             x
        } else {
            println!("Database {} not found/loaded, defaulting to default schema context.", db_name);
             if let Some(s) = state.schemas.get("default") { s.read().await.clone() } else { 
                 let mut resp = async_graphql::Response::new(async_graphql::Value::Null);
                 resp.errors = vec![async_graphql::ServerError::new("Internal Error: Default schema missing", None)];
                 return resp.into();
             }
        }
    };
    
    let request = req.into_inner();
    
    // Check if Mutation (Simple check: contains "mutation")
    // A robust check would parse, but for this Demo/PoC string match is 99% effective.
    let query_string = request.query.clone(); // Clone to keep owned String for Cache Key
    let is_mutation = query_string.contains("mutation");

    if is_mutation {
        let resp = schema.execute(request).await;
        // Invalidate Cache on Mutation (Global for now, ideally scoped to DB)
        state.cache.invalidate();
        return resp.into();
    }

    // Read Path (Caching)
    // Key needs to include DB Name!
    let vars_str = serde_json::to_string(&request.variables).unwrap_or_default();
    let cache_key_suffix = format!("{}:{}", db_name, vars_str);
    
    if let Some(cached_json) = state.cache.get(&query_string, &cache_key_suffix) {
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
            state.cache.put(&query_string, &cache_key_suffix, json);
        }
    }



    // Record Latency
    let duration = start.elapsed().as_secs_f64();
    histogram!("graphql_request_duration_seconds").record(duration);
    
    resp.into()
}

// ... subscription_handler ...

async fn admin_schema_handler(
    axum::extract::State(state): axum::extract::State<ServerState>,
    body: String,
) -> impl IntoResponse {
    println!("Received new schema update...");
    // CRITICAL: Use shared EventBus to ensure SyncManager and subscriptions use the same bus
    let resolver = FjallResolver::with_bus(state.storage.clone(), state.event_bus.clone());
    
    match crate::engine::schema::Schema::load_with_resolver(&body, resolver) {
        Ok(new_schema) => {
             // Update DashMap
             // We need to fetch the existing entry and update the WRITE lock
             if let Some(entry) = state.schemas.get("default") {
                 let mut lock = entry.write().await;
                 *lock = Arc::new(new_schema);
             } else {
                 // Should not happen, but insert new
                 state.schemas.insert("default".to_string(), Arc::new(RwLock::new(Arc::new(new_schema))));
             }
            
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
    let schema_wrapper = state.schemas.get("default").expect("Default schema missing from map").read().await.clone();
    // Inner Schema (likely async_graphql::Schema) usually implements execution/protocols
    // But we need to ensure we pass a type that implements Executor + Send + Sync + Clone
    // crate::engine::schema::Schema is likely our wrapper.
    // If it is just a wrapper around async_graphql::Schema, we can use it.
    // But if we need the inner async_graphql::Schema:
    // let schema = schema_wrapper.inner().clone(); // Assuming inner() exists? 
    // Wait, in previous code: `state.schema.read().await.clone()` was passed.
    // `state.schema` was `Arc<RwLock<Arc<Schema>>>`. read() -> `Arc<Schema>`. clone() -> `Arc<Schema>`.
    // So we are passing `Arc<Schema>`.
    
    // The ERROR said: `trait Executor is not implemented for Arc<Schema>`.
    // This implies `Schema` implements `Executor`, but `Arc<Schema>` does not.
    // We should deference the Arc if possible, OR clone the inner Schema if it's cheap (async_graphql::Schema is cheap).
    
    // Let's try to get the inner schema.
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
    let source = GraphiQLSource::build().endpoint("/graphql").finish();
    let source = source.replace(
        "fetcher: GraphiQL.createFetcher({",
        "headers: JSON.stringify({ 'x-varda-db': 'archondb' }), fetcher: GraphiQL.createFetcher({",
    );
    Html(source)
}

pub fn build_schema(sdl: &str) -> Result<crate::engine::schema::Schema, String> {
    crate::engine::schema::Schema::load_from_sdl(sdl)
}
