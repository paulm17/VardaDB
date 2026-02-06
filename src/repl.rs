use rustyline::error::ReadlineError;
use rustyline::{DefaultEditor, Result};
use reqwest::Client;
use crate::config::VardaConfig;
use comfy_table::Table;
use serde_json::Value;

pub async fn run_repl(config: &VardaConfig) -> Result<()> {
    let mut rl = DefaultEditor::new()?;
    
    // Load history if exists
    if rl.load_history("history.txt").is_err() {
        // No history found
    }
    
    let client = Client::new();
    let port = config.server.port;
    let base_url = format!("http://127.0.0.1:{}", port);
    
    let mut current_db = "default".to_string();
    
    println!("VardaDB Interactive Shell");
    println!("Type 'help' for commands.");
    println!("Connected to http://127.0.0.1:{}/graphql", port);

    loop {
        let readline = rl.readline(&format!("vardadb({})> ", current_db));
        match readline {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() { continue; }
                
                // Add full line to history
                rl.add_history_entry(line)?;

                // Normalize for execution: remove trailing semicolon
                let mut exec_line = line;
                if exec_line.ends_with(';') {
                    exec_line = &exec_line[..exec_line.len() - 1].trim();
                }
                
                if exec_line.eq_ignore_ascii_case("exit") || exec_line.eq_ignore_ascii_case("quit") {
                    break;
                } else if exec_line.eq_ignore_ascii_case("help") {
                    println!("Commands:");
                    println!("  use <dbname>      Switch active database");
                    println!("  show databases    List all databases");
                    println!("  exit / quit       Exit the shell");
                    println!("  create database <name> Create a new database");
                    println!("  drop database <name>   Delete a database");
                    println!("  <query>           Execute GraphQL query");
                } else if exec_line.starts_with("use ") {
                    let parts: Vec<&str> = exec_line.split_whitespace().collect();
                    if parts.len() == 2 {
                        let new_db = parts[1].to_string();
                        // Verify DB exists? optional, but polite.
                        // We can call list_dbs to check.
                        
                        match list_databases(&client, &base_url).await {
                             Ok(dbs) => {
                                 if dbs.contains(&new_db) {
                                     current_db = new_db;
                                     println!("Switched to database '{}'", current_db);
                                 } else {
                                     println!("Database '{}' does not exist.", new_db);
                                 }
                             },
                             Err(e) => println!("Error checking databases: {}", e),
                        }
                    } else {
                        println!("Usage: use <dbname>");
                    }
                } else if exec_line.eq_ignore_ascii_case("show databases") {
                     match list_databases(&client, &base_url).await {
                         Ok(dbs) => {
                             let mut table = Table::new();
                             table.set_header(vec!["Database Name"]);
                             for db in dbs {
                                 table.add_row(vec![db]);
                             }
                             println!("{}", table);
                         },
                         Err(e) => println!("Error: {}", e),
                     }
                } else if exec_line.to_lowercase().starts_with("create database ") {
                    let parts: Vec<&str> = exec_line.split_whitespace().collect();
                    if parts.len() == 3 {
                        let db_name = parts[2];
                        if let Err(e) = create_database(&client, &base_url, db_name).await {
                            println!("Error: {}", e);
                        }
                    } else {
                         println!("Usage: create database <dbname>");
                    }
                } else if exec_line.to_lowercase().starts_with("drop database ") {
                    let parts: Vec<&str> = exec_line.split_whitespace().collect();
                    if parts.len() == 3 {
                        let db_name = parts[2];
                         if let Err(e) = drop_database(&client, &base_url, db_name).await {
                            println!("Error: {}", e);
                        }
                    } else {
                         println!("Usage: drop database <dbname>");
                    }
                } else {
                    // Strict Query Check
                    let lower = exec_line.to_lowercase();
                    if lower.starts_with("query") || lower.starts_with("mutation") || lower.starts_with("subscription") || exec_line.starts_with("{") {
                        match execute_query(&client, &base_url, &current_db, exec_line).await {
                            Ok(json) => {
                                 // Pretty print output
                                 if let Ok(pretty) = serde_json::to_string_pretty(&json) {
                                     println!("{}", pretty);
                                 } else {
                                     println!("{:?}", json);
                                 }
                            },
                            Err(e) => println!("Error: {}", e),
                        }
                    } else {
                        println!("Unknown command: '{}'. Type 'help' for available commands.", exec_line);
                    }
                }
            },
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break
            },
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break
            },
            Err(err) => {
                println!("Error: {:?}", err);
                break
            }
        }
    }
    rl.save_history("history.txt")?;
    Ok(())
}

async fn list_databases(client: &Client, base_url: &str) -> anyhow::Result<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct ListResp { databases: Vec<String> }
    
    let res = client.get(format!("{}/_mgmt/db", base_url)).send().await?;
    if res.status().is_success() {
        let resp: ListResp = res.json().await?;
        Ok(resp.databases)
    } else {
        Err(anyhow::anyhow!("Failed to list databases: {}", res.status()))
    }
}

async fn execute_query(client: &Client, base_url: &str, db: &str, query: &str) -> anyhow::Result<Value> {
    let res = client.post(format!("{}/graphql", base_url))
        .header("x-varda-db", db)
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await?;
        
    if res.status().is_success() {
        let json: Value = res.json().await?;
        Ok(json)
    } else {
        Err(anyhow::anyhow!("Query failed: {}", res.text().await?))
    }
}
async fn create_database(client: &Client, base_url: &str, name: &str) -> anyhow::Result<()> {
    let res = client.post(format!("{}/_mgmt/db", base_url))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await?;
    if res.status().is_success() {
        println!("Database '{}' created.", name);
        Ok(())
    } else {
        Err(anyhow::anyhow!("Failed to create database: {}", res.text().await?))
    }
}

async fn drop_database(client: &Client, base_url: &str, name: &str) -> anyhow::Result<()> {
     let res = client.delete(format!("{}/_mgmt/db/{}", base_url, name))
        .send()
        .await?;
    if res.status().is_success() {
        println!("Database '{}' deleted.", name);
        Ok(())
    } else {
        Err(anyhow::anyhow!("Failed to delete database: {}", res.text().await?))
    }
}
