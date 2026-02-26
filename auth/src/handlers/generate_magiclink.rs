use axum::{
    extract::State, http::{header, HeaderMap, Response, StatusCode}, response::IntoResponse, Json
};
use anyhow::Result;
use ulid::Ulid;
use chrono::{Duration, Utc};


use crate::models::MagicLinkSchema;
use crate::state::{UserRecord, ConfirmationRecord, ConfirmationFlow};
use crate::handlers::forgot_password::generate_random_string;

pub async fn generate_magiclink_handler(
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    Json(body): Json<MagicLinkSchema>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {


    let email = body.email.to_ascii_lowercase();
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

    // Find User
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
            "message": "User email does not exist"
        })))
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

    if let Err(e) = auth_state.store.confirmations.insert(confirmation_key.as_bytes(), &serialized_confirmation) {
        tracing::error!("Failed to save magic link code: {}", e);
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
             "status": "fail",
             "message": "Failed to save confirmation"
        }))));
    }

    let email_sent = if let Some(_) = auth_state.as_ref().config.smtp.as_ref() {
        if let Some(queue) = &auth_state.as_ref().email_queue {
            use jobs::Job;
            use chrono::Utc;
            
            let job_payload = serde_json::json!({
                "type": "magic_link",
                "email": email,
                "code": code,
                "redirect_to": redirect_to
            });
            
            let job = Job::new(
                Utc::now().timestamp_millis() as u64 + rand::Rng::gen_range(&mut rand::thread_rng(), 1..100000), 
                "auth_email".to_string(), 
                job_payload.to_string().into_bytes()
            );
            
            let _ = queue.push(job);
            true
        } else {
            tracing::warn!("Auth email is disabled! Magic link generated but not sent: {}", code);
            false
        }
    } else {
        tracing::warn!("Auth email is disabled! Magic link generated but not sent: {}", code);
        false
    };

    let mut headers = HeaderMap::new();
    headers.append(header::CONTENT_TYPE, "application/json".parse().unwrap());

    let mut response = Response::new(
        serde_json::json!({
            "status": if email_sent { "success" } else { "fail" },
            "message": if email_sent { "Email sent" } else { "Email dispatch disabled/failed" }
        }).to_string(),
    );

    response.headers_mut().extend(headers);
    Ok(response)
}
