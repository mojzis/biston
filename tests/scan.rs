#![allow(clippy::expect_used, reason = "integration-test helpers treat setup failures as fatal")]

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

fn config_for_file_with_containment(filename: &str) -> Config {
    let mut config = config_for_file(filename);
    config.containment.enabled = true;
    config
}

/// `(contained name, container name, role, 1-indexed container span)` for each finding.
fn describe_containments(
    report: &biston::report::CloneReport,
) -> Vec<(String, String, &str, usize, usize)> {
    report
        .containments
        .iter()
        .map(|c| {
            (
                report.functions[c.contained].name.clone(),
                report.functions[c.container].name.clone(),
                c.role.as_str(),
                c.start_line + 1,
                c.end_line + 1,
            )
        })
        .collect()
}

#[test]
fn containment_is_off_by_default() {
    let config = config_for_file("containment_append.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(
        report.containments.is_empty(),
        "containment must be opt-in, got {:?}",
        describe_containments(&report)
    );
}

#[test]
fn containment_finds_an_appended_container() {
    // B = A + C: A is the leading run of B's body.
    let config = config_for_file_with_containment("containment_append.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    let found = describe_containments(&report);
    assert_eq!(found.len(), 1, "expected exactly one finding, got {found:?}");

    let (inner, outer, role, start, end) = &found[0];
    assert_eq!(inner, "normalize_records");
    assert_eq!(outer, "normalize_and_index_records");
    assert_eq!(*role, "prefix");
    // The run is the first three statements of the container's body.
    assert_eq!((*start, *end), (25, 40), "container run span");
    assert!(report.containments[0].score >= 0.85, "score {}", report.containments[0].score);
    assert_eq!(report.containments[0].statement_count, 3);
}

#[test]
fn containment_span_skips_the_prose_it_no_longer_normalizes() {
    // The container carries a docstring and comments the contained function does not.
    // Normalization drops them, so `body_statement_spans` has to drop them as well:
    // if the two walks disagree the run is reported against the wrong lines, or the
    // finding is dropped with only a `tracing::warn!` behind it.
    let config = config_for_file_with_containment("containment_prose.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    let found = describe_containments(&report);
    assert_eq!(found.len(), 1, "expected exactly one finding, got {found:?}");

    let source = std::fs::read_to_string(fixtures_path().join("containment_prose.py"))
        .expect("read fixture");
    // 1-indexed line of the container's first and last shared statements. Derived
    // from the fixture rather than hard-coded, so the assertion says what it means.
    let line_of = |needle: &str| -> usize {
        source
            .lines()
            .enumerate()
            .filter(|(_, line)| line.trim_start().starts_with(needle))
            .map(|(index, _)| index + 1)
            .last()
            .unwrap_or_else(|| panic!("fixture must contain {needle}"))
    };
    let first_statement = line_of("cleaned = []");
    let last_statement = line_of("cleaned.sort(");

    let (inner, outer, role, start, end) = &found[0];
    assert_eq!(inner, "normalize_records");
    assert_eq!(outer, "normalize_and_index_records");
    assert_eq!(*role, "prefix");
    assert_eq!(
        (*start, *end),
        (first_statement, last_statement),
        "the run must span the container's executable statements, not its prose"
    );
    assert_eq!(report.containments[0].statement_count, 3, "docstring must not count as one");
}

#[test]
fn containment_finds_a_prepended_container() {
    // B = C + A: A is the *trailing* run of B's body. Every local in that run is
    // registered after C's, so this only works with run-relative renumbering.
    let config = config_for_file_with_containment("containment_prepend.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    let found = describe_containments(&report);
    assert_eq!(found.len(), 1, "expected exactly one finding, got {found:?}");

    let (inner, outer, role, start, end) = &found[0];
    assert_eq!(inner, "normalize_records");
    assert_eq!(outer, "load_then_normalize_records");
    assert_eq!(*role, "suffix");
    // `cleaned = []` through `return cleaned` — the same four statements that make
    // up the contained function's body, including its return.
    assert_eq!((*start, *end), (42, 58), "container run span");
    assert_eq!(report.containments[0].statement_count, 4);
    assert!(report.containments[0].score >= 0.85, "score {}", report.containments[0].score);
}

#[test]
fn containment_ignores_a_function_in_the_middle() {
    // Interior containment is out of scope for this phase. The run that contains the
    // shared statements also carries a substantial trailing block, so it is far
    // larger than the contained function and the size-balance guard rejects it.
    let config = config_for_file_with_containment("containment_middle.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(
        report.containments.is_empty(),
        "interior containment must not be reported, got {:?}",
        describe_containments(&report)
    );
}

#[test]
fn containment_ignores_a_lexically_nested_function() {
    // A nested `def` is extracted in its own right and is also a statement of its
    // parent, so it always matches a run of the parent. That is not duplication.
    let config = config_for_file_with_containment("containment_nested.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(report.functions.len(), 2, "fixture should yield the parent and the nested helper");
    assert!(
        report.containments.is_empty(),
        "a nested function is not contained duplication, got {:?}",
        describe_containments(&report)
    );
}

#[test]
fn containment_supersedes_the_symmetric_pair() {
    // The append fixture is *also* found by symmetric detection (Jaccard 0.71).
    // Reporting both says the same thing twice, and the directed form is the more
    // actionable one, so the symmetric pair must be suppressed.
    let plain = biston::scan(&fixtures_path(), &config_for_file("containment_append.py")).unwrap();
    assert_eq!(plain.pairs.len(), 1, "symmetric detection should find this pair on its own");

    let config = config_for_file_with_containment("containment_append.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(report.containments.len(), 1);
    assert!(report.pairs.is_empty(), "containment must supersede the symmetric pair");
}

#[test]
fn suggest_emits_nothing_for_a_containment_relation() {
    // An anti-unified template for a containment pair is an artefact of the lockstep
    // walk diverging at the run boundary — a run of holes. Emitting nothing is better.
    let mut config = config_for_file_with_containment("containment_append.py");
    config.suggest.enabled = true;
    let report = biston::scan(&fixtures_path(), &config).unwrap();

    assert_eq!(report.containments.len(), 1, "fixture must produce a containment finding");
    assert!(
        report.suggestions.is_empty(),
        "no template may be produced for a containment relation, got {} suggestion(s)",
        report.suggestions.len()
    );
}

#[test]
fn suggest_still_works_for_ordinary_clones_with_containment_on() {
    // Guard against "fixed" by disabling suggestions wholesale.
    let mut config = config_for_file_with_containment("suggest_clones.py");
    config.suggest.enabled = true;
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(!report.suggestions.is_empty(), "ordinary clone pairs must still get templates");
}

/// Split the prepend fixture so the two sides live in separate files.
///
/// Returns `(dir, contained_path, container_path)`.
fn split_prepend_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let source = std::fs::read_to_string(fixtures_path().join("containment_prepend.py"))
        .expect("read fixture");
    let (inner, outer) =
        source.split_once("\n\ndef load_then_normalize_records").expect("split fixture");
    let dir = tempfile::tempdir().expect("tempdir");
    let contained_path = dir.path().join("contained.py");
    let container_path = dir.path().join("container.py");
    std::fs::write(&contained_path, inner).expect("write contained");
    std::fs::write(&container_path, format!("def load_then_normalize_records{outer}"))
        .expect("write container");
    (dir, contained_path, container_path)
}

#[test]
fn containment_is_found_across_files() {
    let (dir, _, _) = split_prepend_fixture();
    let mut config = Config::default();
    config.scan.exclude = vec![];
    config.containment.enabled = true;
    let report = biston::scan(dir.path(), &config).unwrap();
    assert_eq!(report.containments.len(), 1, "cross-file containment must be detected");
}

#[test]
fn containment_is_in_scope_when_only_the_container_is_focused() {
    let (dir, _, container_path) = split_prepend_fixture();
    let mut config = Config::default();
    config.scan.exclude = vec![];
    config.containment.enabled = true;
    let report = biston::scan_focused(dir.path(), &config, Some(&[container_path])).unwrap();
    assert_eq!(report.containments.len(), 1, "focusing the container keeps the finding");
}

#[test]
fn containment_is_in_scope_when_only_the_contained_is_focused() {
    let (dir, contained_path, _) = split_prepend_fixture();
    let mut config = Config::default();
    config.scan.exclude = vec![];
    config.containment.enabled = true;
    let report = biston::scan_focused(dir.path(), &config, Some(&[contained_path])).unwrap();
    assert_eq!(report.containments.len(), 1, "focusing the contained side keeps the finding");
}

#[test]
fn containment_is_dropped_when_neither_side_is_focused() {
    let (dir, _, _) = split_prepend_fixture();
    let unrelated = dir.path().join("unrelated.py");
    std::fs::write(&unrelated, "def unrelated():\n    return 1\n").unwrap();
    let mut config = Config::default();
    config.scan.exclude = vec![];
    config.containment.enabled = true;
    let report = biston::scan_focused(dir.path(), &config, Some(&[unrelated])).unwrap();
    assert!(report.containments.is_empty(), "neither side in focus must drop the finding");
}

#[test]
fn containment_ignores_a_trivial_shared_idiom() {
    // open/read/guard/parse boilerplate really is a leading run of the larger
    // function, but "extract this" is not useful advice about it. If this starts
    // being reported, `min_fragment_lines` is too low.
    let config = config_for_file_with_containment("containment_trivial.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert!(
        report.containments.is_empty(),
        "trivial idiom must be suppressed by the size floor, got {:?}",
        describe_containments(&report)
    );
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
fn comment_and_docstring_noise_still_matches_exactly() {
    // The two functions differ only in a docstring and comments. Normalization drops
    // both outright, so they must reach the *exact* path — a near-miss score here
    // means a placeholder node survived and perturbed the hashes.
    let config = config_for_file("comment_noise.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(report.functions.len(), 2, "fixture should yield two functions");
    assert_eq!(report.pairs.len(), 1, "expected one pair, got {:?}", report.pairs.len());
    assert!(
        (report.pairs[0].similarity - 1.0).abs() < f64::EPSILON,
        "expected an exact match, got {}",
        report.pairs[0].similarity
    );
}

#[test]
fn bodies_left_empty_by_stripping_are_scanned_without_panicking() {
    // Once the prose is dropped these bodies are empty blocks. That is a valid tree,
    // and it stays unreportable — exactly as a docstring-only body was before.
    let config = config_for_file("no_logic_bodies.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(report.functions.len(), 2, "fixture should yield two functions");
    assert!(
        report.pairs.is_empty(),
        "a body with no logic is not reportable, got {:?}",
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
    // Also a regression test for prose removal: `# biston: ignore` leaves no node
    // behind in the normalized tree any more, and suppression must not care —
    // it reads the directive from the source, not from what normalization kept.
    let config = config_for_file("suppressed_inline.py");
    let report = biston::scan(&fixtures_path(), &config).unwrap();
    assert_eq!(
        report.functions.len(),
        1,
        "expected 1 function after inline suppression, got {}",
        report.functions.len()
    );
    assert_eq!(report.functions[0].name, "process_data_alpha", "the wrong function survived");
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
