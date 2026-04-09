use std::path::{Path, PathBuf};

use anyhow::Context;
use ignore::WalkBuilder;

use crate::config::ScanConfig;

/// Discover Python files under `root`, respecting `.gitignore` and config filters.
///
/// Returns a sorted list of file paths for deterministic processing.
pub fn discover_files(root: &Path, config: &ScanConfig) -> anyhow::Result<Vec<PathBuf>> {
    let mut builder = WalkBuilder::new(root);
    builder.hidden(false).git_ignore(true).git_global(true);

    let mut files = Vec::new();

    for entry in builder.build() {
        let entry = entry.context("error walking directory")?;

        // Skip directories
        if entry.file_type().is_some_and(|ft| ft.is_dir()) {
            continue;
        }

        let path = entry.into_path();

        // Check include patterns
        if !matches_any_glob(&path, root, &config.include) {
            continue;
        }

        // Check exclude patterns
        if matches_any_glob(&path, root, &config.exclude) {
            continue;
        }

        files.push(path);
    }

    files.sort();
    Ok(files)
}

/// Check if a path matches any of the given glob patterns.
///
/// Patterns are matched against the path relative to `root`.
fn matches_any_glob(path: &Path, root: &Path, patterns: &[String]) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let relative_str = relative.to_string_lossy();

    patterns.iter().any(|pattern| {
        // Try matching against the relative path
        glob_match::glob_match(pattern, &relative_str)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_scan_config() -> ScanConfig {
        ScanConfig { include: vec!["**/*.py".to_owned()], exclude: vec![], ..ScanConfig::default() }
    }

    #[test]
    fn discovers_py_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.py"), "# python").expect("write");
        std::fs::write(dir.path().join("b.py"), "# python").expect("write");
        std::fs::write(dir.path().join("c.txt"), "text").expect("write");

        let files = discover_files(dir.path(), &default_scan_config()).expect("discover");
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.extension().is_some_and(|e| e == "py")));
    }

    #[test]
    fn respects_exclude_patterns() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("tests")).expect("mkdir");
        std::fs::write(dir.path().join("main.py"), "# python").expect("write");
        std::fs::write(dir.path().join("tests/test_main.py"), "# test").expect("write");

        let config = ScanConfig {
            include: vec!["**/*.py".to_owned()],
            exclude: vec!["tests/**".to_owned()],
            ..ScanConfig::default()
        };

        let files = discover_files(dir.path(), &config).expect("discover");
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("main.py"));
    }

    #[test]
    fn empty_directory_returns_empty_vec() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = discover_files(dir.path(), &default_scan_config()).expect("discover");
        assert!(files.is_empty());
    }

    #[test]
    fn discovers_files_in_subdirectories() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src/utils")).expect("mkdir");
        std::fs::write(dir.path().join("src/main.py"), "# python").expect("write");
        std::fs::write(dir.path().join("src/utils/helpers.py"), "# python").expect("write");

        let files = discover_files(dir.path(), &default_scan_config()).expect("discover");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn results_are_sorted() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("z.py"), "# python").expect("write");
        std::fs::write(dir.path().join("a.py"), "# python").expect("write");
        std::fs::write(dir.path().join("m.py"), "# python").expect("write");

        let files = discover_files(dir.path(), &default_scan_config()).expect("discover");
        assert_eq!(files.len(), 3);
        let names: Vec<_> = files
            .iter()
            .filter_map(|f| f.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.py", "m.py", "z.py"]);
    }

    #[test]
    fn respects_gitignore() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Initialize a git repo so .gitignore is respected
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("git init");

        std::fs::write(dir.path().join(".gitignore"), "*.pyc\n__pycache__/\n").expect("write");
        std::fs::write(dir.path().join("main.py"), "# python").expect("write");
        std::fs::write(dir.path().join("main.pyc"), "bytecode").expect("write");
        std::fs::create_dir_all(dir.path().join("__pycache__")).expect("mkdir");
        std::fs::write(dir.path().join("__pycache__/cached.py"), "# cached").expect("write");

        let files = discover_files(dir.path(), &default_scan_config()).expect("discover");
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("main.py"));
    }
}
