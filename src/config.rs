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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SuggestConfig {
    /// Minimum template coverage score to suggest (0.0 - 1.0).
    pub min_quality: f64,
    /// Maximum number of holes before suppressing.
    pub max_holes: usize,
    /// Whether to render templates as Python source.
    pub render_python: bool,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self { min_quality: 0.6, max_holes: 5, render_python: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    pub min_lines: usize,
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
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            min_lines: 10,
            threshold: 0.7,
            exclude: vec![
                "tests/".to_owned(),
                "**/conftest.py".to_owned(),
                "migrations/".to_owned(),
            ],
            include: vec!["**/*.py".to_owned()],
        }
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
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let config = Config::default();
        assert_eq!(config.scan.min_lines, 10);
        assert!((config.scan.threshold - 0.7).abs() < f64::EPSILON);
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
    fn partial_toml_fills_defaults() {
        let toml_str = r"
[scan]
min_lines = 5
";
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.scan.min_lines, 5);
        // Rest should be defaults
        assert!((config.scan.threshold - 0.7).abs() < f64::EPSILON);
        assert!(config.normalization.anonymize_locals);
        assert_eq!(config.output.max_results, 50);
    }

    #[test]
    fn full_toml_roundtrip() {
        let toml_str = r#"
[scan]
min_lines = 15
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
        assert_eq!(config.scan.min_lines, 15);
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
        assert_eq!(config.scan.min_lines, 20);
    }

    #[test]
    fn load_pyproject_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.biston.scan]\nmin_lines = 25\n")
            .expect("write");
        let config = Config::load(dir.path()).expect("load");
        assert_eq!(config.scan.min_lines, 25);
    }

    #[test]
    fn load_defaults_when_no_config_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Config::load(dir.path()).expect("load");
        assert_eq!(config.scan.min_lines, 10);
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
    fn biston_toml_takes_precedence_over_pyproject() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("biston.toml"), "[scan]\nmin_lines = 20\n").expect("write");
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.biston.scan]\nmin_lines = 25\n")
            .expect("write");
        let config = Config::load(dir.path()).expect("load");
        assert_eq!(config.scan.min_lines, 20);
    }
}
