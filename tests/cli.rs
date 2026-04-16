use assert_cmd::Command;
use predicates::prelude::*;

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
