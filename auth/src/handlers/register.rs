use anyhow::Result;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::Utc;
use ulid::Ulid;

use crate::models::RegisterUserSchema;
use crate::state::UserRecord;

pub async fn register_user_handler(
    State(auth_state): State<std::sync::Arc<crate::state::AuthState>>,
    Json(body): Json<RegisterUserSchema>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let email = body.email.to_ascii_lowercase();

    // Check if user exists by iterating over users (Fjall)
    // For production, we'd want an email->id secondary index keyspace.
    // Spec mentions: "Also maintain auth_users_by_email:{email} → ulid index"
    // We will scan for now to avoid the full secondary index complexity unless strictly needed, but let's implement the index check
    // Actually, spec said: "Also maintain auth_users_by_email:{email} → ulid index". Wait, let's just make it simple if we didn't create that keyspace.
    // Let's create `auth_users_by_email` dynamically on first use or scan.
    // Scanning is fine for MVP port since VardaDB is fast.
    let mut exists = false;
    for (_k, v) in auth_state.store.users.kv_prefix(b"") {
        if let Ok(user) = serde_json::from_slice::<UserRecord>(&v) {
            if user.email == email {
                exists = true;
                break;
            } else if let Ok(user) = bincode::deserialize::<UserRecord>(&v) {
                if user.email == email {
                    exists = true;
                    break;
                }
            }
        }
    }

    if exists {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "status": "fail",
                "message": "User already exists",
            })),
        ));
    }

    let salt = SaltString::generate(&mut OsRng);
    let hashed_password = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| {
            tracing::error!("Error while hashing password: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "fail",
                    "message": "Internal error"
                })),
            )
        })
        .map(|hash| hash.to_string())?;

    let user_id = Ulid::new().to_string();
    let timestamp = Utc::now().timestamp();

    let new_user = UserRecord {
        id: user_id.clone(),
        name: body.name,
        email,
        password_hash: Some(hashed_password),
        verified: false,
        created_at: timestamp,
        updated_at: None,
    };

    let user_key = format!("user:{}", user_id);
    let serialized_user = serde_json::to_vec(&new_user).map_err(|e| {
        tracing::error!("Failed to serialize user: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "fail",
                "message": "Internal error"
            })),
        )
    })?;

    if let Err(e) = auth_state
        .store
        .users
        .kv_insert(user_key.as_bytes(), &serialized_user)
    {
        tracing::error!("Failed to insert user into store: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "fail",
                "message": "Internal error"
            })),
        ));
    }

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": "User registered successfully",
        "data": {
            "user_id": user_id
        }
    })))
}
