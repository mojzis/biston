//! Acceptance-tier fixtures: what each tier reports, and what neither one does.
//!
//! Every negative case is written the same way: scan once with the floor that
//! rejects it lowered, to establish that the pair *is* found and how strong the
//! evidence is, then scan again with the shipped defaults and assert nothing is
//! reported. That way each test says which condition did the rejecting, rather than
//! passing for any reason at all — including a fixture that stopped parsing.

#![allow(clippy::expect_used, reason = "integration-test helpers treat setup failures as fatal")]

use std::path::PathBuf;

use biston::config::Config;
use biston::report::CloneReport;
use biston::tier::Tier;

fn fixtures_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Default configuration scoped to one tier fixture.
fn config_for(fixture: &str) -> Config {
    let mut config = Config::default();
    config.scan.include = vec![format!("tiers/{fixture}")];
    config.scan.exclude = vec![];
    config
}

fn scan(config: &Config) -> CloneReport {
    biston::scan(&fixtures_path(), config).expect("scan must succeed")
}

/// `(left name, right name, similarity, tier)` for each reported pair.
fn pairs_of(report: &CloneReport) -> Vec<(String, String, f64, Tier)> {
    report
        .pairs
        .iter()
        .map(|p| {
            (
                report.functions[p.left].name.clone(),
                report.functions[p.right].name.clone(),
                p.similarity,
                p.tier,
            )
        })
        .collect()
}

/// `(contained, container, score, tier)` for each containment finding.
fn containments_of(report: &CloneReport) -> Vec<(String, String, f64, Tier)> {
    report
        .containments
        .iter()
        .map(|c| {
            (
                report.functions[c.contained].name.clone(),
                report.functions[c.container].name.clone(),
                c.score,
                c.tier,
            )
        })
        .collect()
}

/// The one pair the fixture is expected to produce.
fn only_pair(report: &CloneReport) -> (String, String, f64, Tier) {
    let found = pairs_of(report);
    assert_eq!(found.len(), 1, "expected exactly one pair, got {found:?}");
    found.into_iter().next().expect("length checked above")
}

// --- Positive: the exact tier ---

#[test]
fn a_six_line_identical_pair_is_reported_as_exact() {
    // Six executable lines, four statements: short, and an exact structural match.
    // Under a single ten-line floor this was invisible.
    let report = scan(&config_for("exact_short.py"));
    let (left, right, similarity, tier) = only_pair(&report);
    assert_eq!((left.as_str(), right.as_str()), ("split_header", "split_frame"));
    assert!((similarity - 1.0).abs() < f64::EPSILON, "an exact match scores 1.0, got {similarity}");
    assert_eq!(tier, Tier::Exact);
    assert_eq!(report.functions[0].size.executable_lines, 6);
    assert_eq!(report.functions[0].size.executable_stmts, 4);
}

#[test]
fn a_six_line_pair_identical_after_renaming_is_reported_as_exact() {
    // Normalization anonymizes locals, so renaming leaves nothing to score: this is
    // an exact match, not a near one, and the tier says so.
    let report = scan(&config_for("exact_renamed.py"));
    let (_, _, similarity, tier) = only_pair(&report);
    assert!((similarity - 1.0).abs() < f64::EPSILON, "got {similarity}");
    assert_eq!(tier, Tier::Exact);
}

// --- Positive: the similar tier ---

#[test]
fn a_twelve_line_pair_above_the_threshold_is_reported_as_similar() {
    let config = config_for("similar_long.py");
    let report = scan(&config);
    let (left, right, similarity, tier) = only_pair(&report);
    assert_eq!((left.as_str(), right.as_str()), ("summarize_orders", "summarize_refunds"));
    assert!(
        similarity >= config.scan.threshold && similarity < 1.0,
        "fixture must be a fuzzy match above the threshold, got {similarity}"
    );
    assert_eq!(tier, Tier::Similar);
    assert_eq!(report.functions[0].size.executable_lines, 12);
}

// --- Positive: contained runs ---

#[test]
fn an_exactly_contained_eleven_line_run_is_reported_as_exact() {
    let mut config = config_for("contained_exact.py");
    config.containment.enabled = true;
    let report = scan(&config);

    let found = containments_of(&report);
    assert_eq!(found.len(), 1, "expected exactly one finding, got {found:?}");
    let (inner, outer, score, tier) = &found[0];
    assert_eq!(inner, "load_settings");
    assert_eq!(outer, "load_and_apply_settings");
    assert!((score - 1.0).abs() < f64::EPSILON, "an exactly shared run scores 1.0, got {score}");
    assert_eq!(*tier, Tier::Exact);

    // Eleven executable lines: over the exact tier's fragment floor, under the
    // fuzzy tier's, so this finding exists only because the match is exact.
    assert!(config.containment.exact_fragment_floor() <= 11);
    assert!(config.containment.similar_fragment_floor() > 11);
    let mut too_strict = config;
    too_strict.containment.exact_min_fragment_lines = Some(12);
    too_strict.containment.similar_min_fragment_lines = Some(40);
    let refused = containments_of(&scan(&too_strict));
    assert!(
        refused.is_empty(),
        "the shared run is eleven executable lines, so a floor of twelve rejects it: {refused:?}"
    );
}

// --- Negative: the exact tier's statement guard ---

#[test]
fn an_identical_delegation_wrapper_is_not_reported() {
    // Six executable lines and one statement: a docstring and a `return other(...)`.
    // The bodies are identical because the idiom is identical, and there is nothing
    // in either to extract.
    let config = config_for("exact_delegation.py");
    let report = scan(&config);
    assert_eq!(report.functions.len(), 2, "both must be extracted, and then refused");
    assert_eq!(report.functions[0].size.executable_lines, 6);
    assert_eq!(report.functions[0].size.executable_stmts, 1);
    assert!(report.pairs.is_empty(), "got {:?}", pairs_of(&report));

    // Establish that only the statement guard stands between it and a report.
    let mut relaxed = config;
    relaxed.scan.exact_min_stmts = 1;
    let found = pairs_of(&scan(&relaxed));
    assert_eq!(found.len(), 1, "the pair is found; the guard is what refuses it: {found:?}");
    assert_eq!(found[0].3, Tier::Exact);
}

// --- Negative: below the exact tier's line floor ---

#[test]
fn a_three_line_identical_pair_is_not_reported() {
    let config = config_for("exact_tiny.py");
    let report = scan(&config);
    assert!(report.pairs.is_empty(), "got {:?}", pairs_of(&report));
    assert!(
        report.functions.is_empty(),
        "three executable lines is below the extraction floor, so they are not even indexed"
    );

    let mut relaxed = config;
    relaxed.scan.exact_min_lines = Some(3);
    relaxed.scan.exact_min_stmts = 1;
    let found = pairs_of(&scan(&relaxed));
    assert_eq!(found.len(), 1, "the pair is identical; the floor is what refuses it: {found:?}");
    assert_eq!(found[0].3, Tier::Exact);
}

// --- Negative: below the fuzzy tier's line floor ---

#[test]
fn a_six_line_fuzzy_pair_is_not_reported() {
    // The fuzzy tier asks for nine executable lines. An exact match would clear the
    // exact tier's floor of five — this pair is not one.
    let mut config = config_for("similar_short.py");
    config.scan.threshold = 0.7;
    let report = scan(&config);
    assert!(report.pairs.is_empty(), "got {:?}", pairs_of(&report));
    assert_eq!(report.functions[0].size.executable_lines, 6);

    let mut relaxed = config.clone();
    relaxed.scan.similar_min_lines = Some(5);
    let found = pairs_of(&scan(&relaxed));
    assert_eq!(found.len(), 1, "expected the pair once the floor is lowered: {found:?}");
    let (_, _, similarity, tier) = &found[0];
    assert!(
        *similarity >= config.scan.threshold && *similarity < 1.0,
        "fixture must clear the threshold and fall short of an exact match, got {similarity}"
    );
    assert_eq!(*tier, Tier::Similar, "so the line floor is the only thing refusing it");
}

// --- Negative: padding is not evidence ---

#[test]
fn a_pair_padded_with_prose_is_not_reported() {
    // Eleven raw lines each, four executable lines each. A floor read off the raw
    // span would report this; the executable-line measure is not fooled.
    let config = config_for("padded_similar.py");
    let report = scan(&config);
    assert!(report.pairs.is_empty(), "got {:?}", pairs_of(&report));

    let mut relaxed = config.clone();
    relaxed.scan.exact_min_lines = Some(1);
    relaxed.scan.similar_min_lines = Some(4);
    let relaxed_report = scan(&relaxed);
    let found = pairs_of(&relaxed_report);
    assert_eq!(found.len(), 1, "expected the pair once the floor is lowered: {found:?}");
    assert!(
        found[0].2 >= config.scan.threshold && found[0].2 < 1.0,
        "fixture must be a fuzzy match above the threshold, got {}",
        found[0].2
    );
    let sizes = &relaxed_report.functions[0];
    assert_eq!(sizes.size.executable_lines, 4, "four executable lines...");
    assert_eq!(
        sizes.end_line - sizes.start_line + 1,
        11,
        "...spread over eleven raw ones, which is what makes this the interesting case"
    );
}

// --- Negative: contained runs ---

#[test]
fn a_twelve_line_fuzzy_containment_is_not_reported() {
    let mut config = config_for("contained_fuzzy.py");
    config.containment.enabled = true;
    let report = scan(&config);
    assert!(report.containments.is_empty(), "got {:?}", containments_of(&report));

    let mut relaxed = config.clone();
    relaxed.containment.similar_min_fragment_lines = Some(12);
    let found = containments_of(&scan(&relaxed));
    assert_eq!(found.len(), 1, "expected the finding once the floor is lowered: {found:?}");
    let (_, _, score, tier) = &found[0];
    assert!(
        *score >= config.containment.threshold && *score < 1.0,
        "fixture must clear the containment threshold without matching exactly, got {score}"
    );
    assert_eq!(*tier, Tier::Similar, "so the fragment floor is the only thing refusing it");
}

#[test]
fn a_containment_failing_an_older_guard_is_not_reported() {
    // The tiers do not replace the containment guards, they compose with them.
    let mut config = config_for("contained_guarded.py");
    config.containment.enabled = true;
    let report = scan(&config);
    assert!(report.containments.is_empty(), "got {:?}", containments_of(&report));

    let mut relaxed = config;
    relaxed.containment.max_run_fraction = 0.95;
    let found = containments_of(&scan(&relaxed));
    assert_eq!(found.len(), 1, "the run is exactly shared; the guard refuses it: {found:?}");
    assert_eq!(found[0].3, Tier::Exact, "and the tier gates accept it on the default floors");
}

// --- Suppression works on either tier ---

#[test]
fn a_suppression_directive_silences_an_exact_tier_pair() {
    let report = scan(&config_for("suppressed_exact.py"));
    assert!(report.pairs.is_empty(), "got {:?}", pairs_of(&report));
    assert_eq!(report.suppression_stats.inline_functions, 1);

    // The same pair without the directive is reported, so the fixture is not passing
    // for want of a clone.
    let unsuppressed = scan(&config_for("exact_short.py"));
    assert_eq!(only_pair(&unsuppressed).3, Tier::Exact);
}

#[test]
fn a_suppression_directive_silences_a_similar_tier_pair() {
    let report = scan(&config_for("suppressed_similar.py"));
    assert!(report.pairs.is_empty(), "got {:?}", pairs_of(&report));
    assert_eq!(report.suppression_stats.inline_functions, 1);

    let unsuppressed = scan(&config_for("similar_long.py"));
    assert_eq!(only_pair(&unsuppressed).3, Tier::Similar);
}

// --- The retained alias ---

#[test]
fn min_lines_alone_still_means_one_floor_for_both_tiers() {
    // A config written before the tiers existed keeps behaving as it did: one floor,
    // applied to every reported pair, exact ones included.
    let mut config = config_for("exact_short.py");
    config.scan.min_lines = Some(10);
    assert!(scan(&config).pairs.is_empty(), "six executable lines is below a floor of ten");

    config.scan.min_lines = Some(6);
    assert_eq!(only_pair(&scan(&config)).3, Tier::Exact);
}
