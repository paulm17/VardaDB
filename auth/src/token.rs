use crate::state::AuthState;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rusty_paseto::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind { Access, Refresh }

#[derive(Debug)]
pub struct TokenDetails {
    pub user_id: String,
    pub token_uuid: Uuid,
    pub expires_in: i64,
    pub token: String,
}

#[derive(Debug)]
pub struct TokenClaims {
    pub sub: String,
    pub token_uuid: String,
    pub exp: i64,
    pub iat: i64,
    pub nbf: i64,
}

pub fn generate_paseto_token(
    user_id: &str,
    kind: TokenKind,
    state: &AuthState,
) -> Result<TokenDetails> {
    let key_bytes = match kind {
        TokenKind::Access => &state.access_key,
        TokenKind::Refresh => &state.refresh_key,
    };
    
    let private_key = PasetoAsymmetricPrivateKey::<V4, Public>::from(key_bytes.as_slice());

    let token_uuid = Uuid::new_v4();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    
    let ttl_minutes = match kind {
        TokenKind::Access => state.config.access_token_ttl_minutes,
        TokenKind::Refresh => state.config.refresh_token_ttl_days * 24 * 60,
    };
    let exp = now + (ttl_minutes * 60);

    let claims = TokenClaims {
        sub: user_id.to_string(),
        token_uuid: token_uuid.to_string(),
        exp,
        iat: now,
        nbf: now,
    };

    let exp_datetime: DateTime<Utc> = DateTime::<Utc>::from_timestamp(exp, 0).ok_or_else(|| anyhow!("Invalid timestamp"))?;
    let iat_datetime: DateTime<Utc> = DateTime::<Utc>::from_timestamp(now, 0).ok_or_else(|| anyhow!("Invalid timestamp"))?;
    let nbf_datetime: DateTime<Utc> = DateTime::<Utc>::from_timestamp(now, 0).ok_or_else(|| anyhow!("Invalid timestamp"))?;

    let token = PasetoBuilder::<V4, Public>::default()
        .set_claim(SubjectClaim::from(claims.sub.as_str()))
        .set_claim(CustomClaim::try_from(("token_uuid", claims.token_uuid.clone()))?)
        .set_claim(ExpirationClaim::try_from(exp_datetime.to_rfc3339())?)
        .set_claim(IssuedAtClaim::try_from(iat_datetime.to_rfc3339())?)
        .set_claim(NotBeforeClaim::try_from(nbf_datetime.to_rfc3339())?)
        .build(&private_key)?;

    Ok(TokenDetails {
        user_id: user_id.to_string(),
        token_uuid,
        expires_in: exp,
        token,
    })
}

pub fn verify_paseto_token(
    token: &str,
    kind: TokenKind,
    state: &AuthState,
) -> Result<TokenDetails> {
    let key_bytes = match kind {
        TokenKind::Access => &state.access_key,
        TokenKind::Refresh => &state.refresh_key,
    };
    
    let mut key_data = [0u8; 32];
    key_data.copy_from_slice(&key_bytes[32..]);

    let key = Key::<32>::from(key_data);
    let public_key = PasetoAsymmetricPublicKey::<V4, Public>::from(&key);

    let mut parser = PasetoParser::<V4, Public>::default();
    
    let parsed_token = parser.parse(token, &public_key)
        .map_err(|e| anyhow!("Failed to parse token: {}", e))?;

    let claims = parsed_token.as_object().ok_or_else(|| anyhow!("Invalid token structure"))?;

    let sub = claims.get("sub")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing subject claim"))?
        .to_string();

    let token_uuid = claims.get("token_uuid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing token_uuid claim"))?;

    let parsed_expires_in = if let Some(exp) = claims.get("exp") {
        match exp {
            serde_json::Value::String(exp_str) => {
                let exp_datetime = chrono::DateTime::parse_from_rfc3339(exp_str)
                    .map_err(|_| anyhow!("Could not parse exp claim as RFC3339: {}", exp_str))?;
                if exp_datetime < chrono::Utc::now() {
                    return Err(anyhow!("Token has expired"));
                }
                exp_datetime.timestamp()
            },
            serde_json::Value::Number(num) => {
                let exp_timestamp = num.as_i64().ok_or_else(|| anyhow!("exp claim is not a valid i64"))?;
                if exp_timestamp < chrono::Utc::now().timestamp() {
                    return Err(anyhow!("Token has expired"));
                }
                exp_timestamp
            },
            _ => return Err(anyhow!("exp claim has unexpected format")),
        }
    } else {
        return Err(anyhow!("Missing exp claim"));
    };

    // Nbf checking
    if let Some(nbf) = claims.get("nbf") {
        match nbf {
            serde_json::Value::String(nbf_str) => {
                let nbf_datetime = chrono::DateTime::parse_from_rfc3339(nbf_str)
                    .map_err(|_| anyhow!("Could not parse nbf claim as RFC3339: {}", nbf_str))?;
                if nbf_datetime > chrono::Utc::now() {
                    return Err(anyhow!("Token not yet valid"));
                }
            },
            serde_json::Value::Number(num) => {
                let nbf_timestamp = num.as_i64().ok_or_else(|| anyhow!("nbf claim is not a valid i64"))?;
                if nbf_timestamp > chrono::Utc::now().timestamp() {
                    return Err(anyhow!("Token not yet valid"));
                }
            },
            _ => return Err(anyhow!("nbf claim has unexpected format")),
        }
    } else {
        return Err(anyhow!("Missing nbf claim"));
    }

    Ok(TokenDetails {
        token: token.to_string(),
        token_uuid: Uuid::parse_str(token_uuid)?,
        user_id: sub,
        expires_in: parsed_expires_in,
    })
}
