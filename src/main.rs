use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use vardadb::{build_schema, codegen, run};

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
    /// Embedded Restate runtime
    Runtime {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[tokio::main]
async fn main() {
    if let Err(err) = maybe_delegate_runtime() {
        eprintln!("Runtime command failed: {}", err);
        std::process::exit(1);
    }

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
            let schema_path = schema
                .or(config.server.schema_path)
                .expect("Schema path must be provided in config or CLI");
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
            let schema_path = schema
                .or(config.server.schema_path)
                .expect("Schema path must be provided in config or CLI");
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
        Some(Commands::Runtime { .. }) => unreachable!("runtime handled before parsing CLI"),
    }
}

fn maybe_delegate_runtime() -> anyhow::Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();
    let Some(runtime_index) = raw_args.iter().position(|arg| arg == "runtime") else {
        return Ok(());
    };

    let forwarded_args = raw_args[runtime_index + 1..].to_vec();

    let runtime_bin = locate_runtime_binary()?;
    let mut command = std::process::Command::new(runtime_bin);
    if let Some(config_path) = runtime_config_handoff_path(&raw_args) {
        command.arg("--vardadb-config").arg(config_path);
    }

    let status = command
        .args(forwarded_args)
        .status()
        .map_err(|err| anyhow::anyhow!("failed to start embedded runtime: {err}"))?;

    std::process::exit(status.code().unwrap_or(1));
}

fn extract_config_path(args: &[String]) -> Option<(String, bool)> {
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--config=") {
            return Some((value.to_string(), true));
        }
        if arg == "--config" || arg == "-c" {
            if let Some(value) = iter.peek() {
                return Some(((**value).to_string(), true));
            }
        }
    }
    None
}

fn runtime_config_handoff_path(args: &[String]) -> Option<String> {
    match extract_config_path(args) {
        Some((path, _explicit)) => Some(path),
        None => {
            let default = Path::new("config.toml");
            default.exists().then(|| default.display().to_string())
        }
    }
}

fn locate_runtime_binary() -> anyhow::Result<PathBuf> {
    let current = std::env::current_exe()
        .map_err(|err| anyhow::anyhow!("failed to determine current executable path: {err}"))?;
    let sibling = current
        .parent()
        .map(|dir| dir.join("vardadb-runtime"))
        .ok_or_else(|| anyhow::anyhow!("failed to determine executable directory"))?;

    if sibling.exists() {
        return Ok(sibling);
    }

    for candidate in [
        Path::new("runtime")
            .join("target")
            .join("debug")
            .join("vardadb-runtime"),
        Path::new("runtime")
            .join("target")
            .join("release")
            .join("vardadb-runtime"),
        Path::new("target").join("debug").join("vardadb-runtime"),
        Path::new("target").join("release").join("vardadb-runtime"),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    anyhow::bail!(
        "embedded runtime binary not found; build it from the nested workspace with `cargo +1.93.0 build --manifest-path runtime/Cargo.toml --bin vardadb-runtime`"
    );
}
