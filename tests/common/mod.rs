//! Shared fixtures for focus-file integration tests.
//!
//! Integration test crates compile independently, so we make this a `mod`
//! that each test file includes explicitly. Unused helpers in a given file
//! would warn, so every public helper is marked `#[allow(dead_code, ...)]`.

#![allow(clippy::expect_used, reason = "test helpers treat fixture setup failures as fatal")]

use std::path::Path;

#[allow(dead_code, reason = "each integration test file uses a different subset of helpers")]
pub fn write_filter_shape(dir: &Path, name: &str, fn_name: &str) {
    let src = format!(
        "def {fn_name}(items, threshold):\n    \
         \"\"\"Filter shape.\"\"\"\n    \
         matched = []\n    \
         checked = 0\n    \
         for item in items:\n        \
         checked += 1\n        \
         value = item.score()\n        \
         if value > threshold:\n            \
         matched.append(item)\n    \
         log_count(len(matched))\n    \
         log_count(checked)\n    \
         return matched\n"
    );
    std::fs::write(dir.join(name), src).expect("write");
}

#[allow(dead_code, reason = "each integration test file uses a different subset of helpers")]
pub fn write_aggregate_shape(dir: &Path, name: &str, fn_name: &str) {
    // Structurally distinct from the filter shape: while-loop, dict
    // accumulator, different control flow — shouldn't cluster with filter_*.
    let src = format!(
        "def {fn_name}(records):\n    \
         \"\"\"Aggregate shape.\"\"\"\n    \
         totals = {{}}\n    \
         i = 0\n    \
         while i < len(records):\n        \
         key = records[i].group\n        \
         totals[key] = totals.get(key, 0) + records[i].amount\n        \
         i += 1\n    \
         emit_summary(totals)\n    \
         emit_count(i)\n    \
         return totals\n"
    );
    std::fs::write(dir.join(name), src).expect("write");
}

#[allow(dead_code, reason = "each integration test file uses a different subset of helpers")]
pub fn multi_file_dir() -> tempfile::TempDir {
    // Two independent clone pairs with different shapes so the baseline
    // produces two separate clusters (not one giant transitive cluster).
    let dir = tempfile::tempdir().expect("tempdir");
    write_filter_shape(dir.path(), "a.py", "filter_a");
    write_filter_shape(dir.path(), "b.py", "filter_b");
    write_aggregate_shape(dir.path(), "c.py", "aggregate_c");
    write_aggregate_shape(dir.path(), "d.py", "aggregate_d");
    dir
}
