use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RuntimeSettings {
    #[serde(alias = "config_path", alias = "standalone-config-file")]
    pub config_file: Option<PathBuf>,
    pub admin_url: Option<String>,
    pub ingress_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RuntimeConfigDocument {
    runtime: RuntimeSettings,
}

impl RuntimeSettings {
    pub fn resolve(path: Option<&Path>) -> anyhow::Result<Self> {
        match path {
            Some(path) => Self::load_from_vardadb_config(path),
            None => Ok(Self::default()),
        }
    }

    pub fn load_from_vardadb_config(path: &Path) -> anyhow::Result<Self> {
        let contents = fs::read_to_string(path).map_err(|err| {
            anyhow::anyhow!("failed to read VardaDB config {}: {err}", path.display())
        })?;
        let doc: RuntimeConfigDocument = toml::from_str(&contents).map_err(|err| {
            anyhow::anyhow!("failed to parse VardaDB config {}: {err}", path.display())
        })?;
        Ok(doc.runtime)
    }

    pub fn admin_url(&self) -> &str {
        self.admin_url.as_deref().unwrap_or("http://127.0.0.1:9070")
    }

    pub fn ingress_url(&self) -> &str {
        self.ingress_url
            .as_deref()
            .unwrap_or("http://127.0.0.1:9080")
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeSettings;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("vardadb-runtime-{name}-{unique}.toml"))
    }

    #[test]
    fn resolve_defaults_when_config_is_absent() {
        let settings = RuntimeSettings::resolve(None).expect("defaults should resolve");
        assert_eq!(settings.admin_url(), "http://127.0.0.1:9070");
        assert_eq!(settings.ingress_url(), "http://127.0.0.1:9080");
        assert!(settings.config_file.is_none());
    }

    #[test]
    fn load_runtime_section_from_vardadb_config() {
        let path = temp_config_path("load");
        fs::write(
            &path,
            r#"
[runtime]
config-file = "runtime-standalone.toml"
admin-url = "http://127.0.0.1:19070"
ingress-url = "http://127.0.0.1:19080"
"#,
        )
        .expect("write config");

        let settings =
            RuntimeSettings::load_from_vardadb_config(&path).expect("config should parse");
        assert_eq!(
            settings.config_file,
            Some(PathBuf::from("runtime-standalone.toml"))
        );
        assert_eq!(settings.admin_url(), "http://127.0.0.1:19070");
        assert_eq!(settings.ingress_url(), "http://127.0.0.1:19080");

        fs::remove_file(path).ok();
    }

    #[test]
    fn missing_runtime_section_uses_defaults() {
        let path = temp_config_path("default");
        fs::write(
            &path,
            r#"
[server]
port = 7171
"#,
        )
        .expect("write config");

        let settings =
            RuntimeSettings::load_from_vardadb_config(&path).expect("config should parse");
        assert_eq!(settings.admin_url(), "http://127.0.0.1:9070");
        assert_eq!(settings.ingress_url(), "http://127.0.0.1:9080");

        fs::remove_file(path).ok();
    }

    #[test]
    fn explicit_missing_config_path_errors() {
        let path = temp_config_path("missing");
        let err = RuntimeSettings::resolve(Some(&path)).expect_err("missing file should error");
        assert!(err.to_string().contains("failed to read VardaDB config"));
    }

    #[test]
    fn malformed_config_errors() {
        let path = temp_config_path("malformed");
        fs::write(&path, "[runtime\nadmin-url = 123").expect("write malformed config");

        let err =
            RuntimeSettings::load_from_vardadb_config(&path).expect_err("malformed config errors");
        assert!(err.to_string().contains("failed to parse VardaDB config"));

        fs::remove_file(path).ok();
    }
}
