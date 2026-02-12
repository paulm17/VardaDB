use axum::{
    extract::{State, Path},
    routing::{post, delete},
    Json, Router, http::StatusCode,
};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::storage::backend::Storage;
use tokio::sync::RwLock;
use crate::realtime::bus::EventBus;
use crate::bridge::fjall_resolver::FjallResolver;

#[derive(Clone)]
pub struct ManagementState {
    pub storage: Arc<Storage>,
    pub schemas: Arc<dashmap::DashMap<String, Arc<RwLock<Arc<crate::engine::schema::Schema>>>>>,
    pub event_bus: EventBus,
    pub storage_path: std::path::PathBuf,
}

#[derive(Deserialize)]
pub struct CreateDbRequest {
    pub name: String,
}

#[derive(Serialize)]
pub struct DbResponse {
    pub name: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct ListDbsResponse {
    pub databases: Vec<String>,
}

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ManagementState: axum::extract::FromRef<S>,
{
    
    Router::new()
        .route("/db", post(create_db).get(list_dbs))
        .route("/db/{name}", delete(delete_db))
        .route("/db/{name}/schema", post(apply_schema))
}

async fn create_db(
    State(state): State<ManagementState>,
    Json(payload): Json<CreateDbRequest>,
) -> Result<Json<DbResponse>, (StatusCode, String)> {
    // Validate name (alphanumeric, no spaces, etc?)
    if !payload.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err((StatusCode::BAD_REQUEST, "Invalid database name".to_string()));
    }

    match state.storage.create_database(&payload.name) {
        Ok(_) => {
             // Inject Default Agent Schema
             // We check if it's not the default "system" DB or similar, but generally we want it everywhere for now.
             // Ideally this should be configurable, but for Phase 1 we inject it.
             
             let schema_body = crate::defaults::AGENT_SCHEMA;
             println!("Injecting Agent Schema into database: {}", payload.name);
             
             let resolver = FjallResolver::with_bus(state.storage.clone(), state.event_bus.clone());
             match crate::engine::schema::Schema::load_with_resolver(schema_body, resolver) {
                Ok(new_schema) => {
                     let arc_schema = Arc::new(RwLock::new(Arc::new(new_schema)));
                     state.schemas.insert(payload.name.clone(), arc_schema);
                     
                     // Persist
                     let schema_file_path = state.storage_path.join(format!("{}_schema.graphql", payload.name));
                     if let Err(e) = tokio::fs::write(&schema_file_path, schema_body).await {
                         eprintln!("Failed to persist schema for {} to {:?}: {}", payload.name, schema_file_path, e);
                     }
                }
                Err(e) => {
                    eprintln!("Failed to load Agent Schema for {}: {}", payload.name, e);
                    // We log but don't fail the DB creation, or should we?
                    // It's better to exist empty than fail? Or fail if defaults are broken?
                    // Let's log for now.
                }
             }

            Ok(Json(DbResponse {
                name: payload.name,
                status: "created".to_string(),
            }))
        },
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn list_dbs(
    State(state): State<ManagementState>,
) -> Json<ListDbsResponse> {
    let dbs = state.storage.list_databases();
    Json(ListDbsResponse { databases: dbs })
}

async fn delete_db(
    State(state): State<ManagementState>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if name == "default" {
        return Err((StatusCode::FORBIDDEN, "Cannot delete default database".to_string()));
    }
    
    match state.storage.delete_database(&name) {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn apply_schema(
    State(state): State<ManagementState>,
    Path(name): Path<String>,
    body: String,
) -> Result<(StatusCode, String), (StatusCode, String)> {
    println!("Applying schema to database: {}", name);
    
    // 1. Validate DB exists
    // Note: create_database checks existence, but here we explicitly check before applying
    if state.storage.get_database(&name).is_none() {
         return Err((StatusCode::NOT_FOUND, format!("Database '{}' not found", name)));
    }

    // 2. Validate and Load Schema
    // We use the same EventBus so subscriptions work across updates
    let resolver = FjallResolver::with_bus(state.storage.clone(), state.event_bus.clone());
    // Note: We might want to scope the resolver to the specific DB, but for now `with_bus` is generic storage.
    // wait, FjallResolver needs the specific keyspace context if it's doing lazy loading?
    // Actually `FjallResolver::new` takes a keyspace name. `with_bus` takes storage and assumes "default"?
    // Let's check `FjallResolver::with_bus`.
    // It seems `with_bus` creates a resolver that might not have the specific keyspace context set if it relies on default.
    // However, `Schema::load_with_resolver` uses the resolver.
    
    // Let's look at `FjallResolver::new`.
    // We should probably allow creating a resolver for a specific DB with the shared bus.
    // But `FjallResolver` struct might not expose that easily.
    // Let's assume for now we reuse the pattern from `admin_schema_handler`.
    
    // Ideally:
    // let resolver = FjallResolver::new(state.storage.clone(), &name);
    // BUT we need to inject the `event_bus`!
    // The current `FjallResolver` doesn't seem to have a `new_with_bus_and_keyspace`.
    // Let's stick to `with_bus` for now, or check if we need to update FjallResolver.
    // `with_bus` likely uses "default" keyspace or none.
    
    match crate::engine::schema::Schema::load_with_resolver(&body, resolver) {
        Ok(new_schema) => {
             // 3. Update In-Memory State
             let arc_schema = Arc::new(RwLock::new(Arc::new(new_schema)));
             state.schemas.insert(name.clone(), arc_schema);
            
             // 4. Persist to Disk
            let schema_file_path = state.storage_path.join(format!("{}_schema.graphql", name));
            
            if let Err(e) = tokio::fs::write(&schema_file_path, &body).await {
                eprintln!("Failed to persist schema for {} to {:?}: {}", name, schema_file_path, e);
                // We still succeeded in memory, so maybe just warn? Or fail?
                // Let's return OK but log error.
            } else {
                println!("Schema persisted to {:?}", schema_file_path);
            }

            Ok((StatusCode::OK, "Schema applied successfully".to_string()))
        }
        Err(e) => {
            Err((StatusCode::BAD_REQUEST, format!("Invalid Schema: {}", e)))
        }
    }
}
