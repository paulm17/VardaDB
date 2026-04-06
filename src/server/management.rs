use crate::bridge::redb_resolver::RedbResolver;
use crate::realtime::bus::EventBus;
use crate::storage::backend::Storage;
use async_trait::async_trait;
use management::{DatabaseManager, DbStatus};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ManagementState {
    pub storage: Arc<Storage>,
    pub schemas: Arc<dashmap::DashMap<String, Arc<RwLock<Arc<crate::engine::schema::Schema>>>>>,
    pub event_bus: EventBus,
    pub storage_path: std::path::PathBuf,
}

#[async_trait]
impl DatabaseManager for ManagementState {
    async fn create_db(&self, name: &str) -> Result<(), String> {
        match self.storage.create_database(name) {
            Ok(_) => {
                // Inject Default Agent Schema
                let schema_body = crate::defaults::AGENT_SCHEMA;
                println!("Injecting Agent Schema into database: {}", name);

                let resolver = RedbResolver::with_db(
                    self.storage.clone(),
                    self.event_bus.clone(),
                    name.to_string(),
                );
                match crate::engine::schema::Schema::load_with_resolver(schema_body, resolver) {
                    Ok(new_schema) => {
                        let arc_schema = Arc::new(RwLock::new(Arc::new(new_schema)));
                        self.schemas.insert(name.to_string(), arc_schema);

                        // Persist
                        let schema_file_path =
                            self.storage_path.join(format!("{}_schema.graphql", name));
                        if let Err(e) = tokio::fs::write(&schema_file_path, schema_body).await {
                            eprintln!(
                                "Failed to persist schema for {} to {:?}: {}",
                                name, schema_file_path, e
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to load Agent Schema for {}: {}", name, e);
                    }
                }
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    async fn list_dbs(&self) -> Result<Vec<management::DbInfo>, String> {
        let dbs = self.storage.list_databases();
        Ok(dbs
            .into_iter()
            .map(|(name, path)| management::DbInfo { name, path })
            .collect())
    }

    async fn delete_db(&self, name: &str) -> Result<(), String> {
        if name == "default" {
            return Err("Cannot delete default database".to_string());
        }

        match self.storage.delete_database(name) {
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn get_db_status(&self, name: &str) -> Result<DbStatus, String> {
        // Check if DB exists
        if self.storage.get_database(name).is_none() {
            return Err(format!("Database '{}' not found", name));
        }

        // Simple status for now
        Ok(DbStatus {
            name: name.to_string(),
            status: "active".to_string(),
        })
    }

    async fn get_schema(&self, db_name: &str) -> Result<String, String> {
        // Check if DB exists
        if self.storage.get_database(db_name).is_none() {
            return Err(format!("Database '{}' not found", db_name));
        }

        let schema_file_path = self
            .storage_path
            .join(format!("{}_schema.graphql", db_name));
        match tokio::fs::read_to_string(&schema_file_path).await {
            Ok(sdl) => Ok(sdl),
            Err(_) => {
                // Fallback to default if not found but DB exists?
                // Or maybe check in-memory schemas?
                // For now, simpler to return empty or specific error.
                Ok("".to_string())
            }
        }
    }

    async fn apply_schema(&self, db_name: &str, sdl: &str) -> Result<(), String> {
        println!("Applying schema to database: {}", db_name);

        if self.storage.get_database(db_name).is_none() {
            return Err(format!("Database '{}' not found", db_name));
        }

        let resolver = RedbResolver::with_db(
            self.storage.clone(),
            self.event_bus.clone(),
            db_name.to_string(),
        );

        match crate::engine::schema::Schema::load_with_resolver(sdl, resolver) {
            Ok(new_schema) => {
                let arc_schema = Arc::new(RwLock::new(Arc::new(new_schema)));
                self.schemas.insert(db_name.to_string(), arc_schema);

                let schema_file_path = self
                    .storage_path
                    .join(format!("{}_schema.graphql", db_name));

                if let Err(e) = tokio::fs::write(&schema_file_path, sdl).await {
                    eprintln!(
                        "Failed to persist schema for {} to {:?}: {}",
                        db_name, schema_file_path, e
                    );
                } else {
                    println!("Schema persisted to {:?}", schema_file_path);
                }

                Ok(())
            }
            Err(e) => Err(format!("Invalid Schema: {}", e)),
        }
    }

    async fn update_db_path(&self, name: &str, new_path: &str) -> Result<(), String> {
        match self.storage.update_db_path(name, new_path) {
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
