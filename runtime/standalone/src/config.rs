// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml::Value;

use restate_time_util::NonZeroFriendlyDuration;
use restate_types::config::{
    CommonOptions, InvokerOptions, ListenerOptions, LogFormat, TracingOptions,
};
use restate_types::net::address::{AdminPort, HttpIngressPort};

#[derive(Debug, Clone)]
pub(crate) struct LoadedStandaloneConfig {
    pub(crate) common: CommonOptions,
    pub(crate) invoker_options: InvokerOptions,
    pub(crate) node_name: String,
    pub(crate) base_dir: PathBuf,
    pub(crate) storage_dir: PathBuf,
    pub(crate) admin_listener_options: ListenerOptions<AdminPort>,
    pub(crate) ingress_listener_options: ListenerOptions<HttpIngressPort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct StandaloneConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    node_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_dir: Option<PathBuf>,
    #[serde(default = "default_shutdown_timeout")]
    shutdown_timeout: NonZeroFriendlyDuration,
    #[serde(default)]
    log_filter: String,
    #[serde(default)]
    log_format: LogFormat,
    #[serde(default)]
    log_disable_ansi_codes: bool,
    #[serde(flatten)]
    tracing: StandaloneTracingOptions,
    #[serde(default)]
    storage: StandaloneStorageOptions,
    #[serde(default)]
    admin: StandaloneAdminOptions,
    #[serde(default)]
    ingress: StandaloneIngressOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct StandaloneStorageOptions {
    #[serde(default = "default_sqlite_dir")]
    sqlite_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct StandaloneTracingOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracing_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracing_runtime_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracing_services_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracing_json_path: Option<String>,
    #[serde(default = "default_tracing_filter")]
    tracing_filter: String,
    #[serde(
        default,
        skip_serializing_if = "restate_serde_util::SerdeableHeaderHashMap::is_empty"
    )]
    tracing_headers: restate_serde_util::SerdeableHeaderHashMap,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct StandaloneAdminOptions {
    #[serde(flatten)]
    listener_options: ListenerOptions<AdminPort>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct StandaloneIngressOptions {
    #[serde(flatten)]
    listener_options: ListenerOptions<HttpIngressPort>,
}

impl StandaloneConfig {
    pub(crate) fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        match path {
            Some(path) => {
                let contents = fs::read_to_string(path).map_err(|err| {
                    anyhow::anyhow!("failed to read config file {}: {err}", path.display())
                })?;
                toml::from_str(&contents).map_err(|err| {
                    anyhow::anyhow!(
                        "failed to parse standalone config file {}: {err}",
                        path.display()
                    )
                })
            }
            None => Ok(Self::default()),
        }
    }

    pub(crate) fn dump(&self) -> anyhow::Result<String> {
        toml::to_string_pretty(self)
            .map_err(|err| anyhow::anyhow!("standalone config is not TOML serializable: {err}"))
    }

    pub(crate) fn into_loaded(self) -> anyhow::Result<LoadedStandaloneConfig> {
        let mut value = Value::try_from(CommonOptions::default())
            .map_err(|err| anyhow::anyhow!("serialize default standalone common options: {err}"))?;
        let table = value.as_table_mut().ok_or_else(|| {
            anyhow::anyhow!("default standalone common options must serialize as a TOML table")
        })?;

        table.insert(
            "shutdown-timeout".to_owned(),
            Value::String(self.shutdown_timeout.to_string()),
        );
        table.insert("log-filter".to_owned(), Value::String(self.log_filter));
        table.insert(
            "log-format".to_owned(),
            Value::String(
                toml::Value::try_from(self.log_format)
                    .map_err(|err| anyhow::anyhow!("serialize standalone log format: {err}"))?
                    .as_str()
                    .expect("log format serializes to string")
                    .to_owned(),
            ),
        );
        table.insert(
            "log-disable-ansi-codes".to_owned(),
            Value::Boolean(self.log_disable_ansi_codes),
        );
        table.insert("disable-prometheus".to_owned(), Value::Boolean(true));
        table.insert("disable-telemetry".to_owned(), Value::Boolean(true));

        if let Some(node_name) = self.node_name {
            table.insert("node-name".to_owned(), Value::String(node_name));
        }
        if let Some(base_dir) = self.base_dir {
            table.insert(
                "base-dir".to_owned(),
                Value::String(base_dir.display().to_string()),
            );
        }

        for (key, value) in Value::try_from(TracingOptions::from(self.tracing))
            .map_err(|err| anyhow::anyhow!("serialize standalone tracing config: {err}"))?
            .as_table()
            .expect("tracing config serializes to a table")
        {
            table.insert(key.clone(), value.clone());
        }

        let common: CommonOptions = value
            .try_into()
            .map_err(|err| anyhow::anyhow!("deserialize standalone common options: {err}"))?;

        let base_dir = common.base_dir().join(common.node_name());
        let storage_dir = resolve_storage_dir(&base_dir, &self.storage.sqlite_dir);

        Ok(LoadedStandaloneConfig {
            node_name: common.node_name().to_owned(),
            admin_listener_options: self.admin.listener_options,
            ingress_listener_options: self.ingress.listener_options,
            storage_dir,
            base_dir,
            common,
            invoker_options: InvokerOptions::default(),
        })
    }
}

impl Default for StandaloneConfig {
    fn default() -> Self {
        Self {
            node_name: None,
            base_dir: None,
            shutdown_timeout: default_shutdown_timeout(),
            log_filter: "info".to_owned(),
            log_format: LogFormat::Pretty,
            log_disable_ansi_codes: false,
            tracing: StandaloneTracingOptions::default(),
            storage: StandaloneStorageOptions::default(),
            admin: StandaloneAdminOptions::default(),
            ingress: StandaloneIngressOptions::default(),
        }
    }
}

impl Default for StandaloneStorageOptions {
    fn default() -> Self {
        Self {
            sqlite_dir: default_sqlite_dir(),
        }
    }
}

impl Default for StandaloneTracingOptions {
    fn default() -> Self {
        Self {
            tracing_endpoint: None,
            tracing_runtime_endpoint: None,
            tracing_services_endpoint: None,
            tracing_json_path: None,
            tracing_filter: default_tracing_filter(),
            tracing_headers: Default::default(),
        }
    }
}

impl From<StandaloneTracingOptions> for TracingOptions {
    fn from(value: StandaloneTracingOptions) -> Self {
        Self {
            tracing_endpoint: value.tracing_endpoint,
            tracing_runtime_endpoint: value.tracing_runtime_endpoint,
            tracing_services_endpoint: value.tracing_services_endpoint,
            tracing_json_path: value.tracing_json_path,
            tracing_filter: value.tracing_filter,
            tracing_headers: value.tracing_headers,
        }
    }
}

fn resolve_storage_dir(base_dir: &Path, sqlite_dir: &Path) -> PathBuf {
    if sqlite_dir.is_absolute() {
        sqlite_dir.to_path_buf()
    } else {
        base_dir.join(sqlite_dir)
    }
}

fn default_shutdown_timeout() -> NonZeroFriendlyDuration {
    NonZeroFriendlyDuration::from_secs_unchecked(60)
}

fn default_sqlite_dir() -> PathBuf {
    PathBuf::from("sqlite")
}

fn default_tracing_filter() -> String {
    "info".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_config_rejects_unknown_legacy_fields() {
        let err = toml::from_str::<StandaloneConfig>(
            r#"
            legacy-mode = "compat"
            [runtime]
            mode = "legacy"
            "#,
        )
        .expect_err("unknown legacy keys must be rejected");

        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn standalone_config_accepts_minimal_schema() {
        let config = toml::from_str::<StandaloneConfig>(
            r#"
            node-name = "standalone"

            [admin]
            bind-port = 9070

            [ingress]
            bind-port = 8080

            [storage]
            sqlite-dir = "sqlite-data"
            "#,
        )
        .expect("parse standalone config");

        let loaded = config.into_loaded().expect("load standalone config");
        assert_eq!(loaded.node_name, "standalone");
        assert!(loaded.storage_dir.ends_with("sqlite-data"));
        assert!(loaded.common.disable_prometheus);
    }

    #[test]
    fn standalone_config_rejects_partition_count() {
        let err = toml::from_str::<StandaloneConfig>(
            r#"
            num-partitions = 3
            "#,
        )
        .expect_err("partition count must be rejected");

        assert!(err.to_string().contains("num-partitions"));
    }
}
