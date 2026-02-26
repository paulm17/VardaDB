use argon2::{
    password_hash::{PasswordHash, PasswordVerifier},
    Argon2,
};
use axum::{
    extract::State, http::{header, HeaderMap, Response, StatusCode}, response::IntoResponse, Json
};
use axum_extra::extract::cookie::{Cookie, SameSite};
use anyhow::Result;
use ulid::Ulid;


use crate::models::LoginUserSchema;
use crate::state::{UserRecord, TokenRecord};
use crate::token::{generate_paseto_token, TokenKind};

pub async fn login_user_handler(
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    Json(body): Json<LoginUserSchema>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {

    let email = body.email.to_ascii_lowercase();

    // Find user (linear scan since we don't have secondary index synced perfectly yet, fine for MVP)
    let mut found_user = None;
    for kv in auth_state.store.users.iter() {
        if let Ok((_k, v)) = kv.into_inner() {
            if let Ok(user) = serde_json::from_slice::<UserRecord>(&v) {
                if user.email == email {
                    found_user = Some(user);
                    break;
                }
            } else if let Ok(user) = bincode::deserialize::<UserRecord>(&v) {
                if user.email == email {
                    found_user = Some(user);
                    break;
                }
            }
        }
    }

    let user = found_user.ok_or_else(|| {
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "status": "fail",
            "message": "Invalid email or password"
        })))
    })?;

    let password_hash = user.password_hash.as_ref().ok_or_else(|| {
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "status": "fail",
            "message": "Invalid email or password" 
        })))
    })?;

    let is_valid_password = match PasswordHash::new(password_hash) {
        Ok(parsed_hash) => Argon2::default()
            .verify_password(body.password.as_bytes(), &parsed_hash)
            .is_ok(),
        Err(_) => false,
    };

    if !is_valid_password {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "status": "fail",
            "message": "Invalid email or password"
        }))));
    }

    let access_token_details = generate_paseto_token(
        &user.id,
        TokenKind::Access,
        &auth_state,
    ).map_err(|e| {
        tracing::error!("Access token gen failed: {:?}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"status": "fail", "message": "Internal error"})))
    })?;

    let refresh_token_details = generate_paseto_token(
        &user.id,
        TokenKind::Refresh,
        &auth_state,
    ).map_err(|e| {
        tracing::error!("Refresh token gen failed: {:?}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"status": "fail", "message": "Internal error"})))
    })?;

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

    // Save tokens in Fjall store (so we can revoke them if needed/ttl check)
    let access_token_record = TokenRecord {
        id: Ulid::new().to_string(),
        user_id: user.id.clone(),
        token_uuid: access_token_details.token_uuid.to_string(),
        expires_at: access_token_details.expires_in,
    };
    
    let refresh_token_record = TokenRecord {
        id: Ulid::new().to_string(),
        user_id: user.id.clone(),
        token_uuid: refresh_token_details.token_uuid.to_string(),
        expires_at: refresh_token_details.expires_in,
    };

    let a_key = format!("token:{}", access_token_details.token_uuid);
    let r_key = format!("token:{}", refresh_token_details.token_uuid);

    auth_state.store.tokens.insert(a_key.as_bytes(), &serde_json::to_vec(&access_token_record).unwrap()).unwrap();
    auth_state.store.tokens.insert(r_key.as_bytes(), &serde_json::to_vec(&refresh_token_record).unwrap()).unwrap();

    let mut response = Response::new(
        serde_json::json!({"status": "success", "access_token": access_token_details.token})
            .to_string(),
    );
    
    let mut headers = HeaderMap::new();
    headers.append(
        header::SET_COOKIE,
        access_cookie.to_string().parse().unwrap(),
    );
    headers.append(
        header::SET_COOKIE,
        refresh_cookie.to_string().parse().unwrap(),
    );
    headers.append(
        header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );

    response.headers_mut().extend(headers);

    Ok(response)
}
