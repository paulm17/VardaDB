pub mod config;

use clap::{Args, Subcommand};

pub use config::RuntimeSettings;

#[derive(Debug, Clone, Args)]
pub struct RuntimeCli {
    #[command(subcommand)]
    pub command: RuntimeCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum RuntimeCommand {
    /// Start the embedded Restate standalone runtime
    Start(StartCommand),
    /// Manage service deployments
    #[command(subcommand)]
    Deployments(DeploymentCommand),
    /// Inspect registered services
    #[command(subcommand)]
    Services(ServiceCommand),
    /// Invoke a service, virtual object, or workflow handler over ingress
    Invoke(InvokeCommand),
}

#[derive(Debug, Clone, Args)]
pub struct StartCommand {
    /// Override the standalone runtime config file
    #[arg(long = "config-file", value_name = "FILE")]
    pub config_file: Option<std::path::PathBuf>,

    /// Print the standalone runtime config and exit
    #[arg(long)]
    pub dump_config: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DeploymentCommand {
    /// Register a deployment endpoint
    Register(RegisterDeploymentCommand),
    /// List known deployments
    List(AdminEndpointCommand),
    /// Describe a deployment
    Describe(DescribeDeploymentCommand),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ServiceCommand {
    /// List discovered services
    List(AdminEndpointCommand),
    /// Describe a service
    Describe(DescribeServiceCommand),
}

#[derive(Debug, Clone, Args)]
pub struct AdminEndpointCommand {
    /// Override the runtime admin URL
    #[arg(long = "admin-url", value_name = "URL")]
    pub admin_url: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct DescribeDeploymentCommand {
    pub id: String,

    /// Override the runtime admin URL
    #[arg(long = "admin-url", value_name = "URL")]
    pub admin_url: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct DescribeServiceCommand {
    pub name: String,

    /// Override the runtime admin URL
    #[arg(long = "admin-url", value_name = "URL")]
    pub admin_url: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct RegisterDeploymentCommand {
    /// The URL or Lambda ARN that Restate should discover and invoke
    pub deployment: String,

    /// Override the runtime admin URL
    #[arg(long = "admin-url", value_name = "URL")]
    pub admin_url: Option<String>,

    /// Force overwrite when the deployment already exists
    #[arg(long)]
    pub force: bool,

    /// Run discovery without persisting the deployment
    #[arg(long)]
    pub dry_run: bool,

    /// Attempt discovery with an HTTP/1.1-first client
    #[arg(long = "use-http1.1")]
    pub use_http_11: bool,

    /// Optional IAM role ARN to assume for Lambda discovery/invocation
    #[arg(long)]
    pub assume_role_arn: Option<String>,

    /// Additional header sent during discovery. Repeat as needed: --header name=value
    #[arg(long = "header", value_name = "NAME=VALUE")]
    pub headers: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct InvokeCommand {
    pub service: String,
    pub handler: String,

    /// Key for a virtual object or workflow invocation
    #[arg(long)]
    pub key: Option<String>,

    /// Override the runtime ingress URL
    #[arg(long = "ingress-url", value_name = "URL")]
    pub ingress_url: Option<String>,

    /// Treat the request as fire-and-forget and return submission status
    #[arg(long)]
    pub send: bool,

    /// Inline request payload
    #[arg(long, conflicts_with = "data_file")]
    pub data: Option<String>,

    /// Read the request payload from a file
    #[arg(long = "data-file", value_name = "FILE", conflicts_with = "data")]
    pub data_file: Option<std::path::PathBuf>,

    /// Additional request header. Repeat as needed: --header name=value
    #[arg(long = "header", value_name = "NAME=VALUE")]
    pub headers: Vec<String>,
}

pub async fn run(cli: RuntimeCli, vardadb_config: Option<&std::path::Path>) -> anyhow::Result<()> {
    let settings = RuntimeSettings::resolve(vardadb_config)?;

    match cli.command {
        RuntimeCommand::Start(cmd) => start_runtime(cmd, &settings),
        RuntimeCommand::Deployments(cmd) => run_deployments(cmd, &settings).await,
        RuntimeCommand::Services(cmd) => run_services(cmd, &settings).await,
        RuntimeCommand::Invoke(cmd) => invoke(cmd, &settings).await,
    }
}

fn start_runtime(cmd: StartCommand, settings: &RuntimeSettings) -> anyhow::Result<()> {
    restate_standalone::run(restate_standalone::StandaloneRunOptions {
        config_file: cmd.config_file.or_else(|| settings.config_file.clone()),
        dump_config: cmd.dump_config,
    })
}

async fn run_deployments(cmd: DeploymentCommand, settings: &RuntimeSettings) -> anyhow::Result<()> {
    match cmd {
        DeploymentCommand::Register(cmd) => {
            let body = build_register_request(&cmd)?;
            let value = send_json(
                reqwest::Method::POST,
                &admin_url(cmd.admin_url, settings),
                "deployments",
                Some(&body),
            )
            .await?;
            print_json(&value)
        }
        DeploymentCommand::List(cmd) => {
            let value = send_json::<serde_json::Value>(
                reqwest::Method::GET,
                &admin_url(cmd.admin_url, settings),
                "deployments",
                None::<&serde_json::Value>,
            )
            .await?;
            print_json(&value)
        }
        DeploymentCommand::Describe(cmd) => {
            let value = send_json::<serde_json::Value>(
                reqwest::Method::GET,
                &admin_url(cmd.admin_url, settings),
                &format!("deployments/{}", cmd.id),
                None::<&serde_json::Value>,
            )
            .await?;
            print_json(&value)
        }
    }
}

async fn run_services(cmd: ServiceCommand, settings: &RuntimeSettings) -> anyhow::Result<()> {
    match cmd {
        ServiceCommand::List(cmd) => {
            let value = send_json::<serde_json::Value>(
                reqwest::Method::GET,
                &admin_url(cmd.admin_url, settings),
                "services",
                None::<&serde_json::Value>,
            )
            .await?;
            print_json(&value)
        }
        ServiceCommand::Describe(cmd) => {
            let value = send_json::<serde_json::Value>(
                reqwest::Method::GET,
                &admin_url(cmd.admin_url, settings),
                &format!("services/{}", cmd.name),
                None::<&serde_json::Value>,
            )
            .await?;
            print_json(&value)
        }
    }
}

async fn invoke(cmd: InvokeCommand, settings: &RuntimeSettings) -> anyhow::Result<()> {
    let base = cmd
        .ingress_url
        .as_deref()
        .unwrap_or(settings.ingress_url())
        .trim_end_matches('/');
    let suffix = if cmd.send { "/send" } else { "" };
    let url = if let Some(key) = &cmd.key {
        format!("{base}/{}/{}/{}{}", cmd.service, key, cmd.handler, suffix)
    } else {
        format!("{base}/{}/{}{}", cmd.service, cmd.handler, suffix)
    };

    let payload = match (&cmd.data, &cmd.data_file) {
        (Some(data), None) => data.clone().into_bytes(),
        (None, Some(path)) => std::fs::read(path)
            .map_err(|err| anyhow::anyhow!("failed to read payload {}: {err}", path.display()))?,
        (None, None) => Vec::new(),
        _ => unreachable!("clap enforces conflicts"),
    };

    let client = reqwest::Client::new();
    let mut request = client.post(url).body(payload);
    for header in &cmd.headers {
        let (name, value) = parse_header(header)?;
        request = request.header(name, value);
    }

    let response = request.send().await?;
    let status = response.status();
    let bytes = response.bytes().await?;

    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes);
        anyhow::bail!("ingress request failed ({status}): {body}");
    }

    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        return print_json(&json);
    }

    if !bytes.is_empty() {
        println!("{}", String::from_utf8_lossy(&bytes));
    }
    Ok(())
}

fn build_register_request(
    cmd: &RegisterDeploymentCommand,
) -> anyhow::Result<restate_admin_rest_model::deployments::RegisterDeploymentRequest> {
    let headers = if cmd.headers.is_empty() {
        None
    } else {
        Some(parse_headers(&cmd.headers)?)
    };

    if cmd.deployment.starts_with("arn:") {
        Ok(
            restate_admin_rest_model::deployments::RegisterDeploymentRequest::Lambda {
                arn: cmd.deployment.clone(),
                assume_role_arn: cmd.assume_role_arn.clone(),
                additional_headers: headers,
                force: cmd.force,
                dry_run: cmd.dry_run,
            },
        )
    } else {
        let mut uri: http::Uri = cmd
            .deployment
            .parse()
            .map_err(|err| anyhow::anyhow!("invalid deployment URI: {err}"))?;
        let mut parts = uri.into_parts();
        if parts.scheme.is_none() {
            parts.scheme = Some(http::uri::Scheme::HTTP);
        }
        if parts.path_and_query.is_none() {
            parts.path_and_query = Some(http::uri::PathAndQuery::from_static("/"));
        }
        uri = http::Uri::from_parts(parts)
            .map_err(|err| anyhow::anyhow!("invalid deployment URI: {err}"))?;

        Ok(
            restate_admin_rest_model::deployments::RegisterDeploymentRequest::Http {
                uri,
                additional_headers: headers,
                use_http_11: cmd.use_http_11,
                force: cmd.force,
                dry_run: cmd.dry_run,
            },
        )
    }
}

fn admin_url(override_url: Option<String>, settings: &RuntimeSettings) -> String {
    override_url.unwrap_or_else(|| settings.admin_url().to_string())
}

async fn send_json<T: serde::Serialize + ?Sized>(
    method: reqwest::Method,
    base_url: &str,
    path: &str,
    body: Option<&T>,
) -> anyhow::Result<serde_json::Value> {
    let url = format!("{}/{}", base_url.trim_end_matches('/'), path);
    let client = reqwest::Client::new();
    let request = client.request(method, &url);
    let request = if let Some(body) = body {
        request.json(body)
    } else {
        request
    };

    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("request failed ({status}): {body}");
    }

    serde_json::from_str(&body)
        .map_err(|err| anyhow::anyhow!("failed to decode JSON response from {url}: {err}"))
}

fn parse_headers(values: &[String]) -> anyhow::Result<restate_serde_util::SerdeableHeaderHashMap> {
    let mut headers = std::collections::HashMap::with_capacity(values.len());
    for value in values {
        let (name, header_value) = parse_header(value)?;
        headers.insert(name, header_value);
    }
    Ok(headers.into())
}

fn parse_header(raw: &str) -> anyhow::Result<(http::HeaderName, http::HeaderValue)> {
    let Some((name, value)) = raw.split_once('=') else {
        anyhow::bail!("invalid header '{raw}', expected NAME=VALUE");
    };
    let name = name
        .parse()
        .map_err(|err| anyhow::anyhow!("invalid header name '{name}': {err}"))?;
    let value = value
        .parse()
        .map_err(|err| anyhow::anyhow!("invalid header value for '{name}': {err}"))?;
    Ok((name, value))
}

fn print_json(value: &serde_json::Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{admin_url, build_register_request, RuntimeSettings};

    #[test]
    fn admin_url_prefers_override() {
        let settings = RuntimeSettings {
            admin_url: Some("http://configured-admin".to_string()),
            ..RuntimeSettings::default()
        };

        assert_eq!(
            admin_url(Some("http://override-admin".to_string()), &settings),
            "http://override-admin"
        );
    }

    #[test]
    fn build_register_request_adds_http_scheme_and_root_path() {
        let request = build_register_request(&super::RegisterDeploymentCommand {
            deployment: "localhost:8080".to_string(),
            admin_url: None,
            force: false,
            dry_run: false,
            use_http_11: false,
            assume_role_arn: None,
            headers: vec![],
        })
        .expect("request should build");

        match request {
            restate_admin_rest_model::deployments::RegisterDeploymentRequest::Http {
                uri, ..
            } => {
                assert_eq!(uri.to_string(), "http://localhost:8080/");
            }
            other => panic!("expected http request, got {other:?}"),
        }
    }
}
