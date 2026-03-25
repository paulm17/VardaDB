use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct DbStatus {
    pub name: String,
    pub status: String,
    // Add more metrics here later (size, item count, etc.)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DbInfo {
    pub name: String,
    pub path: String,
}

#[async_trait]
pub trait DatabaseManager: Send + Sync + 'static {
    async fn create_db(&self, name: &str) -> Result<(), String>;
    async fn list_dbs(&self) -> Result<Vec<DbInfo>, String>;
    async fn delete_db(&self, name: &str) -> Result<(), String>;
    async fn apply_schema(&self, db_name: &str, sdl: &str) -> Result<(), String>;
    async fn get_schema(&self, db_name: &str) -> Result<String, String>;
    async fn get_db_status(&self, name: &str) -> Result<DbStatus, String>;
    async fn update_db_path(&self, name: &str, new_path: &str) -> Result<(), String>;
}
