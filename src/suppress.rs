use std::path::Path;

use crate::config::SuppressConfig;
use crate::discovery::matches_any_glob;

/// Tracks how many functions/files were suppressed and why.
#[derive(Debug, Default, Clone)]
pub struct SuppressionStats {
    /// Files suppressed by config glob patterns.
    pub config_files: usize,
    /// Files suppressed by `# biston: ignore-file` comment.
    pub file_comments: usize,
    /// Individual functions suppressed by `# biston: ignore` comment.
    pub inline_functions: usize,
}

/// Human-readable reference for every way to suppress a biston finding.
///
/// This is the single source of truth shared by the `biston usage` subcommand
/// and (in condensed form) the scan report's suppression hint, so anyone —
/// human or coding agent — reading a clone report can discover how to silence
/// a false positive without leaving the terminal.
pub fn suppression_help() -> &'static str {
    "\
biston: how to suppress a finding
=================================

Inline comments (in your Python source) — preferred for one-off cases:

  # biston: ignore-file
      Suppress the entire file. Must appear within the first 5 lines.

  # biston: ignore
      Suppress a single function. Place it inside the function body, or on
      the line immediately before the `def` / `async def`.

Config file (biston.toml, or [tool.biston] in pyproject.toml) — for whole
directories of generated or vendored code:

  [suppress]
  files = [\"generated/**\", \"migrations/**\"]
      Suppress every file matching these glob patterns.

Suppressed findings are still counted and reported in the \"Suppressed:\"
summary line, so nothing disappears silently.
"
}

/// One-line reminder shown at the foot of any report that surfaces clones.
///
/// Points readers at the inline-comment suppressions and the fuller
/// [`suppression_help`] reachable via `biston usage`. Returned without a
/// trailing newline so each formatter controls its own spacing.
pub fn suppression_hint() -> &'static str {
    "False positive? Add `# biston: ignore` above a function or \
     `# biston: ignore-file` at the top of a file. Run `biston usage` for \
     all suppression options."
}

/// Returns true if the file matches any suppress glob pattern in the config.
pub fn file_suppressed_by_config(path: &Path, root: &Path, config: &SuppressConfig) -> bool {
    matches_any_glob(path, root, &config.files)
}

/// Returns true if the first 5 lines of `source` contain `# biston: ignore-file`.
pub fn file_has_ignore_file_comment(source: &[u8]) -> bool {
    let text = String::from_utf8_lossy(source);
    text.split('\n').take(5).any(|line| line.contains("# biston: ignore-file"))
}

/// Returns true if the function body contains `# biston: ignore` (but NOT `# biston: ignore-file`)
/// or if the line immediately preceding the function's start byte contains `# biston: ignore`.
pub fn function_has_ignore_comment(
    source_text: &str,
    full_source: &[u8],
    fn_start_byte: usize,
) -> bool {
    // Check function body for `# biston: ignore` but not `# biston: ignore-file`
    if has_ignore_not_ignore_file(source_text) {
        return true;
    }

    // Check the preceding line in the full source
    if fn_start_byte > 0 {
        let before = &full_source[..fn_start_byte];
        let before_text = String::from_utf8_lossy(before);
        if let Some(prev_line) = before_text.trim_end_matches('\n').rsplit('\n').next() {
            if has_ignore_not_ignore_file(prev_line) {
                return true;
            }
        }
    }

    false
}

/// Check if any line in `text` starts (ignoring leading whitespace) with `# biston: ignore`
/// but NOT `# biston: ignore-file`.
fn has_ignore_not_ignore_file(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("# biston: ignore") && !trimmed.starts_with("# biston: ignore-file")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- suppression_help ---

    #[test]
    fn help_documents_every_suppression_mechanism() {
        let help = suppression_help();
        assert!(help.contains("# biston: ignore-file"), "should document the file comment");
        assert!(help.contains("# biston: ignore"), "should document the function comment");
        assert!(help.contains("[suppress]"), "should document the config glob section");
        assert!(help.contains("files"), "should mention the config `files` key");
    }

    // --- suppression_hint ---

    #[test]
    fn hint_points_at_inline_comment_and_usage_command() {
        let hint = suppression_hint();
        assert!(hint.contains("# biston: ignore"), "hint should show the inline comment");
        assert!(hint.contains("biston usage"), "hint should point at the usage command");
        assert!(!hint.contains('\n'), "hint is a single line; callers add the newline");
    }

    // --- file_suppressed_by_config ---

    #[test]
    fn config_glob_matches_file() {
        let config = SuppressConfig { files: vec!["generated/**".to_owned()] };
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/generated/models.py");
        assert!(file_suppressed_by_config(&path, &root, &config));
    }

    #[test]
    fn config_glob_does_not_match_unrelated_file() {
        let config = SuppressConfig { files: vec!["generated/**".to_owned()] };
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/main.py");
        assert!(!file_suppressed_by_config(&path, &root, &config));
    }

    #[test]
    fn config_empty_patterns_matches_nothing() {
        let config = SuppressConfig::default();
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/anything.py");
        assert!(!file_suppressed_by_config(&path, &root, &config));
    }

    // --- file_has_ignore_file_comment ---

    #[test]
    fn ignore_file_on_first_line() {
        let source = b"# biston: ignore-file\ndef foo():\n    pass\n";
        assert!(file_has_ignore_file_comment(source));
    }

    #[test]
    fn ignore_file_on_fifth_line() {
        let source = b"# comment\n# comment\n# comment\n# comment\n# biston: ignore-file\ndef foo():\n    pass\n";
        assert!(file_has_ignore_file_comment(source));
    }

    #[test]
    fn ignore_file_on_sixth_line_not_detected() {
        let source = b"#\n#\n#\n#\n#\n# biston: ignore-file\ndef foo():\n    pass\n";
        assert!(!file_has_ignore_file_comment(source));
    }

    #[test]
    fn no_ignore_file_comment() {
        let source = b"def foo():\n    pass\n";
        assert!(!file_has_ignore_file_comment(source));
    }

    #[test]
    fn plain_ignore_not_treated_as_ignore_file() {
        let source = b"# biston: ignore\ndef foo():\n    pass\n";
        assert!(!file_has_ignore_file_comment(source));
    }

    // --- function_has_ignore_comment ---

    #[test]
    fn inline_ignore_in_function_body() {
        let source_text = "def foo():\n    # biston: ignore\n    pass\n";
        let full_source = source_text.as_bytes();
        assert!(function_has_ignore_comment(source_text, full_source, 0));
    }

    #[test]
    fn ignore_file_in_body_not_treated_as_inline() {
        let source_text = "def foo():\n    # biston: ignore-file\n    pass\n";
        let full_source = source_text.as_bytes();
        assert!(!function_has_ignore_comment(source_text, full_source, 0));
    }

    #[test]
    fn ignore_on_preceding_line() {
        let full_source = b"# biston: ignore\ndef foo():\n    pass\n";
        let fn_start = 17; // byte offset of "def"
        let source_text = "def foo():\n    pass\n";
        assert!(function_has_ignore_comment(source_text, full_source, fn_start));
    }

    #[test]
    fn no_ignore_comment() {
        let source_text = "def foo():\n    pass\n";
        let full_source = source_text.as_bytes();
        assert!(!function_has_ignore_comment(source_text, full_source, 0));
    }

    #[test]
    fn ignore_file_on_preceding_line_not_treated_as_inline() {
        let full_source = b"# biston: ignore-file\ndef foo():\n    pass\n";
        let fn_start = 22;
        let source_text = "def foo():\n    pass\n";
        assert!(!function_has_ignore_comment(source_text, full_source, fn_start));
    }

    #[test]
    fn ignore_in_string_literal_not_treated_as_comment() {
        let source_text = "def foo():\n    msg = \"# biston: ignore\"\n    pass\n";
        let full_source = source_text.as_bytes();
        assert!(!function_has_ignore_comment(source_text, full_source, 0));
    }
}
