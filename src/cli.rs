use crate::bridge::sqlite_resolver::SqliteResolver;
use crate::config::VardaConfig;
use crate::realtime::bus::EventBus;
use crate::storage::backend::Storage;
use comfy_table::Table;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
struct DbResponse {
    name: String,
    #[allow(dead_code)]
    status: String,
}

#[derive(Deserialize)]
struct DbInfo {
    name: String,
    path: String,
}

#[derive(Deserialize)]
struct ListDbsResponse {
    databases: Vec<DbInfo>,
}

use clap::Subcommand;

#[derive(Subcommand, Clone)]
pub enum DbCommands {
    /// Create a new database
    Create {
        /// Name of the database
        name: String,
    },
    /// List all databases
    List,
    /// Delete a database
    Delete {
        /// Name of the database
        name: String,
    },
    /// Update the storage path for a database
    UpdatePath {
        /// Name of the database
        name: String,
        /// New absolute file path
        path: String,
    },
    /// Apply a schema to a database
    Apply {
        /// Name of the database
        #[arg(short, long)]
        name: String,
        /// Path to the schema SDL file
        #[arg(short, long)]
        schema: String,
    },
}

pub async fn handle_db_command(command: &DbCommands, config: &VardaConfig) -> anyhow::Result<()> {
    let client = Client::new();
    let base_url = format!("http://127.0.0.1:{}/_mgmt", config.server.port);

    match command {
        DbCommands::Create { name } => {
            let res = client
                .post(format!("{}/db", base_url))
                .json(&serde_json::json!({ "name": name }))
                .send()
                .await?;

            if res.status().is_success() {
                let db: DbResponse = res.json().await?;
                println!("Database '{}' created successfully.", db.name);
            } else {
                let err = res.text().await?;
                eprintln!("Failed to create database: {}", err);
            }
        }
        DbCommands::List => {
            let res = client.get(format!("{}/db", base_url)).send().await?;

            if res.status().is_success() {
                let list: ListDbsResponse = res.json().await?;
                let mut table = Table::new();
                table.set_header(vec!["Database Name", "Path"]);
                for db in list.databases {
                    table.add_row(vec![db.name, db.path]);
                }
                println!("{}", table);
            } else {
                let err = res.text().await?;
                eprintln!("Failed to list databases: {}", err);
            }
        }
        DbCommands::Delete { name } => {
            let res = client
                .delete(format!("{}/db/{}", base_url, name))
                .send()
                .await?;

            if res.status().is_success() {
                println!("Database '{}' deleted successfully.", name);
            } else {
                let err = res.text().await?;
                eprintln!("Failed to delete database: {}", err);
            }
        }
        DbCommands::UpdatePath { name, path } => {
            let res = client
                .post(format!("{}/db/{}/path", base_url, name))
                .json(&serde_json::json!({ "path": path }))
                .send()
                .await?;

            if res.status().is_success() {
                println!(
                    "Database '{}' path updated successfully to '{}'.",
                    name, path
                );
            } else {
                let err = res.text().await?;
                eprintln!("Failed to update database path: {}", err);
            }
        }
        DbCommands::Apply { name, schema } => {
            let schema_content = std::fs::read_to_string(schema)
                .map_err(|e| anyhow::anyhow!("Failed to read schema file: {}", e))?;

            let res = client
                .post(format!("{}/db/{}/schema", base_url, name))
                .body(schema_content)
                .send()
                .await?;

            if res.status().is_success() {
                println!("Schema applied to database '{}' successfully.", name);
            } else {
                let err = res.text().await?;
                eprintln!("Failed to apply schema: {}", err);
            }
        }
    }
    Ok(())
}

pub fn handle_db_command_direct(command: &DbCommands, config: &VardaConfig) -> anyhow::Result<()> {
    let storage = Arc::new(Storage::new(
        &config.server.storage_path,
        config.server.node_id,
    )?);

    match command {
        DbCommands::Create { name } => {
            storage.create_database(name)?;

            let event_bus = EventBus::new();
            let resolver = SqliteResolver::with_db(
                storage.clone(),
                event_bus,
                name.clone(),
            );
            match crate::engine::schema::Schema::load_with_resolver(
                crate::defaults::AGENT_SCHEMA,
                resolver,
            ) {
                Ok(_) => {
                    let schema_file_path = std::path::PathBuf::from(&config.server.storage_path)
                        .join(format!("{}_schema.graphql", name));
                    if let Err(e) =
                        std::fs::write(&schema_file_path, crate::defaults::AGENT_SCHEMA)
                    {
                        eprintln!("Warning: Failed to persist schema: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to load default schema: {}", e);
                }
            }
            println!("Database '{}' created successfully.", name);
        }
        DbCommands::List => {
            let dbs = storage.list_databases();
            let mut table = Table::new();
            table.set_header(vec!["Database Name", "Path"]);
            for (name, path) in dbs {
                table.add_row(vec![name, path]);
            }
            println!("{}", table);
        }
        DbCommands::Delete { name } => {
            storage.delete_database(name)?;
            println!("Database '{}' deleted successfully.", name);
        }
        DbCommands::UpdatePath { name, path } => {
            storage.update_db_path(name, path)?;
            println!(
                "Database '{}' path updated successfully to '{}'.",
                name, path
            );
        }
        DbCommands::Apply { name, schema } => {
            if storage.get_database(name).is_none() {
                anyhow::bail!("Database '{}' not found", name);
            }
            let schema_content = std::fs::read_to_string(schema)
                .map_err(|e| anyhow::anyhow!("Failed to read schema file: {}", e))?;

            let event_bus = EventBus::new();
            let resolver = SqliteResolver::with_db(
                storage.clone(),
                event_bus,
                name.clone(),
            );
            match crate::engine::schema::Schema::load_with_resolver(&schema_content, resolver) {
                Ok(_) => {
                    let schema_file_path = std::path::PathBuf::from(&config.server.storage_path)
                        .join(format!("{}_schema.graphql", name));
                    std::fs::write(&schema_file_path, &schema_content)?;
                    println!("Schema applied to database '{}' successfully.", name);
                }
                Err(e) => {
                    anyhow::bail!("Invalid Schema: {}", e);
                }
            }
        }
    }

    Ok(())
}
