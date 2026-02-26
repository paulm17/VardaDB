use axum::{
    extract::State, http::{header, HeaderMap, Response, StatusCode}, response::IntoResponse, Json
};
use axum_extra::extract::{
    cookie::{Cookie, SameSite},
    CookieJar,
};
use anyhow::Result;



pub async fn logout_handler(
    cookie_jar: CookieJar,
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {

    let mut access_cookie = Cookie::new("access_token", "");
    access_cookie.set_path("/");
    access_cookie.set_secure(true);
    access_cookie.set_max_age(time::Duration::minutes(-1));
    access_cookie.set_same_site(SameSite::Strict);
    access_cookie.set_http_only(true);

    let mut refresh_cookie = Cookie::new("refresh_token", "");
    refresh_cookie.set_path("/");
    refresh_cookie.set_secure(true);
    refresh_cookie.set_max_age(time::Duration::minutes(-1));
    refresh_cookie.set_same_site(SameSite::Strict);
    refresh_cookie.set_http_only(true);

    let access_token = cookie_jar
        .get("access_token")
        .map(|cookie| cookie.value().to_string());

    let refresh_token = cookie_jar
        .get("refresh_token")
        .map(|cookie| cookie.value().to_string());

    // Fix: We don't have blacklist_token function ported directly. We instead look up the token uuid via verify and delete it from Fjall
    // Actually, to fully blacklist without verifying, we can just let PASETO expire or we scan for the exact token string (auth_tokens contains token_uuid).
    // Wait, the TokenRecord in Fjall doesn't store the raw string token, it stores the token_uuid as the key (`token:{token_uuid}`).
    // Let's implement an inline verify to get the UUID, then delete the key.
    use crate::token::{verify_paseto_token, TokenKind};
    
    if let Some(token) = access_token {
        if let Ok(details) = verify_paseto_token(&token, TokenKind::Access, &auth_state) {
            let key = format!("token:{}", details.token_uuid);
            let _ = auth_state.store.tokens.remove(key.as_bytes());
        }
    }

    if let Some(token) = refresh_token {
        if let Ok(details) = verify_paseto_token(&token, TokenKind::Refresh, &auth_state) {
            let key = format!("token:{}", details.token_uuid);
            let _ = auth_state.store.tokens.remove(key.as_bytes());
        }
    }

    let mut headers = HeaderMap::new();
    headers.append(
        header::SET_COOKIE,
        access_cookie.to_string().parse().unwrap(),
    );
    headers.append(
        header::SET_COOKIE,
        refresh_cookie.to_string().parse().unwrap(),
    );

    let mut response = Response::new(serde_json::json!({"status": "success"}).to_string());
    response.headers_mut().extend(headers);

    Ok(response)
}
