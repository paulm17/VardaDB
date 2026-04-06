use crate::traits::{BackupInfo, DatabaseManager};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct ManagementState {
    pub manager: Arc<dyn DatabaseManager>,
}

#[derive(Deserialize)]
pub struct CreateDbRequest {
    pub name: String,
}

#[derive(Deserialize)]
pub struct UpdatePathRequest {
    pub path: String,
}

#[derive(Deserialize)]
pub struct RestoreRequest {
    pub backup_id: String,
}

#[derive(Serialize)]
pub struct DbResponse {
    pub name: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct ListDbsResponse {
    pub databases: Vec<crate::traits::DbInfo>,
}

#[derive(Serialize)]
pub struct BackupResponse {
    pub backup_id: String,
}

#[derive(Serialize)]
pub struct ListBackupsResponse {
    pub backups: Vec<BackupInfo>,
}

pub fn router(manager: Arc<dyn DatabaseManager>) -> Router {
    let state = ManagementState { manager };
    Router::new()
        .route("/db", post(create_db).get(list_dbs))
        .route("/db/{name}", delete(delete_db))
        .route("/db/{name}/path", post(update_db_path))
        .route("/db/{name}/schema", post(apply_schema).get(get_schema))
        .route("/db/{name}/status", get(get_db_status))
        .route("/backup", post(create_backup).get(list_backups))
        .route("/restore", post(restore_from_backup))
        .with_state(state)
}

async fn create_db(
    State(state): State<ManagementState>,
    Json(payload): Json<CreateDbRequest>,
) -> Result<Json<DbResponse>, (StatusCode, String)> {
    if !payload
        .name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
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

async fn update_db_path(
    State(state): State<ManagementState>,
    Path(name): Path<String>,
    Json(payload): Json<UpdatePathRequest>,
) -> Result<(StatusCode, String), (StatusCode, String)> {
    match state.manager.update_db_path(&name, &payload.path).await {
        Ok(_) => Ok((StatusCode::OK, "Path updated successfully".to_string())),
        Err(e) => {
            if e.contains("Cannot update")
                || e.contains("does not exist")
                || e.contains("missing registry entry")
            {
                Err((StatusCode::BAD_REQUEST, e))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, e))
            }
        }
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

async fn create_backup(
    State(state): State<ManagementState>,
) -> Result<Json<BackupResponse>, (StatusCode, String)> {
    match state.manager.create_backup().await {
        Ok(backup_id) => Ok(Json(BackupResponse { backup_id })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn restore_from_backup(
    State(state): State<ManagementState>,
    Json(payload): Json<RestoreRequest>,
) -> Result<(StatusCode, String), (StatusCode, String)> {
    match state.manager.restore_from_backup(&payload.backup_id).await {
        Ok(_) => Ok((StatusCode::OK, "Restore completed successfully. Restart server to apply changes.".to_string())),
        Err(e) => {
            if e.contains("not found") {
                Err((StatusCode::NOT_FOUND, e))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, e))
            }
        }
    }
}

async fn list_backups(
    State(state): State<ManagementState>,
) -> Result<Json<ListBackupsResponse>, (StatusCode, String)> {
    match state.manager.list_backups().await {
        Ok(backups) => Ok(Json(ListBackupsResponse { backups })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}
