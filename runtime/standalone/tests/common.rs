use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::process::{Child, Command};

#[allow(dead_code)]
enum BaseDirGuard {
    Temp(TempDir),
    Persistent(PathBuf),
}

#[allow(dead_code)]
pub struct StandaloneProcess {
    pub admin_url: String,
    pub ingress_url: String,
    child: Child,
    base_dir: BaseDirGuard,
}

pub fn supports_tcp_loopback() -> bool {
    TcpListener::bind(("127.0.0.1", 0)).is_ok()
}

impl StandaloneProcess {
    #[allow(dead_code)]
    pub async fn spawn() -> Self {
        Self::spawn_with_extra_config("").await
    }

    pub async fn spawn_with_extra_config(extra_config: &str) -> Self {
        let temp_dir = TempDir::new().expect("temp dir");
        let base_dir = temp_dir.path().to_path_buf();
        let config_path = base_dir.join("standalone.toml");
        Self::spawn_with_config(
            base_dir,
            config_path,
            extra_config,
            BaseDirGuard::Temp(temp_dir),
        )
        .await
    }

    #[allow(dead_code)]
    pub async fn spawn_in_base_dir(base_dir: PathBuf, extra_config: &str) -> Self {
        std::fs::create_dir_all(&base_dir).expect("create base dir");
        let config_path = base_dir.join("standalone.toml");
        Self::spawn_with_config(
            base_dir.clone(),
            config_path,
            extra_config,
            BaseDirGuard::Persistent(base_dir),
        )
        .await
    }

    async fn spawn_with_config(
        base_dir: PathBuf,
        config_path: PathBuf,
        extra_config: &str,
        base_dir_guard: BaseDirGuard,
    ) -> Self {
        let admin_port = allocate_port();
        let ingress_port = allocate_port();
        write_config(
            &base_dir,
            &config_path,
            admin_port,
            ingress_port,
            extra_config,
        );

        let child = Command::new(env!("CARGO_BIN_EXE_restate-standalone"))
            .arg("--config-file")
            .arg(&config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn standalone");
        let admin_url = format!("http://127.0.0.1:{admin_port}");
        let ingress_url = format!("http://127.0.0.1:{ingress_port}");

        wait_for_ready(&format!("{admin_url}/health")).await;
        wait_for_ready(&format!("{ingress_url}/")).await;

        Self {
            admin_url,
            ingress_url,
            child,
            base_dir: base_dir_guard,
        }
    }

    #[allow(dead_code)]
    pub fn base_dir(&self) -> &Path {
        match &self.base_dir {
            BaseDirGuard::Temp(temp_dir) => temp_dir.path(),
            BaseDirGuard::Persistent(path) => path.as_path(),
        }
    }

    pub async fn shutdown(mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id().expect("child pid") as i32, libc::SIGTERM);
        }

        let status = tokio::time::timeout(Duration::from_secs(15), self.child.wait())
            .await
            .expect("shutdown timeout")
            .expect("wait on child");
        assert!(
            status.success(),
            "standalone exited unsuccessfully: {status}"
        );
    }
}

fn write_config(
    base_dir: &Path,
    config_path: &Path,
    admin_port: u16,
    ingress_port: u16,
    extra_config: &str,
) {
    let config = format!(
        "\
base-dir = \"{}\"
node-name = \"standalone-test\"
{extra_config}

[admin]
bind-port = {admin_port}
bind-ip = \"127.0.0.1\"
listen-mode = \"tcp\"

[ingress]
bind-port = {ingress_port}
bind-ip = \"127.0.0.1\"
listen-mode = \"tcp\"
",
        base_dir.display()
    );
    std::fs::write(config_path, config).expect("write config");
}

#[allow(dead_code)]
pub async fn run_standalone_expect_failure(extra_config: &str) -> std::process::Output {
    let temp_dir = TempDir::new().expect("temp dir");
    let base_dir = temp_dir.path().to_path_buf();
    let config_path = base_dir.join("standalone.toml");
    write_config(&base_dir, &config_path, 19070, 18080, extra_config);

    Command::new(env!("CARGO_BIN_EXE_restate-standalone"))
        .arg("--config-file")
        .arg(config_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("run standalone failure case")
}

fn allocate_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read local addr")
        .port()
}

async fn wait_for_ready(url: &str) {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(15);

    loop {
        match client.get(url).send().await {
            Ok(response) if response.status().is_success() || response.status().as_u16() == 501 => {
                return;
            }
            Ok(response) if Instant::now() >= deadline => {
                panic!("timed out waiting for {url}: status {}", response.status());
            }
            Err(err) if Instant::now() >= deadline => {
                panic!("timed out waiting for {url}: {err}");
            }
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}
