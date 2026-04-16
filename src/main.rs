use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};

use biston::config::{Config, OutputFormat};
use biston::report;
use biston::stats;

#[derive(Parser)]
#[command(name = "biston", about = "Structural clone detector for Python")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// CLI options shared by every subcommand.
///
/// Flattened into each `Commands` variant so the flag set and override
/// precedence stay in a single place. Subcommand-specific flags (like
/// `--suggest` for `scan`) live alongside the flattened `CommonOpts`.
#[derive(Args)]
struct CommonOpts {
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

    /// Restrict the scan to Python test files (overrides include/exclude)
    #[arg(long)]
    tests_only: bool,
}

impl CommonOpts {
    /// Load config from disk and apply the shared CLI overrides.
    fn resolve(&self) -> anyhow::Result<Config> {
        let config_path = self.config.as_deref().unwrap_or(&self.path);
        let mut config = Config::load(config_path).context("failed to load config")?;

        if let Some(fmt) = self.format {
            config.output.format = fmt;
        }
        if let Some(ml) = self.min_lines {
            config.scan.min_lines = ml;
        }
        if let Some(th) = self.threshold {
            config.scan.threshold = th;
        }
        if self.tests_only {
            config.scan.scope_to_tests();
        }

        Ok(config)
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Scan a directory for code clones
    Scan {
        #[command(flatten)]
        common: CommonOpts,

        /// Generate abstraction suggestions for similar pairs
        #[arg(long)]
        suggest: bool,
    },

    /// Show statistics about scan findings
    Stats {
        #[command(flatten)]
        common: CommonOpts,
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
        Commands::Scan { common, suggest } => {
            let mut config = common.resolve()?;
            if suggest {
                config.suggest.enabled = true;
            }

            if config.output.format == OutputFormat::Text && std::io::stdout().is_terminal() {
                config.output.color = true;
            }

            let report = biston::scan(&common.path, &config)?;

            let output = match config.output.format {
                OutputFormat::Text => report::format_text(&report, &config.output),
                OutputFormat::Json => report::format_json(&report, &config.output)?,
                OutputFormat::Sarif => report::format_sarif(&report, &config.output)?,
            };

            print!("{output}");

            Ok(())
        }
        Commands::Stats { common } => {
            let config = common.resolve()?;

            let report = biston::scan(&common.path, &config)?;
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
