use axum::{
    extract::State, http::{header, HeaderMap, Response, StatusCode}, response::IntoResponse, Json
};
use anyhow::Result;


use crate::models::CheckCodeSchema;
use crate::state::{ConfirmationRecord, ConfirmationFlow};

pub async fn check_code_handler(
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    Json(body): Json<CheckCodeSchema>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {

    let code = body.code.trim();
    let confirmation_key = format!("confirm:{}", code);

    let confirmation_bytes = auth_state.store.confirmations.kv_get(confirmation_key.as_bytes())
        .map_err(|e| {
             tracing::error!("Auth store error: {:?}", e);
             (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "fail", "message": "Internal error" })))
        })?
        .ok_or_else(|| {
              (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "status": "fail", "message": "Code is invalid or has expired" })))
        })?;

    let confirmation: ConfirmationRecord = serde_json::from_slice(&confirmation_bytes).unwrap_or_else(|_| {
        bincode::deserialize(&confirmation_bytes).expect("Failed to deserialize confirmation")
    });

    if confirmation.flow != ConfirmationFlow::Seen {
        return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "is_valid": false
        }))));
    }

    let mut response = Response::new(
        serde_json::json!({"is_valid": true}).to_string(),
    );

    let mut headers = HeaderMap::new();
    headers.append(
        header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );

    response.headers_mut().extend(headers);

    Ok(response)
}
