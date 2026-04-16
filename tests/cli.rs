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
