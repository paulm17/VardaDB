use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use axum::{
    extract::State, http::{header, HeaderMap, Response, StatusCode}, response::IntoResponse, Json
};
use anyhow::Result;
use chrono::Utc;

use crate::models::ResetPasswordSchema;
use crate::state::{UserRecord, ConfirmationRecord, ConfirmationFlow};

pub async fn reset_password_handler(
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    Json(body): Json<ResetPasswordSchema>,
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
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "status": "fail",
            "message": "verification code has expired"
        }))));
    }

    // Hash the new password
    let salt = SaltString::generate(&mut OsRng);
    let hashed_password = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| {
            tracing::error!("Error while hashing password: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "status": "fail",
                "message": "Internal error"
            })))
        })
        .map(|hash| hash.to_string())?;

    // Load User
    let user_key = format!("user:{}", confirmation.user_id);
    let user_bytes = auth_state.store.users.get(user_key.as_bytes())
        .map_err(|e| {
             tracing::error!("Auth store error: {:?}", e);
             (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "fail", "message": "Internal error" })))
        })?
        .ok_or_else(|| {
              (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "status": "fail", "message": "User not found" })))
        })?;

    let mut user: UserRecord = serde_json::from_slice(&user_bytes).unwrap_or_else(|_| {
        bincode::deserialize(&user_bytes).expect("Failed to deserialize user")
    });

    // Update user record
    user.password_hash = Some(hashed_password);
    user.updated_at = Some(Utc::now().timestamp());

    auth_state.store.users.insert(user_key.as_bytes(), &serde_json::to_vec(&user).unwrap()).map_err(|e| {
         tracing::error!("Failed to update user: {:?}", e);
         (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "fail", "message": "Internal error" })))
    })?;

    // Update confirmation flow
    confirmation.flow = ConfirmationFlow::Completed;
    auth_state.store.confirmations.insert(confirmation_key.as_bytes(), &serde_json::to_vec(&confirmation).unwrap()).map_err(|e| {
         tracing::error!("Failed to update confirmation: {:?}", e);
         (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "fail", "message": "Internal error" })))
    })?;

    let mut response = Response::new(
        serde_json::json!({"status": "ok"}).to_string(),
    );

    let mut headers = HeaderMap::new();
    headers.append(
        header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );

    response.headers_mut().extend(headers);

    Ok(response)
}
