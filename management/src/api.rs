use axum::{
    extract::{State, Path},
    routing::{post, delete, get},
    Json, Router, http::StatusCode,
};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::traits::DatabaseManager;

#[derive(Clone)]
pub struct ManagementState {
    pub manager: Arc<dyn DatabaseManager>,
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

pub fn router(manager: Arc<dyn DatabaseManager>) -> Router {
    let state = ManagementState { manager };
    Router::new()
        .route("/db", post(create_db).get(list_dbs))
        .route("/db/{name}", delete(delete_db))
        .route("/db/{name}/schema", post(apply_schema).get(get_schema))
        .route("/db/{name}/status", get(get_db_status))
        .with_state(state)
}

async fn create_db(
    State(state): State<ManagementState>,
    Json(payload): Json<CreateDbRequest>,
) -> Result<Json<DbResponse>, (StatusCode, String)> {
    if !payload.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err((StatusCode::BAD_REQUEST, "Invalid database name".to_string()));
    }

    match state.manager.create_db(&payload.name).await {
        Ok(_) => Ok(Json(DbResponse {
            name: payload.name,
            status: "created".to_string(),
        })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn list_dbs(
    State(state): State<ManagementState>,
) -> Result<Json<ListDbsResponse>, (StatusCode, String)> {
    match state.manager.list_dbs().await {
        Ok(dbs) => Ok(Json(ListDbsResponse { databases: dbs })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn delete_db(
    State(state): State<ManagementState>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    match state.manager.delete_db(&name).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) => {
             if e.contains("Cannot delete") {
                Err((StatusCode::FORBIDDEN, e))
            } else {
                 Err((StatusCode::INTERNAL_SERVER_ERROR, e))
            }
        }
    }
}

async fn get_db_status(
    State(state): State<ManagementState>,
    Path(name): Path<String>,
) -> Result<Json<crate::traits::DbStatus>, (StatusCode, String)> {
     match state.manager.get_db_status(&name).await {
         Ok(status) => Ok(Json(status)),
         Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn apply_schema(
    State(state): State<ManagementState>,
    Path(name): Path<String>,
    body: String,
) -> Result<(StatusCode, String), (StatusCode, String)> {
    match state.manager.apply_schema(&name, &body).await {
        Ok(_) => Ok((StatusCode::OK, "Schema applied successfully".to_string())),
        Err(e) => {
             if e.contains("not found") {
                 Err((StatusCode::NOT_FOUND, e))
             } else if e.contains("Invalid Schema") {
                 Err((StatusCode::BAD_REQUEST, e))
             } else {
                 Err((StatusCode::INTERNAL_SERVER_ERROR, e))
             }
        }
    }
}

async fn get_schema(
    State(state): State<ManagementState>,
    Path(name): Path<String>,
) -> Result<String, (StatusCode, String)> {
    match state.manager.get_schema(&name).await {
        Ok(sdl) => Ok(sdl),
        Err(e) => {
             if e.contains("not found") {
                 Err((StatusCode::NOT_FOUND, e))
             } else {
                 Err((StatusCode::INTERNAL_SERVER_ERROR, e))
             }
        }
    }
}
