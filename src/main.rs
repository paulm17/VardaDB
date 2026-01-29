use clap::{Parser, Subcommand};
use vardadb::{run, build_schema, codegen};
use std::fs;

#[derive(Parser)]
#[command(name = "vardadb")]
#[command(about = "VardaDB Engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Starts the VardaDB Server
    Start {
        /// Port to bind to (default: 8000)
        #[arg(short, long, default_value_t = 8000)]
        port: u16,
    },
    /// Exports the GraphQL Schema SDL
    ExportSchema {
        /// Path to the Input SDL file (VardaDB Schema)
        #[arg(short, long)]
        schema: String,
        /// Output path (Optional, prints to stdout if missing)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Generates client code (TypeScript) from Schema
    Generate {
        /// Path to the Input SDL file (VardaDB Schema)
        #[arg(short, long)]
        schema: String,
        /// Output path (e.g. schema.ts)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Start { port }) => {
            run(port).await;
        }
        None => {
            run(8000).await;
        }
        Some(Commands::ExportSchema { schema, output }) => {
            let content = fs::read_to_string(&schema).expect("Failed to read schema file");
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
            let content = fs::read_to_string(&schema).expect("Failed to read schema file");
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
