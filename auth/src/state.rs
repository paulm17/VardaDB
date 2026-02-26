use fjall::{Database, Keyspace, KeyspaceCreateOptions};
use serde::{Deserialize, Serialize};
use rand::rngs::OsRng;
use ed25519_dalek::SigningKey;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserRecord {
    pub id: String,           // ulid
    pub name: String,
    pub email: String,
    pub password_hash: Option<String>,
    pub verified: bool,
    pub created_at: i64,      // unix timestamp
    pub updated_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TokenRecord {
    pub id: String,           // ulid
    pub user_id: String,
    pub token_uuid: String,
    pub expires_at: i64,      // unix timestamp — used for TTL
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ConfirmationFlow { Created, Seen, Completed }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfirmationRecord {
    pub id: String,           // ulid
    pub user_id: String,
    pub code: String,
    pub redirect_to: Option<String>,
    pub flow: ConfirmationFlow,
    pub expires_at: i64,
}

#[derive(Clone)]
pub struct AuthStore {
    pub users: Keyspace,
    pub tokens: Keyspace,
    pub confirmations: Keyspace,
    pub identities: Keyspace,
    pub social_state: Keyspace,
    pub keys: Keyspace,
}

impl AuthStore {
    pub fn init(db: &Database) -> anyhow::Result<Self> {
        let users = db.keyspace("auth_users", || KeyspaceCreateOptions::default())?;
        let tokens = db.keyspace("auth_tokens", || KeyspaceCreateOptions::default())?;
        let confirmations = db.keyspace("auth_confirmations", || KeyspaceCreateOptions::default())?;
        let identities = db.keyspace("auth_identities", || KeyspaceCreateOptions::default())?;
        let social_state = db.keyspace("auth_social_state", || KeyspaceCreateOptions::default())?;
        let keys = db.keyspace("auth_keys", || KeyspaceCreateOptions::default())?;

        Ok(Self {
            users,
            tokens,
            confirmations,
            identities,
            social_state,
            keys,
        })
    }
}

pub struct AuthState {
    pub config: super::config::AuthConfig,
    pub store: AuthStore,
    pub access_key: [u8; 64],   // Ed25519 private key
    pub refresh_key: [u8; 64],  // Ed25519 private key for refresh tokens
    pub email_queue: Option<std::sync::Arc<jobs::Queue>>,
}

impl AuthState {
    pub fn new(config: super::config::AuthConfig, db: &Database, email_queue: Option<std::sync::Arc<jobs::Queue>>) -> anyhow::Result<Self> {
        let store = AuthStore::init(db)?;

        // Load or generate Access Key
        let access_key = if let Some(key_bytes) = store.keys.get("access_key")? {
            key_bytes.as_ref().try_into().unwrap_or_else(|_| Self::generate_and_save_key(&store.keys, "access_key"))
        } else {
            Self::generate_and_save_key(&store.keys, "access_key")
        };

        // Load or generate Refresh Key
        let refresh_key = if let Some(key_bytes) = store.keys.get("refresh_key")? {
            key_bytes.as_ref().try_into().unwrap_or_else(|_| Self::generate_and_save_key(&store.keys, "refresh_key"))
        } else {
            Self::generate_and_save_key(&store.keys, "refresh_key")
        };

        Ok(Self {
            config,
            store,
            access_key,
            refresh_key,
            email_queue,
        })
    }

    fn generate_and_save_key(keyspace: &Keyspace, key_name: &str) -> [u8; 64] {
        let signing_key = SigningKey::generate(&mut OsRng);
        let key_bytes = signing_key.to_keypair_bytes();
        let _ = keyspace.insert(key_name, &key_bytes);
        key_bytes
    }
}

pub async fn start_pruning_task(auth_state: std::sync::Arc<AuthState>) {
    tracing::info!("Auth State Pruning Task Started");
    
    loop {
        // Run every 60 minutes
        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
        
        let now = chrono::Utc::now().timestamp();
        tracing::debug!("Running pruning job for Auth Store");

        // 1. Prune Tokens
        let mut tokens_to_delete = Vec::new();
        let iter = auth_state.store.tokens.iter();
        for item in iter {
            if let Ok(kvp) = item.into_inner() {
                if let Ok(record) = serde_json::from_slice::<TokenRecord>(&kvp.1) {
                    if record.expires_at < now {
                        tokens_to_delete.push(kvp.0.to_vec());
                    }
                }
            }
        }
        
        for key in tokens_to_delete {
            let _ = auth_state.store.tokens.remove(&*key);
        }

        // 2. Prune Confirmations
        let mut confirmations_to_delete = Vec::new();
        let iter = auth_state.store.confirmations.iter();
        for item in iter {
            if let Ok(kvp) = item.into_inner() {
                if let Ok(record) = serde_json::from_slice::<ConfirmationRecord>(&kvp.1) {
                    if record.expires_at < now {
                        confirmations_to_delete.push(kvp.0.to_vec());
                    }
                }
            }
        }
        
        for key in confirmations_to_delete {
            let _ = auth_state.store.confirmations.remove(&*key);
        }
    }
}
