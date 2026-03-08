use ed25519_dalek::SigningKey;
use jobs::KvStore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserRecord {
    pub id: String,
    pub name: String,
    pub email: String,
    pub password_hash: Option<String>,
    pub verified: bool,
    pub created_at: i64,
    pub updated_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TokenRecord {
    pub id: String,
    pub user_id: String,
    pub token_uuid: String,
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ConfirmationFlow {
    Created,
    Seen,
    Completed,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfirmationRecord {
    pub id: String,
    pub user_id: String,
    pub code: String,
    pub redirect_to: Option<String>,
    pub flow: ConfirmationFlow,
    pub expires_at: i64,
}

#[derive(Clone)]
pub struct AuthStore {
    pub users: Arc<dyn KvStore>,
    pub tokens: Arc<dyn KvStore>,
    pub confirmations: Arc<dyn KvStore>,
    pub identities: Arc<dyn KvStore>,
    pub social_state: Arc<dyn KvStore>,
    pub keys: Arc<dyn KvStore>,
}

impl AuthStore {
    pub fn new(
        users: Arc<dyn KvStore>,
        tokens: Arc<dyn KvStore>,
        confirmations: Arc<dyn KvStore>,
        identities: Arc<dyn KvStore>,
        social_state: Arc<dyn KvStore>,
        keys: Arc<dyn KvStore>,
    ) -> Self {
        Self {
            users,
            tokens,
            confirmations,
            identities,
            social_state,
            keys,
        }
    }
}

pub struct AuthState {
    pub config: super::config::AuthConfig,
    pub store: AuthStore,
    pub access_key: [u8; 64],
    pub refresh_key: [u8; 64],
    pub email_queue: Option<Arc<dyn jobs::JobEnqueuer>>,
}

impl AuthState {
    pub fn new(
        config: super::config::AuthConfig,
        store: AuthStore,
        email_queue: Option<Arc<dyn jobs::JobEnqueuer>>,
    ) -> anyhow::Result<Self> {
        let access_key = if let Ok(Some(key_bytes)) = store.keys.kv_get(b"access_key") {
            key_bytes
                .as_slice()
                .try_into()
                .unwrap_or_else(|_| Self::generate_and_save_key(&*store.keys, "access_key"))
        } else {
            Self::generate_and_save_key(&*store.keys, "access_key")
        };

        let refresh_key = if let Ok(Some(key_bytes)) = store.keys.kv_get(b"refresh_key") {
            key_bytes
                .as_slice()
                .try_into()
                .unwrap_or_else(|_| Self::generate_and_save_key(&*store.keys, "refresh_key"))
        } else {
            Self::generate_and_save_key(&*store.keys, "refresh_key")
        };

        Ok(Self {
            config,
            store,
            access_key,
            refresh_key,
            email_queue,
        })
    }

    fn generate_and_save_key(kv: &dyn KvStore, key_name: &str) -> [u8; 64] {
        let signing_key = SigningKey::generate(&mut OsRng);
        let key_bytes = signing_key.to_keypair_bytes();
        let _ = kv.kv_insert(key_name.as_bytes(), &key_bytes);
        key_bytes
    }
}

pub async fn start_pruning_task(auth_state: Arc<AuthState>) {
    tracing::info!("Auth State Pruning Task Started");

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;

        let now = chrono::Utc::now().timestamp();
        tracing::debug!("Running pruning job for Auth Store");

        // 1. Prune Tokens
        let mut tokens_to_delete = Vec::new();
        let all_tokens = auth_state.store.tokens.kv_prefix(b"");
        for (key, val) in all_tokens {
            if let Ok(record) = serde_json::from_slice::<TokenRecord>(&val) {
                if record.expires_at < now {
                    tokens_to_delete.push(key);
                }
            }
        }
        for key in tokens_to_delete {
            let _ = auth_state.store.tokens.kv_remove(&key);
        }

        // 2. Prune Confirmations
        let mut confirmations_to_delete = Vec::new();
        let all_confs = auth_state.store.confirmations.kv_prefix(b"");
        for (key, val) in all_confs {
            if let Ok(record) = serde_json::from_slice::<ConfirmationRecord>(&val) {
                if record.expires_at < now {
                    confirmations_to_delete.push(key);
                }
            }
        }
        for key in confirmations_to_delete {
            let _ = auth_state.store.confirmations.kv_remove(&key);
        }
    }
}
