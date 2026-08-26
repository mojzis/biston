#![allow(clippy::expect_used, reason = "integration-test helpers treat setup failures as fatal")]

use assert_cmd::Command;
use predicates::prelude::*;

mod common;

fn fixtures_dir() -> String {
    format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"))
}

// --- Stats subcommand tests ---

#[test]
fn stats_help_exits_zero() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("statistics"));
}

#[test]
fn stats_empty_dir_shows_zeros() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Files scanned:        0"));
}

#[test]
fn stats_fixtures_shows_statistics() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Scan statistics:"))
        .stdout(predicate::str::contains("Clone pairs:"))
        .stdout(predicate::str::contains("Clone clusters:"));
}

#[test]
fn stats_json_format_valid() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", &fixtures_dir(), "--format", "json"])
        .assert()
        .code(1)
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn stats_json_contains_expected_fields() {
    let output = Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", &fixtures_dir(), "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert!(json["files_scanned"].is_u64());
    assert!(json["functions_extracted"].is_u64());
    assert!(json["clone_pairs"].is_u64());
    assert!(json["clone_clusters"].is_u64());
    assert!(json["breakdown"].is_object());
}

// --- Scan subcommand tests ---

#[test]
fn scan_help_exits_zero() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Scan a directory"));
}

// --- Containment tests ---

/// A directory holding one function plus a second that ends by doing the same work.
fn containment_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = std::fs::read_to_string(format!("{}/containment_prepend.py", fixtures_dir()))
        .expect("read fixture");
    let (inner, outer) =
        source.split_once("\n\ndef load_then_normalize_records").expect("split fixture");
    std::fs::write(dir.path().join("contained.py"), inner).expect("write contained");
    std::fs::write(
        dir.path().join("container.py"),
        format!("def load_then_normalize_records{outer}"),
    )
    .expect("write container");
    dir
}

#[test]
fn scan_without_containment_flag_reports_nothing_directed() {
    let dir = containment_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("already implemented by").not());
}

#[test]
fn scan_with_containment_flag_reports_the_direction() {
    let dir = containment_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--containment", dir.path().to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("is already implemented by normalize_records"))
        .stdout(predicate::str::contains("call it instead"));
}

#[test]
fn stats_counts_containment_separately_from_clone_pairs() {
    let dir = containment_dir();
    let output = Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", "--containment", "--format", "json", dir.path().to_str().unwrap()])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["containment_findings"], 1, "got: {parsed}");
    assert_eq!(parsed["clone_pairs"], 0, "containment must not inflate clone_pairs: {parsed}");
}

#[test]
fn overview_with_containment_never_calls_a_file_with_findings_clean() {
    // `--containment` reaches `overview` too, and supersession removes the symmetric
    // pair. If overview does not render containment, the flag *deletes* the finding
    // and then reports the file as clean — strictly worse than not supporting it.
    let dir = containment_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["overview", "--containment", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("already-implemented runs"))
        .stdout(predicate::str::contains("already implemented by normalize_records"))
        .stdout(predicate::str::contains("clean files not shown").not());
}

#[test]
fn overview_containment_json_counts_findings() {
    let dir = containment_dir();
    let output = Command::cargo_bin("biston")
        .unwrap()
        .args(["overview", "--containment", "--format", "json", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["summary"]["containment_findings"], 1, "got: {parsed}");
    let total: u64 = parsed["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["containment_count"].as_u64().unwrap())
        .sum();
    assert_eq!(total, 2, "both sides of the finding should be marked: {parsed}");
}

#[test]
fn scan_containment_json_declares_the_current_schema_version() {
    let dir = containment_dir();
    let output = Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--containment", "--format", "json", dir.path().to_str().unwrap()])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["schema_version"], 3);
    assert_eq!(parsed["containments"][0]["role"], "suffix", "got: {parsed}");
    assert_eq!(
        parsed["containments"][0]["tier"], "exact",
        "every finding names the tier that accepted it: {parsed}"
    );
}

#[test]
fn scan_empty_dir_no_clones() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No clones detected"));
}

#[test]
fn scan_fixtures_detects_clones() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Clone cluster #1"));
}

/// A tempdir holding a single copy of one fixture, so a scan sees only that file.
fn fixture_in_tempdir(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let source =
        std::fs::read_to_string(format!("{}/{name}", fixtures_dir())).expect("read fixture");
    std::fs::write(dir.path().join(name), source).expect("write fixture");
    dir
}

#[test]
fn scan_reports_comment_only_differences_as_an_exact_clone() {
    // Through the CLI: two functions differing only in a docstring and comments must
    // be printed as an exact clone at 100% similarity, not as a near miss.
    let dir = fixture_in_tempdir("comment_noise.py");

    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Clone cluster #1"))
        .stdout(predicate::str::contains("aggregate_totals"))
        .stdout(predicate::str::contains("aggregate_sums"))
        .stdout(predicate::str::contains("similarity: 1.00"));
}

#[test]
fn scan_json_reports_comment_only_differences_at_similarity_one() {
    let dir = fixture_in_tempdir("comment_noise.py");
    let output = Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "findings exit 1: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let clusters = json["clusters"].as_array().expect("clusters array");
    assert_eq!(clusters.len(), 1, "expected exactly one cluster, got {json}");
    let similarity = clusters[0]["similarity"].as_f64().expect("similarity");
    assert!((similarity - 1.0).abs() < f64::EPSILON, "expected 1.0, got {similarity}");
    let names: Vec<&str> = clusters[0]["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .map(|f| f["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["aggregate_totals", "aggregate_sums"], "got {json}");
}

#[test]
fn scan_json_format_valid() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", &fixtures_dir(), "--format", "json"])
        .assert()
        .code(1)
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn an_alias_conflict_warns_on_stderr_and_leaves_json_parseable() {
    // The report is stdout and diagnostics are stderr. A warning printed into the
    // report would make `--format json` unparseable for whatever consumes it.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("biston.toml"), "[scan]\nmin_lines = 6\nexact_min_lines = 5\n")
        .expect("write config");
    std::fs::copy(format!("{}/tiers/exact_short.py", fixtures_dir()), dir.path().join("a.py"))
        .expect("copy fixture");

    let output = Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap(), "--format", "json"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("scan.min_lines is ignored"))
        .stderr(predicate::str::contains("scan.exact_min_lines"))
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output).expect("stdout must be JSON and nothing else");
    assert_eq!(parsed["schema_version"], 3);
}

#[test]
fn contradictory_size_floors_fail_the_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("biston.toml"),
        "[scan]\nexact_min_lines = 20\nsimilar_min_lines = 5\n",
    )
    .expect("write config");

    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must not exceed"));
}

#[test]
fn scan_sarif_format_valid() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", &fixtures_dir(), "--format", "sarif"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("sarif-schema"));
}

#[test]
fn suggest_flag_produces_output() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--suggest", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Suggested abstraction"));
}

#[test]
fn suggest_flag_json_includes_suggestions() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--suggest", "--format", "json", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("suggestions"));
}

// --- --tests-only flag tests ---

/// Build a tempdir with a production file and two test files.
/// Default scan should find only the production file; --tests-only the two test files.
fn mixed_prod_and_tests_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir src");
    std::fs::create_dir_all(dir.path().join("tests")).expect("mkdir tests");
    std::fs::write(dir.path().join("src/main.py"), "# prod\n").expect("write main");
    std::fs::write(dir.path().join("tests/test_a.py"), "# test\n").expect("write test_a");
    std::fs::write(dir.path().join("tests/test_b.py"), "# test\n").expect("write test_b");
    dir
}

#[test]
fn stats_default_excludes_tests() {
    let dir = mixed_prod_and_tests_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Files scanned:        1"));
}

#[test]
fn stats_tests_only_scans_only_test_files() {
    let dir = mixed_prod_and_tests_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", "--tests-only", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Files scanned:        2"));
}

#[test]
fn scan_tests_only_scans_only_test_files() {
    // Two identical test bodies (differing only by function name / variable names,
    // which normalization collapses). Under default scan `tests/**` is excluded;
    // under --tests-only the pair should surface as a clone.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("tests")).unwrap();
    let body = "\
def test_alpha():
    x = compute()
    assert x is not None
    assert isinstance(x, int)
    assert x > 0
    assert x < 100
    assert x != 0
    assert str(x) != ''
    assert x + 1 > x
    assert x - 1 < x
    assert x == x

def test_beta():
    y = compute()
    assert y is not None
    assert isinstance(y, int)
    assert y > 0
    assert y < 100
    assert y != 0
    assert str(y) != ''
    assert y + 1 > y
    assert y - 1 < y
    assert y == y
";
    std::fs::write(dir.path().join("tests/test_compute.py"), body).unwrap();

    // --tests-only: the test file is in scope, so the pair should surface and
    // the report must name both test functions in the detected cluster.
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--tests-only", dir.path().to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Clone cluster #1"))
        .stdout(predicate::str::contains("test_alpha"))
        .stdout(predicate::str::contains("test_beta"));
}

#[test]
fn scan_help_mentions_tests_only_flag() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--tests-only"));
}

// --- Focus-file (commit-hook) CLI tests ---

fn cluster_count(json: &serde_json::Value) -> usize {
    json["clusters"].as_array().expect("clusters array").len()
}

#[test]
fn scan_files_flag_restricts_pairs_to_focus() {
    let dir = common::multi_file_dir();
    let a_py = dir.path().join("a.py");

    // Baseline: no focus → 2 pairs.
    let baseline = Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--format", "json", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    let baseline_json: serde_json::Value =
        serde_json::from_slice(&baseline.stdout).expect("valid json");
    assert_eq!(cluster_count(&baseline_json), 2);

    // With --files a.py → only the a-b pair.
    let focused = Command::cargo_bin("biston")
        .unwrap()
        .args([
            "scan",
            "--format",
            "json",
            "--files",
            a_py.to_str().unwrap(),
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let focused_json: serde_json::Value =
        serde_json::from_slice(&focused.stdout).expect("valid json");
    assert_eq!(cluster_count(&focused_json), 1);
}

#[test]
fn scan_files_flag_accepts_relative_path() {
    // Simulates `git diff --name-only` output: relative paths, with the hook
    // run from the repo root.
    let dir = common::multi_file_dir();
    let output = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--format", "json", "--files", "a.py", "."])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(cluster_count(&json), 1);
}

#[test]
fn scan_files_from_reads_list_from_file() {
    let dir = common::multi_file_dir();
    let list_path = dir.path().join("changed.txt");
    std::fs::write(&list_path, "a.py\nb.py\n").expect("write list");

    let output = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--format", "json", "--files-from", list_path.to_str().unwrap(), "."])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(cluster_count(&json), 1);
}

#[test]
fn scan_files_from_stdin_reads_list() {
    let dir = common::multi_file_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--format", "json", "--files-from", "-", "."])
        .write_stdin("a.py\n")
        .assert()
        .code(1)
        .stdout(predicate::function(|out: &str| {
            let json: serde_json::Value = serde_json::from_str(out).expect("valid json");
            cluster_count(&json) == 1
        }));
}

#[test]
fn scan_focus_args_matches_files_flag() {
    // `--focus-args a.py b.py` (positional focus files, implicit `.` scan root)
    // should produce the same report as `--files a.py --files b.py .`.
    let dir = common::multi_file_dir();

    let via_focus_args = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--focus-args", "--format", "json", "a.py", "b.py"])
        .output()
        .unwrap();
    let via_files = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--files", "a.py", "--files", "b.py", "--format", "json", "."])
        .output()
        .unwrap();

    assert_eq!(via_focus_args.status.code(), Some(1), "focus-args run reports findings");
    assert_eq!(via_files.status.code(), Some(1), "files run reports findings");
    let focus_json: serde_json::Value =
        serde_json::from_slice(&via_focus_args.stdout).expect("valid json");
    let files_json: serde_json::Value =
        serde_json::from_slice(&via_files.stdout).expect("valid json");
    // Compare cluster *content*, not just count: a swap or canonicalisation
    // regression that left pair counts unchanged would otherwise slip past.
    assert_eq!(
        focus_json["clusters"], files_json["clusters"],
        "--focus-args and --files should emit identical clusters"
    );
}

#[test]
fn scan_focus_args_empty_emits_no_pairs() {
    // Simulates pre-commit invoking the hook on a commit that touched no
    // Python files: positional list is empty, exit 0, no pairs reported.
    let dir = common::multi_file_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--focus-args", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let json: serde_json::Value = serde_json::from_str(out).expect("valid json");
            cluster_count(&json) == 0
        }));
}

#[test]
fn scan_focus_args_conflicts_with_files() {
    let dir = common::multi_file_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--focus-args", "a.py", "--files", "b.py"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--focus-args").and(predicate::str::contains("--files")));
}

#[test]
fn scan_focus_args_conflicts_with_files_from() {
    let dir = common::multi_file_dir();
    let list = dir.path().join("changed.txt");
    std::fs::write(&list, "a.py\n").expect("write list");
    Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--focus-args", "a.py", "--files-from", list.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("--focus-args").and(predicate::str::contains("--files-from")),
        );
}

#[test]
fn stats_focus_args_matches_files_flag() {
    let dir = common::multi_file_dir();

    let via_focus_args = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["stats", "--focus-args", "--format", "json", "a.py"])
        .output()
        .unwrap();
    let via_files = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["stats", "--files", "a.py", "--format", "json", "."])
        .output()
        .unwrap();

    assert_eq!(via_focus_args.status.code(), Some(1), "a scan that reports findings exits 1");
    assert_eq!(via_files.status.code(), Some(1), "a scan that reports findings exits 1");
    let focus_json: serde_json::Value =
        serde_json::from_slice(&via_focus_args.stdout).expect("valid json");
    let files_json: serde_json::Value =
        serde_json::from_slice(&via_files.stdout).expect("valid json");
    assert_eq!(
        focus_json, files_json,
        "--focus-args and --files should emit identical stats payloads"
    );
}

#[test]
fn stats_focus_args_empty_reports_zero_pairs() {
    let dir = common::multi_file_dir();
    let output = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["stats", "--focus-args", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["clone_pairs"].as_u64().unwrap(), 0);
}

#[test]
fn stats_focus_args_conflicts_with_files() {
    let dir = common::multi_file_dir();
    Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["stats", "--focus-args", "a.py", "--files", "b.py"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--focus-args").and(predicate::str::contains("--files")));
}

#[test]
fn stats_focus_args_conflicts_with_files_from() {
    let dir = common::multi_file_dir();
    let list = dir.path().join("changed.txt");
    std::fs::write(&list, "a.py\n").expect("write list");
    Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["stats", "--focus-args", "a.py", "--files-from", list.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("--focus-args").and(predicate::str::contains("--files-from")),
        );
}

// --- Pre-commit invocation pattern ---
//
// Simulates how `pre-commit` / `prek` invoke the hook: the framework changes
// to the repo root and passes the staged Python files as trailing positional
// arguments to `biston scan --focus-args`. Uses a fixture with two
// independent clone pairs so we can verify that focusing on one pair hides
// the other.

#[test]
fn precommit_style_focus_reports_only_involved_cluster() {
    // A↔B clone pair (filter shape), C↔D clone pair (aggregate shape).
    // Focusing on A.py should surface A↔B and hide C↔D entirely.
    let dir = common::multi_file_dir();
    let output = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--focus-args", "--format", "json", "a.py"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "a scan that reports findings exits 1");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(cluster_count(&json), 1, "only the a↔b cluster should remain");
    let cluster = &json["clusters"][0];
    let files: Vec<String> = cluster["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|f| f["file"].as_str().map(std::string::ToString::to_string))
        .collect();
    assert!(
        files.iter().any(|f| f.ends_with("a.py")),
        "focused cluster should involve a.py, got {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.ends_with("c.py") || f.ends_with("d.py")),
        "c.py/d.py should not appear in a focus on a.py, got {files:?}"
    );
}

#[test]
fn precommit_style_focus_on_clone_free_file_reports_nothing() {
    // e.py has only one short, structurally unique function — no clones
    // involve it. `biston scan --focus-args e.py` should exit 0 with an
    // empty cluster list.
    let dir = common::multi_file_dir();
    std::fs::write(
        dir.path().join("e.py"),
        "def unique_shape(config):\n    \
         \"\"\"Structurally unique.\"\"\"\n    \
         handler = config.handler\n    \
         token = handler.token()\n    \
         remaining = handler.quota()\n    \
         if remaining <= 0:\n        \
         handler.refresh()\n        \
         remaining = handler.quota()\n    \
         handler.commit(token)\n    \
         return remaining\n",
    )
    .expect("write e.py");

    let output = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--focus-args", "--format", "json", "e.py"])
        .output()
        .unwrap();
    assert!(output.status.success(), "clone-free focus should still exit 0");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(cluster_count(&json), 0, "no clusters involve e.py");
}

#[test]
fn precommit_style_empty_focus_exits_zero_with_no_pairs() {
    // `pre-commit` passes zero positionals when no matching files changed —
    // the hook must pass silently rather than falling back to a full scan.
    let dir = common::multi_file_dir();
    let output = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .args(["scan", "--focus-args", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "empty focus list must not fail the hook");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(cluster_count(&json), 0);
}

#[test]
fn stats_files_flag_restricts_to_focus() {
    let dir = common::multi_file_dir();
    let a_py = dir.path().join("a.py");
    let output = Command::cargo_bin("biston")
        .unwrap()
        .args([
            "stats",
            "--format",
            "json",
            "--files",
            a_py.to_str().unwrap(),
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["clone_pairs"].as_u64().unwrap(), 1);
}

// --- --version / --color / -v / -q / completions tests ---

#[test]
fn version_flag_prints_pkg_version() {
    Command::cargo_bin("biston")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn color_always_emits_ansi() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["--color", "always", "scan", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\x1b["));
}

#[test]
fn color_never_suppresses_ansi() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["--color", "never", "scan", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\x1b[").not());
}

#[test]
fn no_color_env_suppresses_ansi() {
    // In --color=auto (the default) mode, NO_COLOR disables color even
    // if stdout were a TTY. assert_cmd captures stdout, which is not a
    // TTY — but setting --color=always would normally force colour.
    // NO_COLOR should not override an explicit --color=always, only auto.
    Command::cargo_bin("biston")
        .unwrap()
        .env("NO_COLOR", "1")
        .args(["--color", "always", "scan", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\x1b["));
}

#[test]
fn no_color_env_suppresses_ansi_in_auto_mode() {
    Command::cargo_bin("biston")
        .unwrap()
        .env("NO_COLOR", "1")
        .args(["scan", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\x1b[").not());
}

#[test]
fn verbose_flag_accepted() {
    Command::cargo_bin("biston").unwrap().args(["-v", "scan", &fixtures_dir()]).assert().code(1);
}

#[test]
fn quiet_flag_accepted() {
    Command::cargo_bin("biston").unwrap().args(["-q", "scan", &fixtures_dir()]).assert().code(1);
}

#[test]
fn completions_bash_outputs_script() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_biston"));
}

#[test]
fn completions_zsh_outputs_script() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("#compdef biston"));
}

// --- Guide subcommand ---

/// The docs pages are the guide text. Comparing CLI output against them is what
/// keeps the site and the terminal byte-identical; a snapshot copied into the
/// test would just be a third version to forget to update.
const SETUP_DOC: &str = include_str!("../docs/src/guide/setup.md");
const TRIAGE_DOC: &str = include_str!("../docs/src/guide/triage.md");
const TUNE_DOC: &str = include_str!("../docs/src/guide/tune.md");

fn guide_in(dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::cargo_bin("biston")
        .expect("the biston binary should be built")
        .current_dir(dir)
        .arg("guide")
        .args(args)
        .output()
        .expect("guide should run");
    assert!(output.status.success(), "guide should exit 0, got {:?}", output.status.code());
    String::from_utf8(output.stdout).expect("guide output is UTF-8")
}

#[test]
fn guide_setup_snapshot_matches_the_docs_page() {
    let dir = tempfile::tempdir().unwrap();
    let out = guide_in(dir.path(), &["setup"]);
    assert_eq!(out, format!("# biston guide: setup\n\n{SETUP_DOC}"));
}

#[test]
fn guide_triage_snapshot_matches_the_docs_page() {
    let dir = tempfile::tempdir().unwrap();
    let out = guide_in(dir.path(), &["triage"]);
    assert_eq!(out, format!("# biston guide: triage\n\n{TRIAGE_DOC}"));
}

#[test]
fn guide_tune_snapshot_matches_the_docs_page() {
    let dir = tempfile::tempdir().unwrap();
    let out = guide_in(dir.path(), &["tune"]);
    assert_eq!(out, format!("# biston guide: tune\n\n{TUNE_DOC}"));
}

#[test]
fn guide_auto_selects_setup_when_nothing_is_configured() {
    let dir = tempfile::tempdir().unwrap();
    let out = guide_in(dir.path(), &[]);
    assert_eq!(out, format!("# biston guide: not configured here -> setup\n\n{SETUP_DOC}"));
}

#[test]
fn guide_auto_selects_triage_from_biston_toml() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("biston.toml"), "[scan]\nthreshold = 0.9\n").unwrap();
    let out = guide_in(dir.path(), &[]);
    assert_eq!(
        out,
        format!("# biston guide: configured via biston.toml -> triage\n\n{TRIAGE_DOC}")
    );
}

#[test]
fn guide_auto_selects_triage_from_pyproject() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pyproject.toml"), "[project]\nname = \"x\"\n\n[tool.biston]\n")
        .unwrap();
    let out = guide_in(dir.path(), &[]);
    assert!(
        out.starts_with("# biston guide: configured via pyproject.toml [tool.biston] -> triage\n"),
        "header should name the source, got {:?}",
        out.lines().next(),
    );
}

#[test]
fn guide_auto_selects_setup_when_pyproject_has_no_biston_table() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pyproject.toml"), "[project]\nname = \"x\"\n\n[tool.ruff]\n")
        .unwrap();
    let out = guide_in(dir.path(), &[]);
    assert!(out.starts_with("# biston guide: not configured here -> setup\n"));
}

#[test]
fn guide_auto_selects_triage_from_pre_commit_config() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".pre-commit-config.yaml"),
        "repos:\n  - repo: https://github.com/mojzis/biston\n    rev: v0.6.0\n    hooks:\n      - id: biston\n",
    )
    .unwrap();
    let out = guide_in(dir.path(), &[]);
    assert!(out.starts_with("# biston guide: configured via .pre-commit-config.yaml -> triage\n"));
}

#[test]
fn guide_header_precedence_prefers_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("biston.toml"), "[scan]\n").unwrap();
    std::fs::write(dir.path().join("pyproject.toml"), "[tool.biston]\n").unwrap();
    std::fs::write(
        dir.path().join(".pre-commit-config.yaml"),
        "repos:\n  - repo: https://github.com/mojzis/biston\n",
    )
    .unwrap();
    let out = guide_in(dir.path(), &[]);
    assert!(
        out.starts_with("# biston guide: configured via biston.toml -> triage\n"),
        "config file wins the header, got {:?}",
        out.lines().next(),
    );
}

#[test]
fn guide_never_auto_selects_tune() {
    for configured in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        if configured {
            std::fs::write(dir.path().join("biston.toml"), "[scan]\n").unwrap();
        }
        let out = guide_in(dir.path(), &[]);
        assert!(!out.starts_with("# biston guide: tune"), "tune is reference, never auto-selected");
        assert!(!out.contains("-> tune"), "tune is reference, never auto-selected");
    }
}

#[test]
fn guide_help_states_the_auto_selection_rule() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["guide", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not configured"))
        .stdout(predicate::str::contains("never auto-selected"))
        .stdout(predicate::str::contains("repository root"));
}

// --- The deprecated `usage` alias ---

#[test]
fn usage_emits_the_tune_guide() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("biston")
        .unwrap()
        .current_dir(dir.path())
        .arg("usage")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, format!("# biston guide: tune\n\n{TUNE_DOC}"));
}

#[test]
fn usage_warns_that_it_is_deprecated() {
    let output = Command::cargo_bin("biston").unwrap().arg("usage").output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("deprecated") && stderr.contains("biston guide tune"),
        "the deprecation should name its replacement, got {stderr:?}",
    );
}

#[test]
fn usage_is_absent_from_help() {
    let output = Command::cargo_bin("biston").unwrap().arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("guide"), "guide should be advertised");
    assert!(!stdout.contains("usage\n"), "the deprecated alias should be hidden: {stdout}");
    assert!(
        !stdout.contains("Deprecated alias"),
        "the deprecated alias should be hidden: {stdout}",
    );
}

// --- The footer that leads from a failed gate to the guide ---

#[test]
fn footer_points_at_the_triage_guide_on_findings() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", &fixtures_dir()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("biston guide triage"));
}

#[test]
fn footer_absent_when_there_are_no_findings() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("biston guide triage").not());
}

#[test]
fn footer_absent_from_json_and_sarif() {
    for format in ["json", "sarif"] {
        Command::cargo_bin("biston")
            .unwrap()
            .args(["scan", &fixtures_dir(), "--format", format])
            .assert()
            .code(1)
            .stdout(predicate::str::contains("biston guide triage").not());
    }
}

#[test]
fn footer_survives_a_non_tty_stdout() {
    // `assert_cmd` always pipes stdout, so this run is exactly the aggregator's:
    // no terminal, no colour. The footer must still be there — it is the only
    // breadcrumb a captured failure leaves behind.
    let output =
        Command::cargo_bin("biston").unwrap().args(["scan", &fixtures_dir()]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("biston guide triage"), "footer missing from piped output: {stdout}");
    assert!(!stdout.contains('\x1b'), "piped output should carry no ANSI escapes");
}

#[test]
fn overview_footer_points_at_the_triage_guide() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["overview", &fixtures_dir()])
        .assert()
        .success()
        .stdout(predicate::str::contains("biston guide triage"));
}

// --- Exit codes ---

#[test]
fn scan_exits_zero_on_a_clean_tree() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("only.py"), "def solo():\n    return 1\n").unwrap();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap()])
        .assert()
        .code(0);
}

#[test]
fn scan_exits_one_on_findings() {
    Command::cargo_bin("biston").unwrap().args(["scan", &fixtures_dir()]).assert().code(1);
}

#[test]
fn stats_exits_zero_on_a_clean_tree() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["stats", dir.path().to_str().unwrap()])
        .assert()
        .code(0);
}

#[test]
fn stats_exits_one_on_findings() {
    Command::cargo_bin("biston").unwrap().args(["stats", &fixtures_dir()]).assert().code(1);
}

#[test]
fn bad_flag_exits_two() {
    for subcommand in ["scan", "stats", "overview"] {
        Command::cargo_bin("biston")
            .unwrap()
            .args([subcommand, "--not-a-flag"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("unexpected argument"));
    }
}

#[test]
fn unreadable_path_exits_two() {
    // Distinct from the findings code: a gate has to be able to tell "this tree
    // has duplication" from "biston could not look at this tree".
    for subcommand in ["scan", "stats"] {
        Command::cargo_bin("biston")
            .unwrap()
            .args([subcommand, "/nonexistent/definitely/not/here"])
            .assert()
            .code(2);
    }
}

#[test]
fn invalid_config_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("biston.toml"),
        "[scan]\nexact_min_lines = 20\nsimilar_min_lines = 5\n",
    )
    .unwrap();
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", dir.path().to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("must not exceed"));
}

#[test]
fn a_failing_scan_piped_to_a_shell_still_shows_the_footer() {
    // The acceptance case: an aggregator captures stdout and reports the exit code.
    let output =
        Command::cargo_bin("biston").unwrap().args(["scan", &fixtures_dir()]).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "findings must trip the gate");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Run `biston guide triage`"), "got {stdout}");
}
