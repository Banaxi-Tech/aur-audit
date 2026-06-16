use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Provider {
    Openrouter,
    Ollama,
    Lmstudio,
}

impl Provider {
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Openrouter => "openai/gpt-5.5",
            Provider::Ollama => "qwen3.5-coder",
            Provider::Lmstudio => "local-model",
        }
    }

    pub fn env_base_url(self) -> &'static str {
        match self {
            Provider::Openrouter => "AUR_AUDIT_OPENROUTER_BASE_URL",
            Provider::Ollama => "AUR_AUDIT_OLLAMA_BASE_URL",
            Provider::Lmstudio => "AUR_AUDIT_LMSTUDIO_BASE_URL",
        }
    }

    pub fn env_api_key(self) -> &'static str {
        match self {
            Provider::Openrouter => "OPENROUTER_API_KEY",
            Provider::Ollama => "AUR_AUDIT_OLLAMA_API_KEY",
            Provider::Lmstudio => "AUR_AUDIT_LMSTUDIO_API_KEY",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::Openrouter => "https://openrouter.ai/api/v1",
            Provider::Ollama => "http://localhost:11434",
            Provider::Lmstudio => "http://localhost:1234/v1",
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Provider::Openrouter => "openrouter",
            Provider::Ollama => "ollama",
            Provider::Lmstudio => "lmstudio",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Parser)]
#[command(name = "aur-audit")]
#[command(about = "Audit an AUR package with an AI model")]
pub struct Cli {
    pub package_name: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(long, value_enum)]
    pub provider: Option<Provider>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub base_url: Option<String>,

    #[arg(long)]
    pub api_key: Option<String>,

    #[arg(long)]
    pub keep_temp: bool,

    #[arg(long)]
    pub verbose: bool,

    #[arg(long, hide = true)]
    pub local_repo: Option<PathBuf>,

    #[arg(long, default_value_t = 1_048_576)]
    pub max_file_bytes: usize,

    #[arg(long, default_value_t = 800_000)]
    pub max_total_bytes: usize,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    Set(ConfigSetArgs),
    Show,
    Path,
}

#[derive(Debug, Clone, Args)]
pub struct ConfigSetArgs {
    #[arg(long, value_enum)]
    pub provider: Option<Provider>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub base_url: Option<String>,

    #[arg(long)]
    pub api_key: Option<String>,

    #[arg(long)]
    pub clear_api_key: bool,
}
