use axum::{
    extract::{Query, State}, http::StatusCode, response::{IntoResponse, Redirect}, Json
};
use anyhow::Result;
use chrono::Utc;


use crate::models::VerifyCodeSchema;
use crate::state::{ConfirmationRecord, ConfirmationFlow};

pub async fn verify_code_handler(
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    Query(body): Query<VerifyCodeSchema>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {

    let code = body.code.trim();
    let confirmation_key = format!("confirm:{}", code);

    let confirmation_bytes = auth_state.store.confirmations.get(confirmation_key.as_bytes())
        .map_err(|e| {
             tracing::error!("Auth store error: {:?}", e);
             (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "fail", "message": "Internal error" })))
        })?
        .ok_or_else(|| {
              (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "status": "fail", "message": "verification code does not exist" })))
        })?;

    let mut confirmation: ConfirmationRecord = serde_json::from_slice(&confirmation_bytes).unwrap_or_else(|_| {
        bincode::deserialize(&confirmation_bytes).expect("Failed to deserialize confirmation")
    });

    if confirmation.flow != ConfirmationFlow::Created {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "status": "fail",
            "message": "verification code has already been used"
        }))));
    }

    if confirmation.expires_at < Utc::now().timestamp() {
        return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "status": "fail",
            "message": "Code is invalid or has expired"
        }))));
    }

    confirmation.flow = ConfirmationFlow::Seen;
    auth_state.store.confirmations.insert(confirmation_key.as_bytes(), &serde_json::to_vec(&confirmation).unwrap()).unwrap();

    let redirect_url = confirmation.redirect_to.unwrap_or_else(|| "/".to_string());
    
    // Check allowlist
    if !auth_state.config.allowed_redirect_origins.is_empty() {
        if !auth_state.config.allowed_redirect_origins.contains(&redirect_url) {
            return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "status": "fail",
                 "message": "redirect_to URL is not in the allowed list"
            }))));
        }
    }

    // Usually frontend expects "?code=XYZ"
    Ok(Redirect::temporary(&format!("{}?code={}", redirect_url, confirmation.code)))
}
