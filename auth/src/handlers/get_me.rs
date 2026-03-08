use crate::middleware::JWTAuthMiddleware;
use crate::models::FilteredUser;
use anyhow::Result;
use axum::{response::IntoResponse, Extension, Json};
use reqwest::StatusCode;

pub async fn get_me_handler(
    Extension(jwtauth): Extension<JWTAuthMiddleware>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let user = &jwtauth.user;
    let json_response = serde_json::json!({
        "user": FilteredUser::from(user.clone())
    });

    Ok(Json(json_response))
}
