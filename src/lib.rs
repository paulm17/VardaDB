/// Returns true if VARDADB_DEBUG=1 is set. Checked once, cached forever.
pub fn debug_logging() -> bool {
    static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var("VARDADB_DEBUG").map(|v| v == "1").unwrap_or(false))
}

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
pub mod llm;

pub mod repl;
pub mod observability;
pub use jobs; 

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
pub struct ServerState {
    // Map<db_name, Arc<RwLock<Arc<crate::engine::schema::Schema>>>>
    pub schemas: Arc<dashmap::DashMap<String, Arc<RwLock<Arc<crate::engine::schema::Schema>>>>>, 
    pub storage: Arc<Storage>,
    pub cache: Arc<crate::engine::cache::QueryCache>,
    pub event_bus: EventBus,
    pub storage_path: std::path::PathBuf,
    pub llm_config: crate::config::LLMConfig,
    // pub mlx_server: Option<Arc<crate::llm::MlxServer>>, // Removed
    pub llama_server: Option<Arc<crate::llm::LlamaServer>>,
    pub auth: Option<Arc<auth::state::AuthState>>,
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


/// Initializes the VardaDB system and returns the State and Axum Router.
/// This allows the caller to inspect state or attach additional services (like the AI Brain)
/// before starting the HTTP server.
pub async fn init_system(config: crate::config::VardaConfig) -> (Arc<ServerState>, Router) {
    let _port = config.server.port;
    println!("VardaDB Engine v0.1.0 initializing...");

    // 1. Initialize Storage
    let storage_path = config.server.storage_path.clone(); 
    let storage = Arc::new(Storage::new(&storage_path, config.server.node_id).expect("Failed to open storage"));
    storage.register_exit_hook();

    println!("Storage initialized at ./{}", storage_path);

    // 1.5 Initialize Observability (Metrics + Tracing)
    crate::observability::init(storage.clone());
    
    info!("Observability initialized (Metrics + Traces in sorted keyspaces)");

    // 2. Load Schema (from config path or default)
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

    // MlxServer init removed
    
    // Initialize Llama Server if Llama Provider
    let llama_server = if config.llm.provider == "llama" {
        match crate::llm::LlamaServer::start(config.llm.clone()) {
            Ok(server) => Some(server),
            Err(e) => {
                eprintln!("Failed to start Llama Server: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Initialize Auth Subsystem
    let auth_state = if let Some(auth_config) = config.auth.clone() {
        println!("Auth subsystem enabled");
        let email_queue = Some(Arc::new(jobs::Queue::new("auth_email".to_string(), storage.jobs_store.clone())));
        
        match auth::state::AuthState::new(auth_config, &storage.db, email_queue.clone()) {
            Ok(state) => {
                let arc_state = Arc::new(state);
                
                let arc_state_for_pruning = arc_state.clone();
                tokio::spawn(async move {
                    auth::state::start_pruning_task(arc_state_for_pruning).await;
                });
                
                #[cfg(feature = "auth-email")]
                if let Some(queue) = email_queue {
                    let arc_state_clone = arc_state.clone();
                    tokio::spawn(async move {
                        auth::email::job::start_email_worker(arc_state_clone, queue).await;
                    });
                }
                Some(arc_state)
            },
            Err(e) => {
                eprintln!("Failed to initialize Auth subsystem: {}", e);
                None
            }
        }
    } else {
        None
    };

    let schemas = Arc::new(dashmap::DashMap::new());
    schemas.insert("default".to_string(), Arc::new(RwLock::new(Arc::new(initial_schema))));

    let state = Arc::new(ServerState {
        schemas: schemas.clone(),
        storage: storage.clone(),
        cache: Arc::new(crate::engine::cache::QueryCache::new(100)), // Bounded LRU: 100 entries
        event_bus: shared_event_bus.clone(),
        storage_path: std::path::PathBuf::from(&config.server.storage_path),
        llm_config: config.llm.clone(),
        // mlx_server: mlx_server, // Removed
        llama_server: llama_server,
        auth: auth_state,
    }); // Wrapped in Arc

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

    // Graceful Shutdown is now handled natively via `ctrlc::set_handler` 
    // mapped globally inside `storage.register_exit_hook()` in `src/storage/backend.rs`.

    // Start Job Workers
    let worker_count = config.jobs.workers.min(10); // Enforce max 10
    println!("Starting {} Job Workers...", worker_count);
    
    // Register System Heartbeat (Run every 10 seconds)
    if let Err(e) = storage.system_queue.register_cron(
        "heartbeat".to_string(), 
        "0/10 * * * * * *".to_string(), 
        "system_queue".to_string(), 
        b"HEARTBEAT".to_vec()
    ) {
        eprintln!("Failed to register heartbeat cron: {}", e);
    }

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

    // Start TCP Bulk Ingestion Listener on Port 9003
    let bulk_ingest_state = state.clone();
    tokio::spawn(async move {
        crate::server::bulk_ingest::start_tcp_listener(bulk_ingest_state, 9003).await;
    });

    // Start R2 Sync Worker for Cloudflare Backup
    let r2_worker = crate::storage::blob::r2_sync::R2SyncWorker::new(&config, state.clone()).await;
    tokio::spawn(async move {
        r2_worker.start().await;
    });

    // 3. Setup Routes
    // Create Management State
    let mgmt_state = crate::server::management::ManagementState {
        storage: storage.clone(),
        schemas: schemas.clone(),
        event_bus: shared_event_bus.clone(),
        storage_path: std::path::PathBuf::from(&config.server.storage_path),
    };
    
    let mgmt_manager = Arc::new(mgmt_state);
    
    // Convert observability router to Router<()> by providing state immediately
    let obs_router = crate::observability::router::routes::<ServerState>()
        .with_state((*state).clone()); 
        
    // Initialize Blob Storage State
    let blob_state = Arc::new(crate::storage::blob::routes::BlobState::new(&config, state.clone())
        .await
        .expect("Failed to initialize Blob Storage"));

    let mgmt_router = management::router(mgmt_manager.clone())
        .merge(management::ui_router())
        .merge(obs_router);

    let app = Router::new()
        .route("/chat", post(crate::llm::chat_handler))
        .route("/graphql", post(graphql_handler).get(subscription_handler))
        .route("/rpc", get(subscription_handler)) // Support Surrealist native connection
        .route("/playground", get(playground_handler))
        .route("/version", get(version_handler))
        .route("/admin/schema", post(admin_schema_handler))
        .nest_service("/management", mgmt_router)
        .nest_service("/_mgmt", management::router(mgmt_manager.clone()))
        // Integrate Blob Storage / TUS Router
        .nest_service("/files", crate::storage::blob::routes::router(blob_state));

    let app = if state.auth.is_some() {
        use axum::middleware;
        let auth_state = state.auth.clone().unwrap();
        let auth_router = Router::new()
            .route("/auth/register", post(auth::handlers::register::register_user_handler))
            .route("/auth/login", post(auth::handlers::login::login_user_handler))
            .route("/auth/forgot_password", post(auth::handlers::forgot_password::forgot_password_handler))
            .route("/auth/reset_password", post(auth::handlers::reset_password::reset_password_handler))
            .route("/auth/generate_magiclink", post(auth::handlers::generate_magiclink::generate_magiclink_handler))
            .route("/auth/verify_magiclink_code", get(auth::handlers::verify_magiclink::verify_magiclink_code_handler))
            .route("/auth/check_code", post(auth::handlers::check_code::check_code_handler))
            .route("/auth/verify_code", get(auth::handlers::verify_code::verify_code_handler))
            .route("/auth/logout", get(auth::handlers::logout::logout_handler)
                .route_layer(middleware::from_fn_with_state(auth_state.clone(), auth::middleware::auth_middleware)))
            .route("/auth/me", get(auth::handlers::get_me::get_me_handler)
                .route_layer(middleware::from_fn_with_state(auth_state.clone(), auth::middleware::auth_middleware)))
            .with_state(auth_state);
            // We will add remaining logic later
        app.merge(auth_router)
    } else {
        app
    };
    
    let app = app
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state((*state).clone()); 

    (state, app)
}

/// Runs the VardaDB Server. 
/// If `is_mcp` is true in config, it runs the MCP stdio server (blocking).
/// Otherwise it runs the Axum HTTP server.
pub async fn run(config: crate::config::VardaConfig) {
    let port = config.server.port;
    let is_mcp = config.server.is_mcp;
    
    let (state, app) = init_system(config).await;

    // 4. Run Server
    if is_mcp {
        // Run MCP Server
        // Use default schema for MCP for now
        let mcp_schema = state.schemas.get("default").expect("Default schema missing").value().clone();
        
        // We need a resolver. We can create a new one since it's cheap (just Arc clones internally)
        let mcp_resolver = Box::new(FjallResolver::with_bus(state.storage.clone(), state.event_bus.clone()));
        
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
        println!("Management at http://127.0.0.1:{}/management", port);
        
        axum::serve(listener, app).await.expect("Server error");
    }
}


fn extract_db_name(headers: &axum::http::HeaderMap) -> String {
    let name = headers
        .get("x-varda-db")
        .or_else(|| headers.get("DB"))
        .or_else(|| headers.get("db"))
        .or_else(|| headers.get("x-surreal-db"))
        .or_else(|| headers.get("ns")) // Fallback to NS if DB not set? Or maybe the UI sends NS as DB? Let's check DB first.
        .or_else(|| headers.get("NS"))
        .and_then(|val| val.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string());
        
    if name == "sandbox" {
        "archondb".to_string()
    } else {
        name
    }
}

async fn graphql_handler(
    headers: axum::http::HeaderMap,
    axum::extract::State(state): axum::extract::State<ServerState>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let start = std::time::Instant::now();
    counter!("graphql_requests_total").increment(1);
    
    let span = tracing::info_span!("graphql_request", method = "POST");
    let _enter = span.enter();
    
    let db_name = extract_db_name(&headers);

    let schema = if let Some(s) = state.schemas.get(&db_name) {
        s.read().await.clone()
    } else {
        // Lazy Load / Create Schema for DB
        // Check if DB exists in Storage
        if state.storage.get_database(&db_name).is_some() {
             println!("Lazy loading schema for database: {}", db_name);
             let resolver = FjallResolver::new(state.storage.clone(), &db_name);
             
             let db_schema_path = state.storage_path.join(format!("{}_schema.graphql", db_name));
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
            let schema_file_path = state.storage_path.join("current_schema.graphql");
            if let Err(e) = tokio::fs::write(&schema_file_path, &body).await {
                eprintln!("Failed to persist schema to {:?}: {}", schema_file_path, e);
            } else {
                println!("Schema persisted to {:?}", schema_file_path);
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
    let db_name = extract_db_name(&headers);
    
    let _schema_wrapper = if let Some(s) = state.schemas.get(&db_name) {
        s.read().await.clone()
    } else {
        // Fallback to default if not found
        state.schemas.get("default").expect("Default schema missing from map").read().await.clone()
    };

    let _schema = _schema_wrapper.inner().clone();

    let protocol_str = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|val| val.to_str().ok())
        .unwrap_or("graphql-transport-ws"); 
    
    let protocol = if protocol_str.contains("graphql-transport-ws") {
        async_graphql::http::WebSocketProtocols::GraphQLWS
    } else {
        async_graphql::http::WebSocketProtocols::SubscriptionsTransportWS
    };
    
    upgrade
        .protocols(async_graphql::http::ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |socket| async move {
            use futures_util::{StreamExt, SinkExt, future, pin_mut};
            let (mut sink, mut stream) = socket.split();

            // 1. Peek/Read the first message to determine DB (if not in headers)
            let mut buffered_msg = None;
            let mut selected_db = db_name.clone(); // Default from headers

            if let Some(Ok(msg)) = stream.next().await {
                 match &msg {
                    axum::extract::ws::Message::Text(s) => {
                        println!("WS: Received initial message: {}", s);
                        // Attempt to parse ConnectionInit or JSON-RPC "use"
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(s) {
                            // 1. GraphQL-WS: { type: "connection_init", payload: { ... } }
                            if val["type"] == "connection_init" {
                                if let Some(payload) = val.get("payload") {
                                    let ns = payload.get("NS").or(payload.get("ns")).and_then(|v| v.as_str());
                                    let db = payload.get("DB").or(payload.get("db")).and_then(|v| v.as_str());
                                    
                                    if let Some(d) = db { selected_db = d.to_string(); }
                                    else if let Some(n) = ns { selected_db = n.to_string(); }
                                }
                            }
                            // 2. JSON-RPC (Surrealist): { method: "use", params: ["ns", "db"] }
                            else if val["method"] == "use" {
                                if let Some(params) = val.get("params").and_then(|p| p.as_array()) {
                                    // Params: [ns, db]
                                    if params.len() >= 2 {
                                        if let Some(db) = params[1].as_str() {
                                            selected_db = db.to_string();
                                        } else if let Some(ns) = params[0].as_str() {
                                            selected_db = ns.to_string();
                                        }
                                    }
                                }
                            }
                             // 3. JSON-RPC (Surrealist): { method: "signin", params: [{ "NS": "...", "DB": "..." }] }
                             else if val["method"] == "signin" {
                                 if let Some(params) = val.get("params").and_then(|p| p.as_array()) {
                                     if let Some(auth) = params.get(0).and_then(|p| p.as_object()) {
                                         let ns = auth.get("NS").or(auth.get("ns")).and_then(|v| v.as_str());
                                         let db = auth.get("DB").or(auth.get("db")).and_then(|v| v.as_str());
                                         
                                         if let Some(d) = db { selected_db = d.to_string(); }
                                         else if let Some(n) = ns { selected_db = n.to_string(); }
                                     }
                                 }
                             }
                        }
                    },
                    axum::extract::ws::Message::Binary(b) => {
                         println!("WS: Received initial message (Binary): {:?} bytes", b.len());
                    },
                     _ => {
                         println!("WS: Received initial message (Other): {:?}", msg);
                     }
                 }
                 buffered_msg = Some(msg);
            }
            
            if selected_db == "sandbox" {
                selected_db = "archondb".to_string();
            }
            
            println!("WS: Selected DB: {}", selected_db);

            // 2. Select Schema based on extracted DB
            let schema_wrapper = if let Some(s) = state.schemas.get(&selected_db) {
                s.read().await.clone()
            } else {
                // Lazy Load Attempt (Copy logic from graphql_handler? Or just try to load?)
                // For subscriptions, we might want to support lazy loading too.
                // Re-using lazy load logic properly:
                 if state.storage.get_database(&selected_db).is_some() {
                     println!("Lazy loading schema for database (WS): {}", selected_db);
                     let resolver = FjallResolver::new(state.storage.clone(), &selected_db);
                     let db_schema_path = state.storage_path.join(format!("{}_schema.graphql", selected_db));
                     let sdl = std::fs::read_to_string(&db_schema_path).unwrap_or_else(|_| "type Health { status: String }".to_string());
                     
                     if let Ok(new_schema) = crate::engine::schema::Schema::load_with_resolver(&sdl, resolver) {
                         let arc_schema = Arc::new(RwLock::new(Arc::new(new_schema)));
                         state.schemas.insert(selected_db.clone(), arc_schema.clone());
                         let x = arc_schema.read().await.clone();
                         x
                     } else {
                         state.schemas.get("default").expect("Default schema missing").read().await.clone()
                     }
                } else {
                     state.schemas.get("default").expect("Default schema missing").read().await.clone()
                }
            };
            
            let schema = schema_wrapper.inner().clone();

            // 3. Reconstruct Stream (Prepend buffered message)
            let initial_stream = if let Some(msg) = buffered_msg {
                futures_util::stream::once(async move { Ok(msg) }).boxed()
            } else {
                futures_util::stream::empty().boxed()
            };
            
            let combined_stream = initial_stream.chain(stream);

            let msg_stream = combined_stream
                .take_while(|res| future::ready(res.is_ok()))
                .map(|res| res.unwrap())
                .filter_map(|msg| async move {
                    match msg {
                        axum::extract::ws::Message::Text(s) => Some(Vec::from(s.as_bytes())),
                        axum::extract::ws::Message::Binary(b) => Some(b.to_vec()),
                        _ => None,
                    }
                });

            let data_stream = async_graphql::http::WebSocket::new(schema, msg_stream, protocol);
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

async fn version_handler() -> impl IntoResponse {
    let json_response = serde_json::json!({
        "version": "surrealdb-2.0.0", // Mimic SurrealDB version to satisfy UI check
        "ui_version": "0.0.0" // Mimic UI version, match package.json if possible or just 0.0.0
    });
    axum::Json(json_response)
}

pub fn build_schema(sdl: &str) -> Result<crate::engine::schema::Schema, String> {
    crate::engine::schema::Schema::load_from_sdl(sdl)
}
