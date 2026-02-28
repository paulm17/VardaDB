use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::IntoResponse,
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Serialize;

use crate::token::{self, TokenKind};
use crate::state::UserRecord;

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub status: &'static str,
    pub message: String,
}

#[derive(Clone)]
pub struct JWTAuthMiddleware {
    pub user: UserRecord,
    pub access_token_uuid: uuid::Uuid,
}

pub async fn auth_middleware(
    cookie_jar: CookieJar,
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    mut req: Request,
    next: Next,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {


    let access_token = cookie_jar
        .get("access_token")
        .map(|cookie| cookie.value().to_string())
        .or_else(|| {
            req.headers()
                .get(header::AUTHORIZATION)
                .and_then(|auth_header| auth_header.to_str().ok())
                .and_then(|auth_value| {
                    if auth_value.starts_with("Bearer ") {
                        Some(auth_value[7..].to_owned())
                    } else {
                        None
                    }
                })
        });

    let access_token = access_token.ok_or_else(|| {
        let err = ErrorResponse {
            status: "fail",
            message: "You are not logged in, please provide token".to_string(),
        };
        (StatusCode::UNAUTHORIZED, Json(err))
    })?;

    let token_details = match token::verify_paseto_token(&access_token, TokenKind::Access, &auth_state) {
        Ok(details) => details,
        Err(e) => {
            tracing::warn!("Token verification failed: {:?}", e);
            let err = ErrorResponse {
                status: "fail",
                message: "Invalid or expired token".to_string(), // Spec Fix 4: No internal info in body
            };
            return Err((StatusCode::UNAUTHORIZED, Json(err)));
        }
    };

    // Bug Fix 2: Token blacklist check in middleware
    let token_key = format!("token:{}", token_details.token_uuid);
    let token_exists = auth_state.store.tokens.kv_get(token_key.as_bytes())
        .map_err(|e| {
            tracing::error!("Auth store error: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { status: "fail", message: "Internal error".to_string() }))
        })?
        .is_some();

    if !token_exists {
         let err = ErrorResponse {
             status: "fail",
             message: "Token has been revoked".to_string(),
         };
         return Err((StatusCode::UNAUTHORIZED, Json(err)));
    }

    // Fetch user from store
    let user_key = format!("user:{}", token_details.user_id);
    let user_bytes = auth_state.store.users.kv_get(user_key.as_bytes())
        .map_err(|e| {
            tracing::error!("Auth store error: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { status: "fail", message: "Internal error".to_string() }))
        })?
        .ok_or_else(|| {
            let err = ErrorResponse { status: "fail", message: "User not found".to_string() };
            (StatusCode::UNAUTHORIZED, Json(err)) // Unauthorized since user was deleted
        })?;

    // Deserialize directly via serde_json as per spec recommendation "use serde_json for debuggability"
    let user: UserRecord = match serde_json::from_slice(&user_bytes) {
        Ok(u) => u,
        Err(_) => bincode::deserialize(&user_bytes).map_err(|e| {
            tracing::error!("Auth store deserialization error: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { status: "fail", message: "Internal error".to_string() }))
        })?
    };

    req.extensions_mut().insert(JWTAuthMiddleware {
        user,
        access_token_uuid: token_details.token_uuid,
    });

    Ok(next.run(req).await)
}
