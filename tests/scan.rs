use std::path::PathBuf;

use biston::config::Config;

fn fixtures_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn config_for_file(filename: &str) -> Config {
    let mut config = Config::default();
    config.scan.include = vec![filename.to_owned()];
    config.scan.exclude = vec![];
    config
}

fn config_for_file_with_suggest(filename: &str) -> Config {
    let mut config = config_for_file(filename);
    config.suggest.enabled = true;
    config
}

#[test]
fn detects_simple_clones() {
    let config = config_for_file("simple_clones.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(report.functions.len(), 2);
    assert_eq!(report.pairs.len(), 1);
    assert!(
        (report.pairs[0].similarity - 1.0).abs() < f64::EPSILON,
        "expected exact match, got {}",
        report.pairs[0].similarity
    );
}

#[test]
fn detects_near_miss() {
    let config = config_for_file("near_miss.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(report.functions.len(), 2);
    assert_eq!(report.pairs.len(), 1, "expected one near-miss pair");
    assert!(
        report.pairs[0].similarity >= 0.7,
        "expected similarity >= 0.7, got {}",
        report.pairs[0].similarity
    );
    assert!(
        report.pairs[0].similarity < 1.0,
        "expected similarity < 1.0, got {}",
        report.pairs[0].similarity
    );
}

#[test]
fn no_false_positives() {
    let config = config_for_file("no_clones.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(report.functions.len(), 3);
    assert!(
        report.pairs.is_empty(),
        "expected no clone pairs from unrelated functions, got {}",
        report.pairs.len()
    );
}

#[test]
fn short_functions_filtered() {
    let config = config_for_file("short_functions.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(
        report.functions.is_empty(),
        "expected no functions extracted (all below min_lines), got {}",
        report.functions.len()
    );
}

#[test]
fn normalized_trees_parallel_to_functions() {
    let config = config_for_file("simple_clones.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(
        report.normalized.len(),
        report.functions.len(),
        "normalized vec must be parallel to functions vec"
    );
}

#[test]
fn suggest_produces_suggestions_for_clones() {
    let config = config_for_file_with_suggest("suggest_clones.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(
        !report.suggestions.is_empty(),
        "expected at least one suggestion for suggest_clones.py"
    );
    // Verify quality fields are populated
    let sug = &report.suggestions[0];
    assert!(sug.quality.score > 0.0, "quality score should be positive");
    assert!(sug.rendered.is_some(), "rendered template should be present");
}

#[test]
fn suggest_disabled_produces_no_suggestions() {
    let config = config_for_file("suggest_clones.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(report.suggestions.is_empty(), "expected no suggestions when suggest is disabled");
}

#[test]
fn suggest_on_simple_clones_produces_suggestions() {
    let config = config_for_file_with_suggest("simple_clones.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(!report.suggestions.is_empty(), "simple_clones.py should produce suggestions");
}
