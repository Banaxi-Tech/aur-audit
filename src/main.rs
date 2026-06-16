mod ai;
mod aur;
mod cli;
mod config;
mod file_reader;
mod inventory;
mod report;

use anyhow::Context;
use cli::{Cli, Commands, ConfigCommand};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse_args();
    match &cli.command {
        Some(Commands::Config {
            command: ConfigCommand::Set(args),
        }) => {
            let path = config::save_config(args).context("failed to save config")?;
            println!("Saved config to {}", path.display());
        }
        Some(Commands::Config {
            command: ConfigCommand::Show,
        }) => {
            let config = config::load_config().context("failed to load config")?;
            println!("{}", config.format_for_display());
        }
        Some(Commands::Config {
            command: ConfigCommand::Path,
        }) => {
            println!("{}", config::default_config_path().display());
        }
        None => {
            let report = report::run(cli).await.context("aur-audit failed")?;
            if !report.is_empty() {
                println!("{report}");
            }
        }
    }
    Ok(())
}
