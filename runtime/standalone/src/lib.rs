mod api;
mod build_info;
mod config;
mod metadata;
mod signal;
mod standalone;
mod worker;

use std::path::PathBuf;
use std::time::Duration;

use rustls::crypto::aws_lc_rs;
use tracing::{error, info, trace, warn};

use restate_clock::ClockUpkeep;
use restate_core::TaskCenterBuilder;
use restate_tracing_instrumentation::TracingGuard;
use restate_tracing_instrumentation::init_tracing_and_logging;

#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

use crate::config::StandaloneConfig;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(target_os = "linux")]
#[unsafe(export_name = "malloc_conf")]
pub static MALLOC_CONF: &[u8] = b"prof:true,prof_active:false,lg_prof_sample:19\0";

#[derive(Debug, Clone, Default)]
pub struct StandaloneRunOptions {
    pub config_file: Option<PathBuf>,
    pub dump_config: bool,
}

const EXIT_CODE_FAILURE: i32 = 1;

pub fn run(options: StandaloneRunOptions) -> anyhow::Result<()> {
    let Ok(_clock) = ClockUpkeep::start() else {
        anyhow::bail!("failed to start restate internal clock thread");
    };

    aws_lc_rs::default_provider()
        .install_default()
        .expect("no other default crypto provider being installed");

    let config_path = options
        .config_file
        .as_ref()
        .map(|p| std::fs::canonicalize(p).expect("config-file path is valid"));

    let standalone_config = StandaloneConfig::load(config_path.as_deref())?;

    if options.dump_config {
        println!(
            "{}",
            standalone_config
                .dump()
                .expect("standalone config is toml serializable")
        );
        return Ok(());
    }

    let loaded = standalone_config.into_loaded()?;

    if rlimit::increase_nofile_limit(u64::MAX).is_err() {
        warn!("Failed to increase the number of open file descriptors limit.");
    }

    if !loaded.base_dir.exists()
        && let Err(err) = std::fs::create_dir_all(&loaded.base_dir)
    {
        anyhow::bail!(
            "failed to create data directory at {}: {err}",
            loaded.base_dir.display()
        );
    }

    let task_center = TaskCenterBuilder::default()
        .options(loaded.common.clone())
        .build()
        .expect("task_center builds");

    let res = task_center.block_on(async move {
        let tracing_guard = init_tracing_and_logging(&loaded.common, "restate-standalone")
            .map_err(|err| anyhow::anyhow!("failed to configure logging and tracing: {err}"))?;

        install_panic_hook();

        info!(
            node_name = loaded.node_name,
            base_dir = %loaded.base_dir.display(),
            "Starting Restate Standalone {}",
            build_info::build_info()
        );

        let runtime_config = standalone::StandaloneRuntimeConfig::from_loaded_config(&loaded);
        let shutdown_grace_period = runtime_config.shutdown_grace_period;
        standalone::run_standalone(runtime_config).await?;
        shutdown_tracing(shutdown_grace_period / 2, tracing_guard).await;

        Ok::<(), anyhow::Error>(())
    });

    let exit_code = task_center.exit_code();
    if let Err(err) = res {
        return Err(anyhow::anyhow!("Restate standalone failed: {err:?}"));
    }
    if exit_code != 0 {
        error!(
            "Restate standalone terminated with exit code {}!",
            exit_code
        );
        std::process::exit(exit_code);
    }

    Ok(())
}

pub fn run_and_exit(options: StandaloneRunOptions) -> ! {
    if let Err(err) = run(options) {
        eprintln!("{err}");
        std::process::exit(EXIT_CODE_FAILURE);
    }

    std::process::exit(0);
}

fn install_panic_hook() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let thread = std::thread::current();
        let thread_name = thread.name();
        eprintln!("\n[{thread_name:?}]  PANIC!!!\n{panic_info}\n");
        tracing_panic::panic_hook(panic_info);
        prev_hook(panic_info);
    }));
}

async fn shutdown_tracing(grace_period: Duration, tracing_guard: TracingGuard) {
    trace!("Shutting down tracing to flush pending spans");

    let shutdown_tracing_with_timeout =
        tokio::time::timeout(grace_period, tracing_guard.async_shutdown());
    let shutdown_result = shutdown_tracing_with_timeout.await;

    if shutdown_result.is_err() {
        trace!("Failed to fully flush pending spans, terminating now.");
    }
}
