use axum::{
    extract::{Query, State}, http::{header, StatusCode}, response::{IntoResponse, Redirect}, Json
};
use axum_extra::extract::cookie::{Cookie, SameSite};
use anyhow::Result;
use chrono::Utc;
use ulid::Ulid;


use crate::models::VerifyMagicLinkSchema;
use crate::state::{TokenRecord, ConfirmationRecord, ConfirmationFlow};
use crate::token::{generate_paseto_token, TokenKind};

pub async fn verify_magiclink_code_handler(
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    Query(body): Query<VerifyMagicLinkSchema>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {

    let redirect_to = body.redirect_to.trim().to_string();
    
    // Check allowlist
    if !auth_state.config.allowed_redirect_origins.is_empty() {
        if !auth_state.config.allowed_redirect_origins.contains(&redirect_to) {
            return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "status": "fail",
                 "message": "redirect_to URL is not in the allowed list"
            }))));
        }
    }

    let code = body.code.trim();
    let confirmation_key = format!("confirm:{}", code);

    let confirmation_bytes = auth_state.store.confirmations.get(confirmation_key.as_bytes())
        .map_err(|e| {
             tracing::error!("Auth store error: {:?}", e);
             (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "fail", "message": "Internal error" })))
        })?
        .ok_or_else(|| {
              (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "status": "fail", "message": "Code is invalid or has expired" })))
        })?;

    let mut confirmation: ConfirmationRecord = serde_json::from_slice(&confirmation_bytes).unwrap_or_else(|_| {
        bincode::deserialize(&confirmation_bytes).expect("Failed to deserialize confirmation")
    });

    if confirmation.flow != ConfirmationFlow::Created {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "status": "fail",
            "message": "Code is invalid or has expired"
        }))));
    }

    if confirmation.expires_at < Utc::now().timestamp() {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "status": "fail",
            "message": "Code is invalid or has expired"
        }))));
    }

    let user_id = confirmation.user_id.clone();

    // Generate tokens
    let access_token_details = generate_paseto_token(
        &user_id,
        TokenKind::Access,
        &auth_state,
    ).unwrap();

    let refresh_token_details = generate_paseto_token(
        &user_id,
        TokenKind::Refresh,
        &auth_state,
    ).unwrap();

    let access_token_record = TokenRecord {
        id: Ulid::new().to_string(),
        user_id: user_id.clone(),
        token_uuid: access_token_details.token_uuid.to_string(),
        expires_at: access_token_details.expires_in,
    };
    
    let refresh_token_record = TokenRecord {
        id: Ulid::new().to_string(),
        user_id: user_id.clone(),
        token_uuid: refresh_token_details.token_uuid.to_string(),
        expires_at: refresh_token_details.expires_in,
    };

    let a_key = format!("token:{}", access_token_details.token_uuid);
    let r_key = format!("token:{}", refresh_token_details.token_uuid);

    auth_state.store.tokens.insert(a_key.as_bytes(), &serde_json::to_vec(&access_token_record).unwrap()).unwrap();
    auth_state.store.tokens.insert(r_key.as_bytes(), &serde_json::to_vec(&refresh_token_record).unwrap()).unwrap();

    confirmation.flow = ConfirmationFlow::Completed;
    auth_state.store.confirmations.insert(confirmation_key.as_bytes(), &serde_json::to_vec(&confirmation).unwrap()).unwrap();

    let mut access_cookie = Cookie::new("access_token", access_token_details.token.clone());
    access_cookie.set_path("/");
    access_cookie.set_secure(true);
    access_cookie.set_max_age(time::Duration::minutes(auth_state.config.access_token_ttl_minutes));
    access_cookie.set_same_site(SameSite::Strict);
    access_cookie.set_http_only(true);

    let mut refresh_cookie = Cookie::new("refresh_token", refresh_token_details.token.clone());
    refresh_cookie.set_path("/");
    refresh_cookie.set_secure(true);
    refresh_cookie.set_max_age(time::Duration::days(auth_state.config.refresh_token_ttl_days));
    refresh_cookie.set_same_site(SameSite::Strict);
    refresh_cookie.set_http_only(true);

    let redirect = Redirect::temporary(&redirect_to);
    let mut response = redirect.into_response();

    response.headers_mut().append(
        header::SET_COOKIE,
        access_cookie.to_string().parse().unwrap(),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        refresh_cookie.to_string().parse().unwrap(),
    );

    Ok(response)
}
