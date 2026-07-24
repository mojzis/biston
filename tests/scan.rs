use std::path::PathBuf;

use biston::config::{Config, SuppressConfig};

mod common;

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
fn docstring_only_functions_are_not_reported_as_clones() {
    // All three bodies normalize identically (a childless `docstring` node), so
    // exact root-hash matching would pair every one of them at similarity 1.0.
    // There is no logic in them to extract, so none of it is worth reporting.
    let config = config_for_file("docstring_only.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(report.functions.len(), 3, "fixture should yield three functions");
    assert!(
        report.pairs.is_empty(),
        "expected no pairs, got {:?}",
        report
            .pairs
            .iter()
            .map(|p| (
                report.functions[p.left].name.as_str(),
                report.functions[p.right].name.as_str(),
                p.similarity
            ))
            .collect::<Vec<_>>()
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

#[test]
fn test_inline_suppress_excludes_function() {
    let config = config_for_file("suppressed_inline.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(
        report.functions.len(),
        1,
        "expected 1 function after inline suppression, got {}",
        report.functions.len()
    );
    assert!(report.pairs.is_empty(), "expected 0 pairs with only 1 function remaining");
    assert_eq!(report.suppression_stats.inline_functions, 1);
}

#[test]
fn test_file_level_suppress_excludes_all() {
    let config = config_for_file("suppressed_file.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(
        report.functions.is_empty(),
        "expected 0 functions after file-level suppression, got {}",
        report.functions.len()
    );
    assert_eq!(report.suppression_stats.file_comments, 1);
}

#[test]
fn test_config_glob_suppress() {
    let mut config = config_for_file("suppressed_inline.py");
    config.suppress = SuppressConfig { files: vec!["suppressed_inline.py".to_owned()] };
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(
        report.functions.is_empty(),
        "expected 0 functions after config glob suppression, got {}",
        report.functions.len()
    );
    assert_eq!(report.suppression_stats.config_files, 1);
}

// --- Focus-file (commit-hook) scan tests ---

#[test]
fn scan_focused_without_focus_matches_scan() {
    let dir = common::multi_file_dir();
    let config = Config::default();
    let focused = biston::scan_focused(dir.path(), &config, None).expect("scan_focused");
    let full = biston::scan(dir.path(), &config).expect("scan");
    assert_eq!(focused.functions.len(), full.functions.len());
    assert_eq!(focused.pairs.len(), full.pairs.len());
}

#[test]
fn scan_focused_restricts_pairs_to_focus_files() {
    let dir = common::multi_file_dir();
    let config = Config::default();

    // Baseline: all four files produce both pairs.
    let full = biston::scan(dir.path(), &config).expect("scan");
    assert_eq!(full.pairs.len(), 2, "expected a-b and c-d pairs without focus");

    // Focus on a.py only: a-b pair is kept (a is in focus), c-d is dropped.
    let focus = vec![dir.path().join("a.py")];
    let report = biston::scan_focused(dir.path(), &config, Some(&focus)).expect("scan_focused");

    // Full repo is still processed so cross-file clones are still found.
    assert_eq!(report.functions.len(), 4, "all functions still extracted");
    assert_eq!(report.pairs.len(), 1, "only the a-b pair should be emitted");

    let pair = &report.pairs[0];
    let left = &report.functions[pair.left].file_path;
    let right = &report.functions[pair.right].file_path;
    let a = dir.path().join("a.py");
    let b = dir.path().join("b.py");
    assert!(
        (left == &a && right == &b) || (left == &b && right == &a),
        "pair should involve a.py and b.py, got {left:?} + {right:?}"
    );
}

#[test]
fn scan_focused_empty_focus_emits_no_pairs() {
    // An explicitly empty focus set means "nothing changed" — no pairs should
    // be emitted, but all files are still parsed so stats stay meaningful.
    let dir = common::multi_file_dir();
    let config = Config::default();
    let focus: Vec<std::path::PathBuf> = vec![];
    let report = biston::scan_focused(dir.path(), &config, Some(&focus)).expect("scan_focused");
    assert_eq!(report.functions.len(), 4);
    assert!(report.pairs.is_empty());
}

#[test]
fn scan_focused_ignores_unknown_focus_path() {
    // A mix of valid + invalid paths should resolve the valid ones and
    // silently skip the invalid ones — not fail the whole scan.
    let dir = common::multi_file_dir();
    let config = Config::default();
    let focus = vec![dir.path().join("does_not_exist.py"), dir.path().join("a.py")];
    let report = biston::scan_focused(dir.path(), &config, Some(&focus)).expect("scan_focused");
    assert_eq!(report.functions.len(), 4);
    // The a.py focus path still resolves, so the a-b pair is emitted. The
    // missing focus path shouldn't accidentally match anything.
    assert_eq!(report.pairs.len(), 1, "valid focus path still matches");
    let pair = &report.pairs[0];
    let left = &report.functions[pair.left].file_path;
    let right = &report.functions[pair.right].file_path;
    let a = dir.path().join("a.py");
    let b = dir.path().join("b.py");
    assert!(
        (left == &a && right == &b) || (left == &b && right == &a),
        "pair should involve a.py and b.py, got {left:?} + {right:?}"
    );
}
