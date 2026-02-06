use axum::{
    extract::{State, Path},
    routing::{post, delete},
    Json, Router, http::StatusCode,
};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::storage::backend::Storage;

#[derive(Clone)]
pub struct ManagementState {
    pub storage: Arc<Storage>,
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

pub fn routes(storage: Arc<Storage>) -> Router {
    let state = ManagementState { storage };
    
    Router::new()
        .route("/db", post(create_db).get(list_dbs))
        .route("/db/{name}", delete(delete_db))
        .with_state(state)
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
        Ok(_) => Ok(Json(DbResponse {
            name: payload.name,
            status: "created".to_string(),
        })),
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
