use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

use biston::config::{Config, OutputFormat};
use biston::guide::{self, Topic};
use biston::overview;
use biston::report;
use biston::stats;

/// Nothing to report: the tree is clean, or the subcommand only printed text.
const EXIT_CLEAN: ExitCode = ExitCode::SUCCESS;
/// Findings were reported. A check aggregator that never sees this is a gate
/// that never trips, which is the whole reason `scan` has an exit code at all.
const EXIT_FINDINGS: ExitCode = ExitCode::FAILURE;
/// biston could not do its job: bad usage, unreadable path, invalid config.
/// Distinct from `EXIT_FINDINGS` so a gate can tell "duplication" from "broken",
/// and matching the code clap already uses for a usage error.
///
/// A `u8` rather than an `ExitCode` like its two siblings only because
/// `ExitCode::from` is not `const`.
const EXIT_ERROR: u8 = 2;

#[derive(Parser)]
#[command(name = "biston", about = "Structural clone detector for Python", version)]
struct Cli {
    /// Colorize output: `auto` honours `NO_COLOR` and TTY detection.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, global = true)]
    color: ColorChoice,

    /// Increase log verbosity: `-v` info, `-vv` debug, `-vvv` trace. `RUST_LOG` overrides.
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count, global = true, conflicts_with = "quiet")]
    verbose: u8,

    /// Suppress warnings (only errors print). `RUST_LOG` overrides.
    #[arg(short = 'q', long = "quiet", global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    /// Resolve to a concrete on/off decision given the runtime environment.
    ///
    /// Precedence in `Auto` mode: `NO_COLOR` (any value) disables colour,
    /// then falls back to stdout TTY detection. `Always`/`Never` are explicit
    /// user overrides and ignore both signals.
    fn resolve(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal(),
        }
    }
}

/// Convert `-v`/`-q` counts into a tracing filter directive.
fn verbosity_filter(verbose: u8, quiet: bool) -> &'static str {
    if quiet {
        return "error";
    }
    match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    }
}

/// CLI options shared by every subcommand.
///
/// Flattened into each `Commands` variant so the flag set and override
/// precedence stay in a single place. Subcommand-specific flags (like
/// `--suggest` for `scan`) live alongside the flattened `CommonOpts`.
#[derive(Args)]
struct CommonOpts {
    /// Scan root (default `.`), or focus files when `--focus-args` is set.
    ///
    /// Without `--focus-args`, at most one positional is accepted and it
    /// names the directory to scan. With `--focus-args`, every positional
    /// is interpreted as a focus file and the scan root is implicitly `.`.
    #[arg(value_name = "PATH", num_args = 0..)]
    positionals: Vec<PathBuf>,

    /// Output format
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,

    /// Minimum function length in executable lines (alias: sets both tier floors)
    #[arg(long)]
    min_lines: Option<usize>,

    /// Executable lines the shorter function needs for an exact match [default: 5]
    #[arg(long)]
    exact_min_lines: Option<usize>,

    /// Executable lines the shorter function needs for a fuzzy match [default: 9]
    #[arg(long)]
    similar_min_lines: Option<usize>,

    /// Statements a body needs for an exact match to be reported [default: 3]
    #[arg(long)]
    exact_min_stmts: Option<usize>,

    /// Executable lines an exactly-matched contained run needs [default: 10]
    #[arg(long)]
    exact_min_fragment_lines: Option<usize>,

    /// Executable lines a fuzzily-matched contained run needs [default: 15]
    #[arg(long)]
    similar_min_fragment_lines: Option<usize>,

    /// Similarity threshold (0.0 - 1.0)
    #[arg(long)]
    threshold: Option<f64>,

    /// Config file directory (looks for biston.toml or pyproject.toml)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Restrict the scan to Python test files (overrides include/exclude)
    #[arg(long)]
    tests_only: bool,

    /// Also report functions that already implement the leading or trailing run
    /// of another function's body ("you already wrote this, call it").
    ///
    /// Off by default. Tune with the `[containment]` config section.
    #[arg(long)]
    containment: bool,

    /// Only emit pairs involving this file (repeat the flag for multiple files).
    /// The rest of the tree is still scanned so cross-file clones between a
    /// focus file and the rest of the repo are still detected.
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

    /// Interpret positional arguments as focus files (scan root is `.`).
    ///
    /// Designed for `pre-commit` / `prek` integration: the framework passes
    /// changed files as trailing positional arguments. An empty list is a
    /// silent pass, matching `--files-from -` with empty stdin.
    #[arg(long = "focus-args", conflicts_with_all = ["files", "files_from"])]
    focus_args: bool,
}

impl CommonOpts {
    /// Resolve the scan root from the positional arguments and `--focus-args`.
    ///
    /// Without `--focus-args`: 0 positionals → `.`, 1 positional → that path,
    /// 2+ positionals → error. With `--focus-args`: always `.` (positionals
    /// are focus files, not paths).
    fn scan_path(&self) -> anyhow::Result<PathBuf> {
        if self.focus_args {
            return Ok(PathBuf::from("."));
        }
        match self.positionals.as_slice() {
            [] => Ok(PathBuf::from(".")),
            [p] => Ok(p.clone()),
            extras => anyhow::bail!(
                "too many positional arguments ({}): pass --focus-args to \
                 interpret positionals as focus files",
                extras.len(),
            ),
        }
    }

    /// Load config from disk and apply the shared CLI overrides.
    ///
    /// Returns the resolved `Config` alongside the scan root so callers don't
    /// have to compute the scan path twice.
    fn resolve(&self) -> anyhow::Result<(Config, PathBuf)> {
        let scan_path = self.scan_path()?;
        let config_path = self.config.as_deref().unwrap_or(&scan_path);
        let mut config = Config::load(config_path).context("failed to load config")?;

        if let Some(fmt) = self.format {
            config.output.format = fmt;
        }
        if let Some(min_lines) = self.min_lines {
            config.scan.min_lines = Some(min_lines);
        }
        if let Some(exact) = self.exact_min_lines {
            config.scan.exact_min_lines = Some(exact);
        }
        if let Some(similar) = self.similar_min_lines {
            config.scan.similar_min_lines = Some(similar);
        }
        if let Some(stmts) = self.exact_min_stmts {
            config.scan.exact_min_stmts = stmts;
        }
        if let Some(exact) = self.exact_min_fragment_lines {
            config.containment.exact_min_fragment_lines = Some(exact);
        }
        if let Some(similar) = self.similar_min_fragment_lines {
            config.containment.similar_min_fragment_lines = Some(similar);
        }
        if let Some(th) = self.threshold {
            config.scan.threshold = th;
        }
        if self.tests_only {
            config.scan.scope_to_tests();
        }
        // The flag can only turn containment on; leaving it off defers to config.
        if self.containment {
            config.containment.enabled = true;
        }

        // Checked once, here: this is the first point where file and CLI settings
        // have been merged, so it is the first point where a conflict between them
        // is real rather than provisional.
        config.check()?;

        Ok((config, scan_path))
    }

    /// Resolve the focus file list from the three mutually-exclusive CLI options.
    ///
    /// Returns `None` when the user supplied no focus flag (= scan everything,
    /// no filtering). Returns `Some(vec)` otherwise — an empty vec is a valid
    /// "no files changed" signal and suppresses all pairs.
    fn focus_files(&self) -> anyhow::Result<Option<Vec<PathBuf>>> {
        if self.focus_args {
            return Ok(Some(self.positionals.clone()));
        }
        if let Some(source) = self.files_from.as_deref() {
            let lines = read_file_list(source)
                .with_context(|| format!("failed to read file list from {}", source.display()))?;
            return Ok(Some(lines));
        }
        if !self.files.is_empty() {
            return Ok(Some(self.files.clone()));
        }
        Ok(None)
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

    /// Show a condensed file-centric overview of clone detection results
    Overview {
        #[command(flatten)]
        common: CommonOpts,
    },

    /// Show statistics about scan findings
    Stats {
        #[command(flatten)]
        common: CommonOpts,
    },

    /// Print setup, triage or tune instructions for this repository
    ///
    /// With no topic, prints `setup` when biston is not configured in the current
    /// directory and `triage` when it is; `tune` is never auto-selected. Detection
    /// looks at the current directory only and never walks up, so run this at the
    /// repository root.
    Guide {
        /// Which instructions to print. Omit to let biston choose.
        #[arg(value_enum)]
        topic: Option<Topic>,
    },

    /// Deprecated alias for `biston guide tune`
    #[command(hide = true)]
    Usage,

    /// Print a shell completion script to stdout
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
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

/// Map a finished report onto the exit code a check aggregator reads.
///
/// Containments count: they are findings the report printed, and a gate that
/// passed on them would be reporting a clean tree that is not clean.
fn findings_exit_code(report: &biston::report::CloneReport) -> ExitCode {
    if report.pairs.is_empty() && report.containments.is_empty() {
        EXIT_CLEAN
    } else {
        EXIT_FINDINGS
    }
}

/// Resolve the guide topic and render it.
///
/// Auto-selection reads the current directory rather than the scan root: the
/// question is "is biston set up where I am standing", and the guide tells its
/// reader to stand at the repository root.
fn guide_output(topic: Option<Topic>) -> String {
    if let Some(topic) = topic {
        guide::render(topic, guide::Selection::Explicit)
    } else {
        let source = guide::detect(std::path::Path::new("."));
        guide::render(guide::auto_topic(source), guide::Selection::Auto(source))
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            // Same shape anyhow's own `Termination` prints, so the error text a
            // user already knows is unchanged; only the code moves, from 1 to 2.
            eprintln!("Error: {err:?}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

/// Everything `main` does, minus the exit-code mapping.
fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    // Diagnostics go to stderr, where the default subscriber does not put them. The
    // report is stdout, and a warning printed into it — an ignored config alias, an
    // unparseable file — makes `--format json` unparseable for whatever consumes it.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(
            |_| tracing_subscriber::EnvFilter::new(verbosity_filter(cli.verbose, cli.quiet)),
        ))
        .init();

    let color_enabled = cli.color.resolve();

    match cli.command {
        Commands::Scan { common, suggest } => {
            let (mut config, scan_path) = common.resolve()?;
            if suggest {
                config.suggest.enabled = true;
            }

            if config.output.format == OutputFormat::Text {
                config.output.color = color_enabled;
            }

            let focus_files = common.focus_files()?;
            let report = biston::scan_focused(&scan_path, &config, focus_files.as_deref())?;

            let output = match config.output.format {
                OutputFormat::Text => report::format_text(&report, &config.output),
                OutputFormat::Json => report::format_json(&report, &config.output)?,
                OutputFormat::Sarif => report::format_sarif(&report, &config.output)?,
            };

            print!("{output}");

            Ok(findings_exit_code(&report))
        }
        Commands::Overview { common } => {
            let (mut config, scan_path) = common.resolve()?;

            if config.output.format == OutputFormat::Text {
                config.output.color = color_enabled;
            }

            let focus_files = common.focus_files()?;
            let report = biston::scan_focused(&scan_path, &config, focus_files.as_deref())?;
            let overviews = overview::compute_overview(&report);

            let output = match config.output.format {
                OutputFormat::Json => overview::format_overview_json(&overviews, &report)?,
                OutputFormat::Text | OutputFormat::Sarif => {
                    overview::format_overview_text(&overviews, &report, &config.output)
                }
            };

            print!("{output}");

            Ok(EXIT_CLEAN)
        }
        Commands::Stats { common } => {
            let (config, scan_path) = common.resolve()?;

            let focus_files = common.focus_files()?;
            let report = biston::scan_focused(&scan_path, &config, focus_files.as_deref())?;
            let scan_stats = stats::compute_stats(&report);

            let output = match config.output.format {
                OutputFormat::Json => stats::format_stats_json(&scan_stats)?,
                OutputFormat::Text | OutputFormat::Sarif => stats::format_stats_text(&scan_stats),
            };

            print!("{output}");

            Ok(findings_exit_code(&report))
        }
        Commands::Guide { topic } => {
            print!("{}", guide_output(topic));
            Ok(EXIT_CLEAN)
        }
        Commands::Usage => {
            tracing::warn!(
                "`biston usage` is deprecated and will be removed in the next minor release; \
                 run `biston guide tune` instead"
            );
            print!("{}", guide::render(Topic::Tune, guide::Selection::Explicit));
            Ok(EXIT_CLEAN)
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(EXIT_CLEAN)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Topic};
    use clap::CommandFactory;

    /// Every biston command the guides show must be one the CLI actually accepts.
    ///
    /// This lives here rather than in `src/guide.rs` because `Cli` is defined in
    /// the binary: the check is only worth anything against the real `Command`,
    /// aliases, conflicts and all. The extraction it relies on is tested in
    /// `guide::tests::extraction_finds_every_command_the_guides_show`, so an empty
    /// list cannot pass as "everything valid".
    #[test]
    fn every_command_shown_in_a_guide_parses() {
        let mut checked = 0_usize;
        for topic in Topic::ALL {
            for argv in biston::guide::embedded_invocations(topic) {
                checked += 1;
                let parsed = Cli::command().try_get_matches_from(&argv);
                assert!(
                    parsed.is_ok(),
                    "guide `{}` shows `{}`, which the CLI rejects: {}",
                    topic.name(),
                    argv.join(" "),
                    parsed.err().map_or_else(String::new, |e| e.to_string()),
                );
            }
        }
        assert!(checked >= 6, "expected several commands across the guides, found {checked}");
    }

    /// Guard the guard: an invocation the CLI would reject must fail this check.
    #[test]
    fn an_unknown_flag_would_be_caught() {
        let argv = ["biston", "scan", "--not-a-flag"];
        assert!(
            Cli::command().try_get_matches_from(argv).is_err(),
            "the command check would pass anything if clap accepted unknown flags",
        );
    }

    /// `usage` still runs, and stays out of `--help`.
    #[test]
    fn usage_is_hidden_but_still_parses() {
        assert!(Cli::command().try_get_matches_from(["biston", "usage"]).is_ok());
        let usage = Cli::command()
            .get_subcommands()
            .find(|c| c.get_name() == "usage")
            .map(clap::Command::is_hide_set);
        assert_eq!(usage, Some(true), "`usage` is deprecated and should not be advertised");
    }

    /// The auto-selection rule has to be discoverable from `--help`, since an
    /// agent that ran `biston guide` with no topic needs to know why it got what
    /// it got before it trusts it.
    #[test]
    fn guide_help_states_the_auto_selection_rule() {
        let help = Cli::command()
            .get_subcommands()
            .find(|c| c.get_name() == "guide")
            .and_then(clap::Command::get_long_about)
            .map(std::string::ToString::to_string)
            .unwrap_or_default();
        assert!(help.contains("setup"), "guide --help should name the setup topic: {help}");
        assert!(help.contains("triage"), "guide --help should name the triage topic: {help}");
        assert!(
            help.contains("never auto-selected"),
            "guide --help should say tune is never auto-selected: {help}",
        );
        assert!(
            help.contains("repository root"),
            "guide --help should say where to run it: {help}",
        );
    }

    #[test]
    fn guide_topics_parse_as_values() {
        for topic in ["setup", "triage", "tune"] {
            assert!(
                Cli::command().try_get_matches_from(["biston", "guide", topic]).is_ok(),
                "`biston guide {topic}` should parse",
            );
        }
        assert!(
            Cli::command().try_get_matches_from(["biston", "guide", "how"]).is_err(),
            "an unknown topic should be rejected rather than silently defaulted",
        );
    }
}
