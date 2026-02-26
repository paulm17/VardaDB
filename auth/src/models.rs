use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[allow(non_snake_case)]
#[derive(Debug, Serialize, Clone)]
pub struct FilteredUser {
    pub id: String,
    pub name: String,
    pub email: String,
    pub verified: bool,
    #[serde(rename = "createdAt")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<crate::state::UserRecord> for FilteredUser {
    fn from(record: crate::state::UserRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            email: record.email,
            verified: record.verified,
            created_at: Some(DateTime::<Utc>::from_timestamp(record.created_at, 0).unwrap_or_default()),
            updated_at: record.updated_at.map(|ts| DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or_default()),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct UserData {
    pub user: FilteredUser,
}

#[derive(Serialize, Debug)]
pub struct UserResponse {
    pub status: String,
    pub data: UserData,
}

#[derive(Debug, Deserialize)]
pub struct RegisterUserSchema {
    pub name: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginUserSchema {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordSchema {
    pub email: String,
    #[serde(rename = "redirectTo")]
    pub redirect_to: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyCodeSchema {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct CheckCodeSchema {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordSchema {
    pub code: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct MagicLinkSchema {
    pub email: String,  
    #[serde(rename = "redirectTo")]
    pub redirect_to: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyMagicLinkSchema {
    pub code: String,
    pub redirect_to: String,
}

#[derive(Debug, Deserialize)]
pub struct OAuthSchema {
    pub provider: String,
    pub scopes: String,
    pub callback_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PkceChallenge {
    pub challenge: String,
    pub method: String,
}
