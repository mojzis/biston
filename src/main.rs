use std::io::{BufRead, IsTerminal};
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

        /// Only emit pairs involving this file (repeat the flag for multiple files).
        /// The rest of the directory is still scanned so cross-file clones
        /// between a focus file and the rest of the repo are still detected.
        ///
        /// Note: `--files $(git diff --name-only)` will silently expand to an
        /// empty flag when no files changed, which reverts to a full scan.
        /// For commit hooks, prefer `--files-from -` piped from `git diff`:
        /// that cleanly handles the empty case as "no pairs to emit."
        #[arg(long = "files", value_name = "FILE", action = clap::ArgAction::Append)]
        files: Vec<PathBuf>,

        /// Read focus file list from this path (one path per line).
        /// Use `-` to read from stdin. An empty list means "no focus files"
        /// and suppresses all pairs — ideal for commit hooks that pipe
        /// `git diff --name-only` directly.
        #[arg(long = "files-from", value_name = "PATH", conflicts_with = "files")]
        files_from: Option<PathBuf>,
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

        /// Only emit pairs involving this file (repeat for multiple files).
        /// See `scan --files` for details.
        #[arg(long = "files", value_name = "FILE", action = clap::ArgAction::Append)]
        files: Vec<PathBuf>,

        /// Read focus file list from this path (one path per line).
        /// Use `-` to read from stdin.
        #[arg(long = "files-from", value_name = "PATH", conflicts_with = "files")]
        files_from: Option<PathBuf>,
    },
}

/// Resolve the focus file list from the two mutually-exclusive CLI options.
///
/// Returns `None` when the user supplied neither flag (= scan everything, no
/// filtering). Returns `Some(vec)` otherwise — an empty vec is a valid "no
/// files changed" signal and suppresses all pairs.
fn resolve_focus_files(
    files: Vec<PathBuf>,
    files_from: Option<PathBuf>,
) -> anyhow::Result<Option<Vec<PathBuf>>> {
    if let Some(source) = files_from {
        let lines = read_file_list(&source)
            .with_context(|| format!("failed to read file list from {}", source.display()))?;
        return Ok(Some(lines));
    }
    if !files.is_empty() {
        return Ok(Some(files));
    }
    Ok(None)
}

/// Read a newline-separated list of paths from a file, or stdin when `source`
/// is `-`. Blank lines are ignored.
fn read_file_list(source: &std::path::Path) -> anyhow::Result<Vec<PathBuf>> {
    let reader: Box<dyn BufRead> = if source == std::path::Path::new("-") {
        Box::new(std::io::BufReader::new(std::io::stdin().lock()))
    } else {
        Box::new(std::io::BufReader::new(
            std::fs::File::open(source).context("failed to open file list")?,
        ))
    };
    let mut paths = Vec::new();
    for line in reader.lines() {
        let line = line.context("failed to read line from file list")?;
        // Strip leading UTF-8 BOM (from files produced by some Windows tools)
        // and surrounding whitespace. `trim` also handles CRLF.
        let trimmed = line.trim().trim_start_matches('\u{FEFF}');
        if trimmed.is_empty() {
            continue;
        }
        paths.push(PathBuf::from(trimmed));
    }
    Ok(paths)
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
        Commands::Scan {
            path,
            format,
            min_lines,
            threshold,
            config: config_dir,
            suggest,
            files,
            files_from,
        } => {
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

            let focus_files = resolve_focus_files(files, files_from)?;
            let report = biston::scan_focused(&path, &config, focus_files.as_deref())?;

            let output = match config.output.format {
                OutputFormat::Text => report::format_text(&report, &config.output),
                OutputFormat::Json => report::format_json(&report, &config.output)?,
                OutputFormat::Sarif => report::format_sarif(&report, &config.output)?,
            };

            print!("{output}");

            Ok(())
        }
        Commands::Stats {
            path,
            format,
            min_lines,
            threshold,
            config: config_dir,
            files,
            files_from,
        } => {
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

            let focus_files = resolve_focus_files(files, files_from)?;
            let report = biston::scan_focused(&path, &config, focus_files.as_deref())?;
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
