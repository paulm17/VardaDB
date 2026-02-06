use clap::{Parser, Subcommand};
use vardadb::{run, build_schema, codegen};
use std::fs;

#[derive(Parser)]
#[command(name = "vardadb")]
#[command(about = "VardaDB Engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    
    /// Path to config file (default: config.toml)
    #[arg(short, long, default_value = "config.toml")]
    config: String,
    
    /// Run as MCP Server (stdio)
    #[arg(long, default_value = "false")]
    mcp: bool,
    
    /// Override server port (e.g., 8000)
    #[arg(short, long)]
    port: Option<u16>,
    
    /// Override data directory path
    #[arg(short = 'd', long)]
    data_dir: Option<String>,
    
    /// Override schema SDL file path
    #[arg(short, long)]
    schema: Option<String>,
    
    /// Override node ID for multi-node deployments
    #[arg(long)]
    node_id: Option<u64>,
}

#[derive(Subcommand)]
enum Commands {
    /// Starts the VardaDB Server
    Start,
    /// Exports the GraphQL Schema SDL
    ExportSchema {
        /// Path to the Input SDL file (VardaDB Schema)
        #[arg(short, long)]
        schema: Option<String>, 
        /// Output path (Optional, prints to stdout if missing)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Generates client code (TypeScript) from Schema
    Generate {
        /// Path to the Input SDL file (VardaDB Schema)
        #[arg(short, long)]
        schema: Option<String>,
        /// Output path (e.g. schema.ts)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Database Management
    #[command(subcommand)]
    Db(vardadb::cli::DbCommands),
    /// Interactive Shell (REPL)
    Cli,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    
    // Load Config
    let config = match vardadb::config::VardaConfig::load_from_file(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config from {}: {}", cli.config, e);
            std::process::exit(1);
        }
    };
    
    // Override with CLI flags
    let mut config = config;
    if cli.mcp {
        config.server.is_mcp = true;
    }
    if let Some(port) = cli.port {
        config.server.port = port;
    }
    if let Some(data_dir) = cli.data_dir {
        config.server.storage_path = data_dir;
    }
    if let Some(schema) = cli.schema {
        config.server.schema_path = Some(schema);
    }
    if let Some(node_id) = cli.node_id {
        config.server.node_id = Some(node_id);
    }

    match cli.command {
        Some(Commands::Start) | None => {
            run(config).await;
        }
        Some(Commands::ExportSchema { schema, output }) => {
            // Use CLI schema path if provided, else config
            let schema_path = schema.or(config.server.schema_path).expect("Schema path must be provided in config or CLI");
            let content = fs::read_to_string(&schema_path).expect("Failed to read schema file");
            match build_schema(&content) {
                Ok(s) => {
                    let sdl = s.sdl();
                    if let Some(path) = output {
                        fs::write(path, sdl).expect("Failed to write output file");
                    } else {
                        println!("{}", sdl);
                    }
                }
                Err(e) => {
                    eprintln!("Error building schema: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Generate { schema, output }) => {
             let schema_path = schema.or(config.server.schema_path).expect("Schema path must be provided in config or CLI");
            let content = fs::read_to_string(&schema_path).expect("Failed to read schema file");
            match build_schema(&content) {
                Ok(s) => {
                    let sdl = s.sdl();
                    // Generate TypeScript
                    match codegen::generate_typescript(&sdl) {
                        Ok(ts) => {
                             if let Some(path) = output {
                                fs::write(path, ts).expect("Failed to write output file");
                            } else {
                                println!("{}", ts);
                            }
                        }
                        Err(e) => {
                            eprintln!("Error generating typescript: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error building schema: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Db(cmd)) => {
            if let Err(e) = vardadb::cli::handle_db_command(&cmd, &config).await {
                eprintln!("Command failed: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::Cli) => {
            if let Err(e) = vardadb::repl::run_repl(&config).await {
                eprintln!("REPL Error: {}", e);
            }
        }
    }
}

