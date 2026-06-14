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
        .success()
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
        .success()
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

// --- Usage subcommand tests ---

#[test]
fn usage_command_explains_inline_comments() {
    Command::cargo_bin("biston")
        .unwrap()
        .arg("usage")
        .assert()
        .success()
        .stdout(predicate::str::contains("# biston: ignore-file"))
        .stdout(predicate::str::contains("# biston: ignore"))
        .stdout(predicate::str::contains("[suppress]"));
}

#[test]
fn scan_with_clones_mentions_suppression() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", &fixtures_dir()])
        .assert()
        .success()
        .stdout(predicate::str::contains("# biston: ignore"))
        .stdout(predicate::str::contains("biston usage"));
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
        .success()
        .stdout(predicate::str::contains("Clone cluster #1"));
}

#[test]
fn scan_json_format_valid() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", &fixtures_dir(), "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn scan_sarif_format_valid() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", &fixtures_dir(), "--format", "sarif"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sarif-schema"));
}

#[test]
fn suggest_flag_produces_output() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--suggest", &fixtures_dir()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Suggested abstraction"));
}

#[test]
fn suggest_flag_json_includes_suggestions() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["scan", "--suggest", "--format", "json", &fixtures_dir()])
        .assert()
        .success()
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
        .success()
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
        .success()
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

    assert!(via_focus_args.status.success(), "focus-args invocation should succeed");
    assert!(via_files.status.success(), "files invocation should succeed");
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

    assert!(via_focus_args.status.success());
    assert!(via_files.status.success());
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
    assert!(output.status.success());
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
        .success()
        .stdout(predicate::str::contains("\x1b["));
}

#[test]
fn color_never_suppresses_ansi() {
    Command::cargo_bin("biston")
        .unwrap()
        .args(["--color", "never", "scan", &fixtures_dir()])
        .assert()
        .success()
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
        .success()
        .stdout(predicate::str::contains("\x1b["));
}

#[test]
fn no_color_env_suppresses_ansi_in_auto_mode() {
    Command::cargo_bin("biston")
        .unwrap()
        .env("NO_COLOR", "1")
        .args(["scan", &fixtures_dir()])
        .assert()
        .success()
        .stdout(predicate::str::contains("\x1b[").not());
}

#[test]
fn verbose_flag_accepted() {
    Command::cargo_bin("biston").unwrap().args(["-v", "scan", &fixtures_dir()]).assert().success();
}

#[test]
fn quiet_flag_accepted() {
    Command::cargo_bin("biston").unwrap().args(["-q", "scan", &fixtures_dir()]).assert().success();
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
