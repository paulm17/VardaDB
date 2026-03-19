use clap::{CommandFactory, FromArgMatches, Parser};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "vardadb-runtime", about = "Embedded Restate runtime")]
struct RuntimeEntry {
    #[arg(long = "vardadb-config", value_name = "FILE", hide = true)]
    vardadb_config: Option<PathBuf>,

    #[command(flatten)]
    cli: runtime::RuntimeCli,
}

#[tokio::main]
async fn main() {
    let mut command = RuntimeEntry::command();
    command = command.bin_name("vardadb runtime");
    let matches = command.get_matches();
    let entry = RuntimeEntry::from_arg_matches(&matches).expect("validated by clap");
    if let Err(err) = runtime::run(entry.cli, entry.vardadb_config.as_deref()).await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
