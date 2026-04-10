use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};

use biston::config::{Config, OutputFormat};
use biston::report;
use biston::stats;

#[derive(Parser)]
#[command(name = "biston", about = "Structural clone detector for Python")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan a directory for code clones
    Scan {
        /// Directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,

        /// Minimum function length in lines
        #[arg(long)]
        min_lines: Option<usize>,

        /// Similarity threshold (0.0 - 1.0)
        #[arg(long)]
        threshold: Option<f64>,

        /// Config file directory (looks for biston.toml or pyproject.toml)
        #[arg(long)]
        config: Option<PathBuf>,

        /// Generate abstraction suggestions for similar pairs
        #[arg(long)]
        suggest: bool,
    },

    /// Show statistics about scan findings
    Stats {
        /// Directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format (text or json)
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,

        /// Minimum function length in lines
        #[arg(long)]
        min_lines: Option<usize>,

        /// Similarity threshold (0.0 - 1.0)
        #[arg(long)]
        threshold: Option<f64>,

        /// Config file directory (looks for biston.toml or pyproject.toml)
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Scan { path, format, min_lines, threshold, config: config_dir, suggest } => {
            // Load config from directory (or scan path)
            let config_path = config_dir.as_deref().unwrap_or(&path);
            let mut config = Config::load(config_path).context("failed to load config")?;

            // Apply CLI overrides
            if let Some(fmt) = format {
                config.output.format = fmt;
            }
            if let Some(ml) = min_lines {
                config.scan.min_lines = ml;
            }
            if let Some(th) = threshold {
                config.scan.threshold = th;
            }
            if suggest {
                config.suggest.enabled = true;
            }

            if config.output.format == OutputFormat::Text && std::io::stdout().is_terminal() {
                config.output.color = true;
            }

            let report = biston::scan(&path, &config)?;

            let output = match config.output.format {
                OutputFormat::Text => report::format_text(&report, &config.output),
                OutputFormat::Json => report::format_json(&report, &config.output)?,
                OutputFormat::Sarif => report::format_sarif(&report, &config.output)?,
            };

            print!("{output}");

            Ok(())
        }
        Commands::Stats { path, format, min_lines, threshold, config: config_dir } => {
            let config_path = config_dir.as_deref().unwrap_or(&path);
            let mut config = Config::load(config_path).context("failed to load config")?;

            if let Some(fmt) = format {
                config.output.format = fmt;
            }
            if let Some(ml) = min_lines {
                config.scan.min_lines = ml;
            }
            if let Some(th) = threshold {
                config.scan.threshold = th;
            }

            let report = biston::scan(&path, &config)?;
            let scan_stats = stats::compute_stats(&report);

            let output = match config.output.format {
                OutputFormat::Json => stats::format_stats_json(&scan_stats)?,
                OutputFormat::Text | OutputFormat::Sarif => stats::format_stats_text(&scan_stats),
            };

            print!("{output}");

            Ok(())
        }
    }
}
