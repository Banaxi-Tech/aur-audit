pub mod lmstudio;
pub mod ollama;
pub mod openrouter;

use crate::cli::Provider;
use anyhow::Context;
use colored::Colorize;
use std::io::{self, Write};

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub provider: Provider,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl ProviderConfig {
    pub fn from_options(
        provider: Provider,
        model: Option<String>,
        base_url: Option<String>,
        api_key: Option<String>,
    ) -> Self {
        let base_url = base_url
            .or_else(|| std::env::var(provider.env_base_url()).ok())
            .unwrap_or_else(|| provider.default_base_url().to_string());
        let model = model.unwrap_or_else(|| provider.default_model().to_string());
        let api_key = api_key.or_else(|| std::env::var(provider.env_api_key()).ok());
        Self {
            provider,
            model,
            base_url: trim_trailing_slash(base_url),
            api_key,
        }
    }
}

#[derive(Clone)]
pub struct AiClient {
    http: reqwest::Client,
    config: ProviderConfig,
}

impl AiClient {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    pub async fn chat(&self, prompt: &str) -> anyhow::Result<String> {
        match self.config.provider {
            Provider::Openrouter => openrouter::chat(&self.http, &self.config, prompt).await,
            Provider::Ollama => ollama::chat(&self.http, &self.config, prompt).await,
            Provider::Lmstudio => lmstudio::chat(&self.http, &self.config, prompt).await,
        }
        .with_context(|| format!("AI provider {:?} request failed", self.config.provider))
    }

    pub async fn chat_stream_stdout(&self, prompt: &str) -> anyhow::Result<String> {
        match self.config.provider {
            Provider::Openrouter => {
                openrouter::chat_stream_stdout(&self.http, &self.config, prompt).await
            }
            Provider::Ollama => ollama::chat_stream_stdout(&self.http, &self.config, prompt).await,
            Provider::Lmstudio => {
                lmstudio::chat_stream_stdout(&self.http, &self.config, prompt).await
            }
        }
        .with_context(|| {
            format!(
                "AI provider {:?} streaming request failed",
                self.config.provider
            )
        })
    }
}

fn trim_trailing_slash(value: String) -> String {
    value.trim_end_matches('/').to_string()
}

pub(crate) struct StreamPrinter {
    pending_line: String,
    output: String,
}

impl StreamPrinter {
    pub(crate) fn new() -> Self {
        Self {
            pending_line: String::new(),
            output: String::new(),
        }
    }

    pub(crate) fn push(&mut self, content: &str) -> anyhow::Result<()> {
        self.output.push_str(content);
        self.pending_line.push_str(content);

        while let Some(newline) = self.pending_line.find('\n') {
            let line = self.pending_line[..newline].to_string();
            self.pending_line = self.pending_line[newline + 1..].to_string();
            println!("{}", color_status_labels(&line));
            io::stdout().flush()?;
        }

        Ok(())
    }

    pub(crate) fn finish(self) -> anyhow::Result<String> {
        if !self.pending_line.is_empty() {
            println!("{}", color_status_labels(&self.pending_line));
        } else {
            println!();
        }
        io::stdout().flush()?;
        Ok(self.output)
    }
}

fn color_status_labels(line: &str) -> String {
    let Some((prefix, label, suffix)) = split_status_label(line) else {
        return line.to_string();
    };
    let colored = match label {
        "GOOD" | "SAFE" => label.green().bold().to_string(),
        "SUSPICIOUS" => label.truecolor(255, 165, 0).bold().to_string(),
        "UNSAFE" => label.red().bold().to_string(),
        _ => label.to_string(),
    };
    format!("{prefix}{colored}{suffix}")
}

fn split_status_label<'a>(line: &'a str) -> Option<(String, &'a str, &'a str)> {
    let trimmed = line.trim_start();
    let prefix_len = line.len() - trimmed.len();
    let candidate = trimmed.strip_prefix("- ").unwrap_or(trimmed);
    let bullet_len = trimmed.len() - candidate.len();
    let prefix = line[..prefix_len + bullet_len].to_string();

    for label in ["SUSPICIOUS", "UNSAFE", "GOOD", "SAFE"] {
        if let Some(suffix) = candidate.strip_prefix(label) {
            if suffix.starts_with(',') || suffix.starts_with(':') || suffix.is_empty() {
                return Some((prefix, label, suffix));
            }
        }
    }

    if let Some(suffix) = candidate.strip_prefix("Verdict: ") {
        for label in ["SUSPICIOUS", "UNSAFE", "SAFE"] {
            if let Some(rest) = suffix.strip_prefix(label) {
                return Some((format!("{prefix}Verdict: "), label, rest));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Provider;

    #[test]
    fn provider_configuration_uses_defaults() {
        std::env::remove_var("AUR_AUDIT_OLLAMA_BASE_URL");
        let config = ProviderConfig::from_options(Provider::Ollama, None, None, None);
        assert_eq!(config.model, "qwen3.5-coder");
        assert_eq!(config.base_url, "http://localhost:11434");
    }

    #[test]
    fn provider_configuration_uses_model_and_endpoint_override() {
        std::env::set_var("AUR_AUDIT_LMSTUDIO_BASE_URL", "http://127.0.0.1:9999/v1/");
        let config =
            ProviderConfig::from_options(Provider::Lmstudio, Some("abc".to_string()), None, None);
        assert_eq!(config.model, "abc");
        assert_eq!(config.base_url, "http://127.0.0.1:9999/v1");
        std::env::remove_var("AUR_AUDIT_LMSTUDIO_BASE_URL");
    }

    #[test]
    fn provider_configuration_prefers_explicit_base_url_and_api_key() {
        std::env::set_var("OPENROUTER_API_KEY", "env-key");
        let config = ProviderConfig::from_options(
            Provider::Openrouter,
            None,
            Some("http://example.test/v1/".to_string()),
            Some("config-key".to_string()),
        );
        assert_eq!(config.base_url, "http://example.test/v1");
        assert_eq!(config.api_key.as_deref(), Some("config-key"));
        std::env::remove_var("OPENROUTER_API_KEY");
    }
}
