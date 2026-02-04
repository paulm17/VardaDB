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
    }
}

