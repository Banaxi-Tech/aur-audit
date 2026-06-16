use crate::cli::{ConfigSetArgs, Provider};
use anyhow::{bail, Context};
use clap::ValueEnum;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppConfig {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

impl AppConfig {
    pub fn format_for_display(&self) -> String {
        let api_key = match &self.api_key {
            Some(value) if !value.is_empty() => mask_secret(value),
            _ => "(not set)".to_string(),
        };

        format!(
            "Config: {}\nprovider={}\nmodel={}\nbase_url={}\napi_key={}",
            default_config_path().display(),
            self.provider
                .map(|provider| provider.to_string())
                .unwrap_or_else(|| "(not set)".to_string()),
            self.model.as_deref().unwrap_or("(not set)"),
            self.base_url.as_deref().unwrap_or("(not set)"),
            api_key
        )
    }

    fn merge_set_args(&mut self, args: &ConfigSetArgs) {
        if let Some(provider) = args.provider {
            self.provider = Some(provider);
        }
        if let Some(model) = &args.model {
            self.model = Some(model.clone());
        }
        if let Some(base_url) = &args.base_url {
            self.base_url = Some(base_url.clone());
        }
        if args.clear_api_key {
            self.api_key = None;
        } else if let Some(api_key) = &args.api_key {
            self.api_key = Some(api_key.clone());
        }
    }

    fn to_file_contents(&self) -> String {
        let mut out = String::new();
        if let Some(provider) = self.provider {
            out.push_str(&format!("provider={provider}\n"));
        }
        if let Some(model) = &self.model {
            out.push_str(&format!("model={model}\n"));
        }
        if let Some(base_url) = &self.base_url {
            out.push_str(&format!("base_url={base_url}\n"));
        }
        if let Some(api_key) = &self.api_key {
            out.push_str(&format!("api_key={api_key}\n"));
        }
        out
    }
}

pub fn default_config_path() -> PathBuf {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home).join("aur-audit").join("config");
    }

    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home)
        .join(".config")
        .join("aur-audit")
        .join("config")
}

pub fn load_config() -> anyhow::Result<AppConfig> {
    let path = default_config_path();
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let contents =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    parse_config(&contents).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn save_config(args: &ConfigSetArgs) -> anyhow::Result<PathBuf> {
    let path = default_config_path();
    let mut config = load_config()?;
    config.merge_set_args(args);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, config.to_file_contents())
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(path)
}

fn parse_config(contents: &str) -> anyhow::Result<AppConfig> {
    let mut config = AppConfig::default();

    for (index, raw_line) in contents.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            bail!("line {} is not key=value", index + 1);
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "provider" => config.provider = Some(parse_provider(value)?),
            "model" => config.model = non_empty(value),
            "base_url" => config.base_url = non_empty(value),
            "api_key" => config.api_key = non_empty(value),
            _ => bail!("line {} has unknown config key {key:?}", index + 1),
        }
    }

    Ok(config)
}

fn parse_provider(value: &str) -> anyhow::Result<Provider> {
    Provider::from_str(value, true).map_err(|err| anyhow::anyhow!("{err}"))
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn mask_secret(value: &str) -> String {
    if value.len() <= 8 {
        return "********".to_string();
    }
    format!("{}...{}", &value[..4], &value[value.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_value_config() {
        let config = parse_config(
            "provider=lmstudio\nmodel=gemma-4-12b-it-qat\nbase_url=http://localhost:1234/v1\napi_key=secret\n",
        )
        .unwrap();

        assert_eq!(config.provider, Some(Provider::Lmstudio));
        assert_eq!(config.model.as_deref(), Some("gemma-4-12b-it-qat"));
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:1234/v1"));
        assert_eq!(config.api_key.as_deref(), Some("secret"));
    }

    #[test]
    fn rejects_unknown_config_key() {
        assert!(parse_config("wat=yes\n").is_err());
    }
}
