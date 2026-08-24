use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub scan: ScanConfig,
    pub normalization: NormalizationConfig,
    pub output: OutputConfig,
    pub suggest: SuggestConfig,
    pub suppress: SuppressConfig,
    pub containment: ContainmentConfig,
}

/// Detection of one function already implementing the leading or trailing run of
/// another's body.
///
/// Off by default. When disabled, no fragment work is performed at all — the cost
/// is structurally zero rather than computed-and-discarded.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ContainmentConfig {
    /// Whether containment detection runs.
    pub enabled: bool,
    /// Retained alias for both fragment floors below.
    ///
    /// Set on its own it still means what it always meant — one floor for every
    /// accepted run. It is only an alias now because the floor a run must clear
    /// depends on how strong the match is; see [`Self::exact_min_fragment_lines`].
    pub min_fragment_lines: Option<usize>,
    /// Executable lines a run must span to be reported on the strength of an
    /// *exact* match.
    ///
    /// Lower than [`Self::similar_min_fragment_lines`] on purpose: an exact
    /// fingerprint match is strong evidence, so less of it is needed. Still
    /// deliberately higher than the whole-function floors — a fragment carries less
    /// context than a function, and a short shared run is stock boilerplate
    /// (`with open(...)`, read, `json.loads`) that "extract this" is no advice about.
    ///
    /// Defaults to `min_fragment_lines` when that is set, otherwise to
    /// [`Self::DEFAULT_EXACT_MIN_FRAGMENT_LINES`].
    pub exact_min_fragment_lines: Option<usize>,
    /// Executable lines a run must span to be reported on the strength of a
    /// *fuzzy* match.
    ///
    /// The containment coefficient over a handful of subtrees is a coarse, jumpy
    /// statistic, so a fuzzy match has to bring more evidence with it.
    ///
    /// Defaults to `min_fragment_lines` when that is set, otherwise to
    /// [`Self::DEFAULT_SIMILAR_MIN_FRAGMENT_LINES`].
    pub similar_min_fragment_lines: Option<usize>,
    /// The contained function must be at least this fraction of the container.
    ///
    /// Below it, what was found is a detail of a much larger function rather than an
    /// abstraction waiting to be named.
    pub min_ratio: f64,
    /// Minimum containment coefficient, `|A ∩ F| / min(|A|, |F|)`.
    ///
    /// Separate from — and stricter than — `scan.threshold`, which scores the
    /// symmetric relation with Jaccard.
    pub threshold: f64,
    /// Largest tolerated size ratio between the contained function and the matched run.
    ///
    /// The containment coefficient alone cannot exclude *interior* containment: if the
    /// run strictly contains the function, `min(|A|,|F|) == |A|` and the coefficient is
    /// 1.0 however much extra the run carries. Requiring the two to be comparable in
    /// size is what keeps a match anchored to the run's boundary.
    pub size_balance: f64,
    /// Largest fraction of the container's statements a run may span.
    ///
    /// A run covering nearly the whole body is the whole function again, which is the
    /// symmetric detector's job.
    pub max_run_fraction: f64,
    /// Cap on candidate-generating probes per function, across both roles.
    pub max_probes_per_function: usize,
}

impl Default for ContainmentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_fragment_lines: None,
            exact_min_fragment_lines: None,
            similar_min_fragment_lines: None,
            min_ratio: 0.30,
            threshold: 0.85,
            size_balance: 1.25,
            max_run_fraction: 0.85,
            max_probes_per_function: 12,
        }
    }
}

impl ContainmentConfig {
    /// Fragment floor for the exact tier when nothing is configured.
    pub const DEFAULT_EXACT_MIN_FRAGMENT_LINES: usize = 10;
    /// Fragment floor for the similar tier when nothing is configured.
    pub const DEFAULT_SIMILAR_MIN_FRAGMENT_LINES: usize = 15;

    /// Executable lines an exactly-matched run must span.
    #[must_use]
    pub fn exact_fragment_floor(&self) -> usize {
        self.exact_min_fragment_lines
            .or(self.min_fragment_lines)
            .unwrap_or(Self::DEFAULT_EXACT_MIN_FRAGMENT_LINES)
    }

    /// Executable lines a fuzzily-matched run must span.
    #[must_use]
    pub fn similar_fragment_floor(&self) -> usize {
        self.similar_min_fragment_lines
            .or(self.min_fragment_lines)
            .unwrap_or(Self::DEFAULT_SIMILAR_MIN_FRAGMENT_LINES)
    }

    /// Shortest run either tier could accept.
    ///
    /// Runs below it are never evaluated, which is what keeps the tier gates from
    /// being a second filter in front of an older, stricter one.
    #[must_use]
    pub fn candidate_fragment_floor(&self) -> usize {
        self.exact_fragment_floor().min(self.similar_fragment_floor())
    }

    /// The minimum tolerated `min(|A|,|F|) / max(|A|,|F|)`.
    ///
    /// Returns 0.0 for a non-positive `size_balance`, which disables the guard rather
    /// than dividing by zero.
    #[must_use]
    pub fn size_balance_floor(&self) -> f64 {
        if self.size_balance > 0.0 {
            1.0 / self.size_balance
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct SuppressConfig {
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SuggestConfig {
    /// Whether suggestion generation is enabled.
    pub enabled: bool,
    /// Minimum template coverage score to suggest (0.0 - 1.0).
    pub min_quality: f64,
    /// Maximum number of holes before suppressing.
    pub max_holes: usize,
    /// Whether to render templates as Python source.
    pub render_python: bool,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self { enabled: false, min_quality: 0.6, max_holes: 5, render_python: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// Retained alias for both line floors below.
    ///
    /// Supported indefinitely: a config that sets only `min_lines` keeps meaning
    /// what it always meant — one floor, applied to every reported pair. It became
    /// an alias because a single floor cannot say what the tiers say, namely that a
    /// short *exact* duplicate is worth reporting and a short *fuzzy* one is not.
    pub min_lines: Option<usize>,
    /// Executable lines the shorter function must have for an *exact* match to be
    /// reported.
    ///
    /// An exact match of the normalized tree is strong evidence on its own, so this
    /// floor is low. It is not the only exact-tier guard: see [`Self::exact_min_stmts`].
    ///
    /// Defaults to `min_lines` when that is set, otherwise to
    /// [`Self::DEFAULT_EXACT_MIN_LINES`].
    pub exact_min_lines: Option<usize>,
    /// Executable lines the shorter function must have for a *fuzzy* match to be
    /// reported.
    ///
    /// Jaccard over a handful of subtrees is a coarse statistic that jumps on small
    /// edits, so a fuzzy match needs a bigger evidence base than an exact one.
    ///
    /// Defaults to `min_lines` when that is set, otherwise to
    /// [`Self::DEFAULT_SIMILAR_MIN_LINES`].
    pub similar_min_lines: Option<usize>,
    /// Statements a body must have for an *exact* match to be reported.
    ///
    /// Applies to the exact tier only. After normalization — locals anonymized,
    /// comments, docstrings and annotations gone — short bodies collide on idiom
    /// rather than on content: delegation wrappers, guard-return pairs,
    /// `try: ... except: pass`. They hash identical because the idiom is identical,
    /// and there is nothing in them to extract.
    pub exact_min_stmts: usize,
    pub threshold: f64,
    pub exclude: Vec<String>,
    pub include: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NormalizationConfig {
    pub anonymize_locals: bool,
    pub anonymize_literals: bool,
    pub strip_decorators: bool,
    pub strip_type_annotations: bool,
    pub sort_commutative: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Sarif,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    pub format: OutputFormat,
    pub group_overlapping: bool,
    pub max_results: usize,
    pub show_source: bool,
    pub context_lines: usize,
    /// Whether to emit ANSI color codes (set at runtime based on TTY detection).
    #[serde(skip)]
    pub color: bool,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            min_lines: None,
            exact_min_lines: None,
            similar_min_lines: None,
            exact_min_stmts: 3,
            threshold: 0.85,
            exclude: vec![
                "tests/**".to_owned(),
                "**/conftest.py".to_owned(),
                "migrations/**".to_owned(),
            ],
            include: vec!["**/*.py".to_owned()],
        }
    }
}

impl ScanConfig {
    /// Line floor for the exact tier when nothing is configured.
    pub const DEFAULT_EXACT_MIN_LINES: usize = 5;
    /// Line floor for the similar tier when nothing is configured.
    pub const DEFAULT_SIMILAR_MIN_LINES: usize = 9;

    /// Executable lines the shorter function of an exact match must have.
    #[must_use]
    pub fn exact_line_floor(&self) -> usize {
        self.exact_min_lines.or(self.min_lines).unwrap_or(Self::DEFAULT_EXACT_MIN_LINES)
    }

    /// Executable lines the shorter function of a fuzzy match must have.
    #[must_use]
    pub fn similar_line_floor(&self) -> usize {
        self.similar_min_lines.or(self.min_lines).unwrap_or(Self::DEFAULT_SIMILAR_MIN_LINES)
    }

    /// Shortest function either tier could accept, and so the extraction floor.
    ///
    /// Extraction keeps everything a tier might later accept and nothing smaller.
    /// Deciding reportability here instead would make the exact tier unable to see
    /// the short functions it exists to report.
    #[must_use]
    pub fn extraction_line_floor(&self) -> usize {
        self.exact_line_floor().min(self.similar_line_floor())
    }

    /// Common glob patterns identifying Python test files.
    ///
    /// Matches pytest's default discovery conventions (`test_*.py`, `*_test.py`,
    /// `conftest.py`) plus anything under any `tests/` directory (including
    /// nested monorepo layouts like `backend/tests/helpers.py`).
    pub const TEST_FILE_PATTERNS: &'static [&'static str] =
        &["**/test_*.py", "**/*_test.py", "**/conftest.py", "tests/**/*.py", "**/tests/**/*.py"];

    /// Narrow the scan scope to test files only.
    ///
    /// Replaces `include` with [`Self::TEST_FILE_PATTERNS`] and clears `exclude`
    /// so that the default test-suppressing excludes no longer apply. Other
    /// scan knobs (the size floors, `threshold`) are left untouched.
    pub fn scope_to_tests(&mut self) {
        self.include = Self::TEST_FILE_PATTERNS.iter().copied().map(String::from).collect();
        self.exclude = Vec::new();
    }
}

impl Default for NormalizationConfig {
    fn default() -> Self {
        Self {
            anonymize_locals: true,
            anonymize_literals: false,
            strip_decorators: true,
            strip_type_annotations: true,
            sort_commutative: false,
        }
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Text,
            group_overlapping: true,
            max_results: 50,
            show_source: true,
            context_lines: 3,
            color: false,
        }
    }
}

/// Intermediate representation for pyproject.toml parsing.
#[derive(Debug, Deserialize)]
struct PyProjectToml {
    tool: Option<PyProjectTool>,
}

#[derive(Debug, Deserialize)]
struct PyProjectTool {
    biston: Option<Config>,
}

impl Config {
    /// Load configuration from the given directory.
    ///
    /// Precedence: `biston.toml` > `pyproject.toml [tool.biston]` > defaults.
    ///
    /// Contradictory size floors are rejected here rather than silently reordered.
    /// Alias warnings are not emitted here: the CLI merges its own overrides on top
    /// of what was loaded, and a warning about the merged result is the only one
    /// worth printing — see [`Self::check`].
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let config = Self::read(dir)?;
        config.validate()?;
        Ok(config)
    }

    /// Read configuration without validating it.
    fn read(dir: &Path) -> anyhow::Result<Self> {
        let biston_toml = dir.join("biston.toml");
        if biston_toml.exists() {
            let contents =
                std::fs::read_to_string(&biston_toml).context("failed to read biston.toml")?;
            return toml::from_str(&contents).context("failed to parse biston.toml");
        }

        let pyproject_toml = dir.join("pyproject.toml");
        if pyproject_toml.exists() {
            let contents = std::fs::read_to_string(&pyproject_toml)
                .context("failed to read pyproject.toml")?;
            let pyproject: PyProjectToml =
                toml::from_str(&contents).context("failed to parse pyproject.toml")?;
            if let Some(tool) = pyproject.tool {
                if let Some(config) = tool.biston {
                    return Ok(config);
                }
            }
        }

        Ok(Self::default())
    }

    /// Warn about shadowed aliases, then validate.
    ///
    /// Call this once, after every source of configuration has been merged — a
    /// warning about a conflict the CLI then resolves would be noise.
    pub fn check(&self) -> anyhow::Result<()> {
        warn_if_alias_shadowed(
            "scan",
            "min_lines",
            self.scan.min_lines,
            &[
                ("exact_min_lines", self.scan.exact_min_lines),
                ("similar_min_lines", self.scan.similar_min_lines),
            ],
        );
        warn_if_alias_shadowed(
            "containment",
            "min_fragment_lines",
            self.containment.min_fragment_lines,
            &[
                ("exact_min_fragment_lines", self.containment.exact_min_fragment_lines),
                ("similar_min_fragment_lines", self.containment.similar_min_fragment_lines),
            ],
        );
        self.validate()
    }

    /// Reject configurations no scan could honour.
    ///
    /// These are errors rather than warnings on purpose: an exact floor above the
    /// fuzzy one inverts the whole policy — the tier meant to catch *more* would
    /// catch less — and silently reordering the two would hide the mistake behind
    /// results that look plausible.
    pub fn validate(&self) -> anyhow::Result<()> {
        let scan = &self.scan;
        anyhow::ensure!(
            scan.exact_line_floor() <= scan.similar_line_floor(),
            "scan.exact_min_lines ({}) must not exceed scan.similar_min_lines ({}): an \
             exact match is stronger evidence than a fuzzy one, so it cannot ask for more",
            scan.exact_line_floor(),
            scan.similar_line_floor(),
        );
        anyhow::ensure!(
            scan.exact_line_floor() >= 1,
            "scan.exact_min_lines must be at least 1, got {}",
            scan.exact_line_floor(),
        );
        anyhow::ensure!(
            scan.exact_min_stmts >= 1,
            "scan.exact_min_stmts must be at least 1, got {}",
            scan.exact_min_stmts,
        );

        let containment = &self.containment;
        anyhow::ensure!(
            containment.exact_fragment_floor() <= containment.similar_fragment_floor(),
            "containment.exact_min_fragment_lines ({}) must not exceed \
             containment.similar_min_fragment_lines ({})",
            containment.exact_fragment_floor(),
            containment.similar_fragment_floor(),
        );
        anyhow::ensure!(
            containment.exact_fragment_floor() >= 1,
            "containment.exact_min_fragment_lines must be at least 1, got {}",
            containment.exact_fragment_floor(),
        );
        Ok(())
    }
}

/// Warn once when a retained alias is set alongside the keys that supersede it.
///
/// The new keys win. The warning names the alias and every key overriding it, so
/// the reader can delete one line and know exactly what changes.
fn warn_if_alias_shadowed(
    section: &str,
    alias: &str,
    alias_value: Option<usize>,
    superseding: &[(&str, Option<usize>)],
) {
    if alias_value.is_none() {
        return;
    }
    let set: Vec<&str> =
        superseding.iter().filter(|(_, value)| value.is_some()).map(|&(key, _)| key).collect();
    if set.is_empty() {
        return;
    }
    tracing::warn!(
        "{section}.{alias} is ignored: {} also set and take(s) precedence",
        set.iter().map(|key| format!("{section}.{key}")).collect::<Vec<_>>().join(" and "),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let config = Config::default();
        assert_eq!(config.scan.min_lines, None, "the alias is unset until a user sets it");
        assert_eq!(config.scan.exact_line_floor(), 5);
        assert_eq!(config.scan.similar_line_floor(), 9);
        assert_eq!(config.scan.exact_min_stmts, 3);
        assert!((config.scan.threshold - 0.85).abs() < f64::EPSILON);
        assert_eq!(config.scan.include, vec!["**/*.py"]);
        assert!(config.normalization.anonymize_locals);
        assert!(!config.normalization.anonymize_literals);
        assert!(config.normalization.strip_decorators);
        assert!(config.normalization.strip_type_annotations);
        assert!(!config.normalization.sort_commutative);
        assert_eq!(config.output.format, OutputFormat::Text);
        assert!(config.output.group_overlapping);
        assert_eq!(config.output.max_results, 50);
        assert!(config.output.show_source);
        assert_eq!(config.output.context_lines, 3);
    }

    #[test]
    fn containment_defaults_split_the_fragment_floor_by_tier() {
        let config = ContainmentConfig::default();
        assert_eq!(config.min_fragment_lines, None);
        assert_eq!(config.exact_fragment_floor(), 10);
        assert_eq!(config.similar_fragment_floor(), 15);
        assert_eq!(config.candidate_fragment_floor(), 10);
    }

    // --- Tier floors and the retained `min_lines` alias ---

    #[test]
    fn min_lines_alone_sets_both_line_floors() {
        let config: Config = toml::from_str("[scan]\nmin_lines = 12\n").expect("should parse");
        assert_eq!(config.scan.exact_line_floor(), 12);
        assert_eq!(config.scan.similar_line_floor(), 12);
        assert_eq!(config.scan.extraction_line_floor(), 12);
    }

    #[test]
    fn tier_floors_win_over_min_lines() {
        let toml_str = "[scan]\nmin_lines = 12\nexact_min_lines = 4\nsimilar_min_lines = 20\n";
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.scan.exact_line_floor(), 4);
        assert_eq!(config.scan.similar_line_floor(), 20);
    }

    #[test]
    fn a_partially_set_tier_falls_back_to_min_lines_not_to_the_default() {
        let config: Config =
            toml::from_str("[scan]\nmin_lines = 12\nsimilar_min_lines = 20\n").expect("parse");
        assert_eq!(config.scan.exact_line_floor(), 12, "the alias still speaks for the other tier");
        assert_eq!(config.scan.similar_line_floor(), 20);
    }

    #[test]
    fn extraction_floor_is_the_lower_of_the_two_tiers() {
        let config: Config =
            toml::from_str("[scan]\nexact_min_lines = 3\nsimilar_min_lines = 30\n").expect("parse");
        assert_eq!(
            config.scan.extraction_line_floor(),
            3,
            "extraction must keep everything a tier could accept"
        );
    }

    #[test]
    fn min_fragment_lines_alone_sets_both_fragment_floors() {
        let config: Config =
            toml::from_str("[containment]\nmin_fragment_lines = 20\n").expect("parse");
        assert_eq!(config.containment.exact_fragment_floor(), 20);
        assert_eq!(config.containment.similar_fragment_floor(), 20);
    }

    #[test]
    fn fragment_tier_floors_win_over_min_fragment_lines() {
        let toml_str = "[containment]\nmin_fragment_lines = 20\nexact_min_fragment_lines = 8\n";
        let config: Config = toml::from_str(toml_str).expect("parse");
        assert_eq!(config.containment.exact_fragment_floor(), 8);
        assert_eq!(config.containment.similar_fragment_floor(), 20);
    }

    // --- Validation ---

    #[test]
    fn an_exact_floor_above_the_similar_floor_is_rejected() {
        let config: Config =
            toml::from_str("[scan]\nexact_min_lines = 12\nsimilar_min_lines = 9\n").expect("parse");
        let err = config.validate().expect_err("inverted floors must not be accepted");
        let message = err.to_string();
        assert!(message.contains("exact_min_lines"), "got: {message}");
        assert!(message.contains("similar_min_lines"), "got: {message}");
    }

    #[test]
    fn an_exact_fragment_floor_above_the_similar_one_is_rejected() {
        let toml_str =
            "[containment]\nexact_min_fragment_lines = 20\nsimilar_min_fragment_lines = 15\n";
        let config: Config = toml::from_str(toml_str).expect("parse");
        let err = config.validate().expect_err("inverted fragment floors must not be accepted");
        assert!(err.to_string().contains("exact_min_fragment_lines"), "got: {err}");
    }

    #[test]
    fn a_zero_line_floor_is_rejected() {
        let config: Config = toml::from_str("[scan]\nexact_min_lines = 0\n").expect("parse");
        let err = config.validate().expect_err("a floor of zero admits everything");
        assert!(err.to_string().contains("at least 1"), "got: {err}");
    }

    #[test]
    fn a_zero_statement_floor_is_rejected() {
        let config: Config = toml::from_str("[scan]\nexact_min_stmts = 0\n").expect("parse");
        let err = config.validate().expect_err("a statement floor of zero admits every idiom");
        assert!(err.to_string().contains("exact_min_stmts"), "got: {err}");
    }

    #[test]
    fn a_zero_fragment_floor_is_rejected() {
        let config: Config =
            toml::from_str("[containment]\nmin_fragment_lines = 0\n").expect("parse");
        let err = config.validate().expect_err("a fragment floor of zero admits everything");
        assert!(err.to_string().contains("at least 1"), "got: {err}");
    }

    #[test]
    fn equal_floors_are_accepted() {
        let config: Config =
            toml::from_str("[scan]\nexact_min_lines = 9\nsimilar_min_lines = 9\n").expect("parse");
        config.validate().expect("one floor for both tiers is a legitimate policy");
    }

    #[test]
    fn load_rejects_an_invalid_config_rather_than_scanning_with_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("biston.toml"),
            "[scan]\nexact_min_lines = 20\nsimilar_min_lines = 5\n",
        )
        .expect("write");
        let err = Config::load(dir.path()).expect_err("invalid config must not load");
        assert!(err.to_string().contains("exact_min_lines"), "got: {err}");
    }

    #[test]
    fn check_accepts_a_config_that_only_uses_the_alias() {
        let config: Config = toml::from_str("[scan]\nmin_lines = 10\n").expect("parse");
        config.check().expect("the alias on its own is not a conflict");
    }

    // --- Parsing ---

    #[test]
    fn partial_toml_fills_defaults() {
        let toml_str = r"
[scan]
min_lines = 5
";
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.scan.min_lines, Some(5));
        // Rest should be defaults
        assert!((config.scan.threshold - 0.85).abs() < f64::EPSILON);
        assert!(config.normalization.anonymize_locals);
        assert_eq!(config.output.max_results, 50);
    }

    #[test]
    fn full_toml_roundtrip() {
        let toml_str = r#"
[scan]
exact_min_lines = 6
similar_min_lines = 15
exact_min_stmts = 4
threshold = 0.8
exclude = ["vendor/"]
include = ["src/**/*.py"]

[normalization]
anonymize_locals = false
anonymize_literals = true
strip_decorators = false
strip_type_annotations = false
sort_commutative = true

[output]
format = "json"
group_overlapping = false
max_results = 100
show_source = false
context_lines = 5
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.scan.exact_line_floor(), 6);
        assert_eq!(config.scan.similar_line_floor(), 15);
        assert_eq!(config.scan.exact_min_stmts, 4);
        assert!((config.scan.threshold - 0.8).abs() < f64::EPSILON);
        assert_eq!(config.scan.exclude, vec!["vendor/"]);
        assert!(!config.normalization.anonymize_locals);
        assert!(config.normalization.anonymize_literals);
        assert_eq!(config.output.format, OutputFormat::Json);
        assert_eq!(config.output.max_results, 100);
    }

    #[test]
    fn load_biston_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("biston.toml"), "[scan]\nmin_lines = 20\n").expect("write");
        let config = Config::load(dir.path()).expect("load");
        assert_eq!(config.scan.min_lines, Some(20));
    }

    #[test]
    fn load_pyproject_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.biston.scan]\nmin_lines = 25\n")
            .expect("write");
        let config = Config::load(dir.path()).expect("load");
        assert_eq!(config.scan.min_lines, Some(25));
    }

    #[test]
    fn load_defaults_when_no_config_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Config::load(dir.path()).expect("load");
        assert_eq!(config.scan.min_lines, None);
        assert_eq!(config.scan.exact_line_floor(), 5);
    }

    #[test]
    fn suggest_section_parses_from_toml() {
        let toml_str = r"
[suggest]
min_quality = 0.8
max_holes = 3
render_python = false
";
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert!((config.suggest.min_quality - 0.8).abs() < f64::EPSILON);
        assert_eq!(config.suggest.max_holes, 3);
        assert!(!config.suggest.render_python);
    }

    #[test]
    fn suggest_defaults_when_absent() {
        let toml_str = r"
[scan]
min_lines = 5
";
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert!((config.suggest.min_quality - 0.6).abs() < f64::EPSILON);
        assert_eq!(config.suggest.max_holes, 5);
        assert!(config.suggest.render_python);
    }

    #[test]
    fn suppress_section_parses_from_toml() {
        let toml_str = r#"
[suppress]
files = ["generated/**", "vendor/**"]
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.suppress.files, vec!["generated/**", "vendor/**"]);
    }

    #[test]
    fn suppress_defaults_when_absent() {
        let toml_str = r"
[scan]
min_lines = 5
";
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert!(config.suppress.files.is_empty());
    }

    #[test]
    fn scope_to_tests_replaces_include_with_test_patterns() {
        let mut config = ScanConfig::default();
        config.scope_to_tests();
        assert_eq!(config.include, ScanConfig::TEST_FILE_PATTERNS);
    }

    #[test]
    fn scope_to_tests_clears_exclude_list() {
        let mut config = ScanConfig::default();
        assert!(!config.exclude.is_empty(), "default exclude should be non-empty");
        config.scope_to_tests();
        assert!(config.exclude.is_empty());
    }

    #[test]
    fn scope_to_tests_preserves_threshold_and_size_floors() {
        let mut config = ScanConfig {
            exact_min_lines: Some(7),
            similar_min_lines: Some(25),
            threshold: 0.9,
            ..ScanConfig::default()
        };
        config.scope_to_tests();
        assert_eq!(config.exact_line_floor(), 7);
        assert_eq!(config.similar_line_floor(), 25);
        assert!((config.threshold - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn biston_toml_takes_precedence_over_pyproject() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("biston.toml"), "[scan]\nmin_lines = 20\n").expect("write");
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.biston.scan]\nmin_lines = 25\n")
            .expect("write");
        let config = Config::load(dir.path()).expect("load");
        assert_eq!(config.scan.min_lines, Some(20));
    }
}
