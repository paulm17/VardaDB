use crate::config::VardaConfig;
use reqwest::Client;
use serde::Deserialize;
use comfy_table::Table;

#[derive(Deserialize)]
struct DbResponse {
    name: String,
    #[allow(dead_code)]
    status: String,
}

#[derive(Deserialize)]
struct ListDbsResponse {
    databases: Vec<String>,
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
            let res = client.post(format!("{}/db", base_url))
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
        },
        DbCommands::List => {
            let res = client.get(format!("{}/db", base_url))
                .send()
                .await?;
            
            if res.status().is_success() {
                let list: ListDbsResponse = res.json().await?;
                let mut table = Table::new();
                table.set_header(vec!["Database Name"]);
                for db in list.databases {
                    table.add_row(vec![db]);
                }
                println!("{}", table);
            } else {
                let err = res.text().await?;
                eprintln!("Failed to list databases: {}", err);
            }
        },
        DbCommands::Delete { name } => {
            let res = client.delete(format!("{}/db/{}", base_url, name))
                .send()
                .await?;
            
            if res.status().is_success() {
                println!("Database '{}' deleted successfully.", name);
            } else {
                let err = res.text().await?;
                eprintln!("Failed to delete database: {}", err);
            }
        },
        DbCommands::Apply { name, schema } => {
            let schema_content = std::fs::read_to_string(schema)
                .map_err(|e| anyhow::anyhow!("Failed to read schema file: {}", e))?;

            let res = client.post(format!("{}/db/{}/schema", base_url, name))
                .body(schema_content)
                .send()
                .await?;
            
            if res.status().is_success() {
                println!("Schema applied to database '{}' successfully.", name);
            } else {
                let err = res.text().await?;
                eprintln!("Failed to apply schema: {}", err);
            }
        },
    }
    Ok(())
}
