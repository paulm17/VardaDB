use anyhow::Result;
use axum::{
    extract::State,
    http::{header, HeaderMap, Response, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{Duration, Utc};
use ulid::Ulid;

use crate::handlers::forgot_password::generate_random_string;
use crate::models::MagicLinkSchema;
use crate::state::{ConfirmationFlow, ConfirmationRecord, UserRecord};

pub async fn generate_magiclink_handler(
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    Json(body): Json<MagicLinkSchema>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let email = body.email.to_ascii_lowercase();
    let redirect_to = body.redirect_to.trim().to_string();

    // Check allowlist
    if !auth_state.config.allowed_redirect_origins.is_empty() {
        if !auth_state
            .config
            .allowed_redirect_origins
            .contains(&redirect_to)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "status": "fail",
                     "message": "redirect_to URL is not in the allowed list"
                })),
            ));
        }
    }

    // Find User
    let mut found_user = None;
    for (_k, v) in auth_state.store.users.kv_prefix(b"") {
        if let Ok(user) = serde_json::from_slice::<UserRecord>(&v) {
            if user.email == email {
                found_user = Some(user);
                break;
            } else if let Ok(user) = bincode::deserialize::<UserRecord>(&v) {
                if user.email == email {
                    found_user = Some(user);
                    break;
                }
            }
        }
    }

    let user = found_user.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "fail",
                "message": "User email does not exist"
            })),
        )
    })?;

    let code = generate_random_string();
    let expires = (Utc::now() + Duration::days(10)).timestamp();

    let confirmation = ConfirmationRecord {
        id: Ulid::new().to_string(),
        user_id: user.id.clone(),
        code: code.clone(),
        redirect_to: Some(redirect_to.clone()),
        flow: ConfirmationFlow::Created,
        expires_at: expires,
    };

    let confirmation_key = format!("confirm:{}", code);
    let serialized_confirmation = serde_json::to_vec(&confirmation).unwrap();

    if let Err(e) = auth_state
        .store
        .confirmations
        .kv_insert(confirmation_key.as_bytes(), &serialized_confirmation)
    {
        tracing::error!("Failed to save magic link code: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                 "status": "fail",
                 "message": "Failed to save confirmation"
            })),
        ));
    }

    // Phase-0 note: this path intentionally does not dispatch email anymore.
    // It should be replaced by Restate-backed delivery when the new runtime lands.
    let email_sent = if let Some(_) = auth_state.as_ref().config.smtp.as_ref() {
        tracing::warn!(
            "SMTP is configured, but asynchronous email dispatch is currently disabled. Magic link generated but not sent: {}",
            code
        );
        false
    } else {
        tracing::warn!(
            "Auth email is disabled! Magic link generated but not sent: {}",
            code
        );
        false
    };

    let mut headers = HeaderMap::new();
    headers.append(header::CONTENT_TYPE, "application/json".parse().unwrap());

    let mut response = Response::new(
        serde_json::json!({
            "status": if email_sent { "success" } else { "fail" },
            "message": if email_sent { "Email sent" } else { "Email dispatch disabled/failed" }
        })
        .to_string(),
    );

    response.headers_mut().extend(headers);
    Ok(response)
}
