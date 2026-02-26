use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AuthConfig {
    pub server_url: String,
    pub access_token_ttl_minutes: i64,   // default: 15
    pub refresh_token_ttl_days: i64,     // default: 30
    pub allowed_redirect_origins: Vec<String>,
    
    pub smtp: Option<SmtpConfig>,
    pub social: Option<SocialConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmtpConfig {
    pub server: String,
    pub port: u16,
    pub tls_mode: SmtpTlsMode,
    pub from: String,
    pub from_name: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub max_attempts: u32,
    pub initial_delay_secs: u64,
    pub max_delay_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SmtpTlsMode {
    StartTls,
    ImplicitTls,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SocialConfig {
    pub callback_redirect_base: String,
    pub google: Option<OAuthProviderConfig>,
    pub github: Option<OAuthProviderConfig>,
    pub microsoft: Option<OAuthProviderConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuthProviderConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
}
