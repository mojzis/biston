use std::fmt::Write;

use anyhow::Context;
use rustc_hash::FxHashSet;
use serde::Serialize;

use crate::report::{cluster_pairs, CloneReport};
use crate::tier::Tier;

/// Statistics computed from a clone detection report.
#[derive(Debug, Serialize)]
pub struct ScanStats {
    /// Number of files scanned.
    pub files_scanned: usize,
    /// Number of functions extracted.
    pub functions_extracted: usize,
    /// Number of symmetric clone pairs detected.
    ///
    /// Deliberately excludes containment findings, which are counted separately by
    /// `containment_findings`. Existing CI gates read this field, so folding a new
    /// kind of finding into it would silently change what they enforce.
    ///
    /// Enabling containment does still *reduce* this count, by the number of pairs
    /// superseded by a directed finding for the same two functions. That direction is
    /// safe for a gate — it never reports more than before — but it is not zero.
    pub clone_pairs: usize,
    /// Number of directed containment findings.
    ///
    /// Always zero unless containment detection is enabled.
    pub containment_findings: usize,
    /// Number of clone clusters (groups of mutually similar functions).
    pub clone_clusters: usize,
    /// Number of distinct functions involved in at least one clone pair.
    pub functions_in_clones: usize,
    /// Number of distinct files containing at least one cloned function.
    pub files_with_clones: usize,
    /// Similarity statistics (None if no pairs).
    pub similarity: Option<SimilarityStats>,
    /// Breakdown of pairs by similarity range.
    pub breakdown: SimilarityBreakdown,
    /// Breakdown of pairs by the acceptance tier that admitted them.
    ///
    /// Distinct from `breakdown`, which bins the *score*: a pair can score 1.0 and
    /// still be a `similar`-tier finding when it cleared the fuzzy rule rather than
    /// the exact one.
    pub tiers: TierBreakdown,
}

/// Counts of clone pairs by acceptance tier.
#[derive(Debug, Serialize)]
pub struct TierBreakdown {
    /// Pairs accepted by the exact tier.
    pub exact: usize,
    /// Pairs accepted by the similar tier.
    pub similar: usize,
}

/// Min/max/average similarity across all clone pairs.
#[derive(Debug, Serialize)]
pub struct SimilarityStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
}

/// Breakdown of clone pairs by similarity range.
#[derive(Debug, Serialize)]
pub struct SimilarityBreakdown {
    /// Pairs with similarity == 1.0.
    pub exact: usize,
    /// Pairs with 0.9 <= similarity < 1.0.
    pub near: usize,
    /// Pairs with threshold <= similarity < 0.9.
    pub similar: usize,
}

/// Compute statistics from a clone detection report.
pub fn compute_stats(report: &CloneReport) -> ScanStats {
    let clusters = cluster_pairs(&report.pairs, report.functions.len());

    let mut involved: FxHashSet<usize> = FxHashSet::default();
    for pair in &report.pairs {
        involved.insert(pair.left);
        involved.insert(pair.right);
    }

    let files_with_clones = involved
        .iter()
        .map(|&idx| &report.functions[idx].file_path)
        .collect::<FxHashSet<_>>()
        .len();

    let similarity = if report.pairs.is_empty() {
        None
    } else {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut sum = 0.0;
        for pair in &report.pairs {
            min = min.min(pair.similarity);
            max = max.max(pair.similarity);
            sum += pair.similarity;
        }
        Some(SimilarityStats { min, max, avg: sum / report.pairs.len() as f64 })
    };

    let mut exact = 0;
    let mut near = 0;
    let mut similar = 0;
    for pair in &report.pairs {
        if (pair.similarity - 1.0).abs() < f64::EPSILON {
            exact += 1;
        } else if pair.similarity >= 0.9 {
            near += 1;
        } else {
            similar += 1;
        }
    }

    let tiers = TierBreakdown {
        exact: report.pairs.iter().filter(|p| p.tier == Tier::Exact).count(),
        similar: report.pairs.iter().filter(|p| p.tier == Tier::Similar).count(),
    };

    ScanStats {
        files_scanned: report.files_scanned,
        functions_extracted: report.functions.len(),
        clone_pairs: report.pairs.len(),
        containment_findings: report.containments.len(),
        clone_clusters: clusters.len(),
        functions_in_clones: involved.len(),
        files_with_clones,
        similarity,
        breakdown: SimilarityBreakdown { exact, near, similar },
        tiers,
    }
}

/// Format scan statistics as human-readable text.
pub fn format_stats_text(stats: &ScanStats) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Scan statistics:");
    let _ = writeln!(out, "  Files scanned:        {}", stats.files_scanned);
    let _ = writeln!(out, "  Functions extracted:   {}", stats.functions_extracted);
    let _ = writeln!(out, "  Clone pairs:          {}", stats.clone_pairs);
    let _ = writeln!(out, "  Clone clusters:       {}", stats.clone_clusters);
    let _ = writeln!(out, "  Containment findings: {}", stats.containment_findings);
    let _ = writeln!(
        out,
        "  Functions in clones:  {} ({:.1}%)",
        stats.functions_in_clones,
        if stats.functions_extracted == 0 {
            0.0
        } else {
            stats.functions_in_clones as f64 / stats.functions_extracted as f64 * 100.0
        }
    );
    let _ = writeln!(out, "  Files with clones:    {}", stats.files_with_clones);

    if let Some(ref sim) = stats.similarity {
        let _ = writeln!(out);
        let _ = writeln!(out, "Similarity:");
        let _ = writeln!(out, "  Min: {:.2}", sim.min);
        let _ = writeln!(out, "  Max: {:.2}", sim.max);
        let _ = writeln!(out, "  Avg: {:.2}", sim.avg);
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "Breakdown:");
    let _ = writeln!(out, "  Exact clones (1.0):       {}", stats.breakdown.exact);
    let _ = writeln!(out, "  Near clones (0.9-1.0):    {}", stats.breakdown.near);
    let _ = writeln!(out, "  Similar (<0.9):           {}", stats.breakdown.similar);

    let _ = writeln!(out);
    let _ = writeln!(out, "Clone pairs by tier:");
    let _ = writeln!(out, "  exact:                    {}", stats.tiers.exact);
    let _ = writeln!(out, "  similar:                  {}", stats.tiers.similar);
    out
}

/// Format scan statistics as JSON.
pub fn format_stats_json(stats: &ScanStats) -> anyhow::Result<String> {
    serde_json::to_string_pretty(stats).context("failed to serialize stats JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suppress::SuppressionStats;
    use crate::testing::{fragment, pair};

    #[test]
    fn stats_empty_report() {
        let report = CloneReport {
            files_scanned: 0,
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        assert_eq!(stats.files_scanned, 0);
        assert_eq!(stats.functions_extracted, 0);
        assert_eq!(stats.clone_pairs, 0);
        assert_eq!(stats.clone_clusters, 0);
        assert_eq!(stats.functions_in_clones, 0);
        assert_eq!(stats.files_with_clones, 0);
        assert!(stats.similarity.is_none());
        assert_eq!(stats.breakdown.exact, 0);
        assert_eq!(stats.breakdown.near, 0);
        assert_eq!(stats.breakdown.similar, 0);
    }

    #[test]
    fn stats_single_exact_pair() {
        let report = CloneReport {
            files_scanned: 2,
            functions: vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 1.0)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        assert_eq!(stats.files_scanned, 2);
        assert_eq!(stats.functions_extracted, 2);
        assert_eq!(stats.clone_pairs, 1);
        assert_eq!(stats.clone_clusters, 1);
        assert_eq!(stats.functions_in_clones, 2);
        assert_eq!(stats.files_with_clones, 2);
        let sim = stats.similarity.as_ref().expect("should have similarity");
        assert!((sim.min - 1.0).abs() < f64::EPSILON);
        assert!((sim.max - 1.0).abs() < f64::EPSILON);
        assert!((sim.avg - 1.0).abs() < f64::EPSILON);
        assert_eq!(stats.breakdown.exact, 1);
        assert_eq!(stats.breakdown.near, 0);
        assert_eq!(stats.breakdown.similar, 0);
    }

    #[test]
    fn stats_mixed_similarities() {
        let report = CloneReport {
            files_scanned: 3,
            functions: vec![
                fragment("a", "x.py", 0, 10),
                fragment("b", "x.py", 20, 30),
                fragment("c", "y.py", 0, 10),
                fragment("d", "z.py", 0, 10),
            ],
            normalized: vec![],
            pairs: vec![
                pair(0, 1, 1.0),  // exact
                pair(0, 2, 0.95), // near
                pair(2, 3, 0.75), // similar
            ],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        assert_eq!(stats.functions_extracted, 4);
        assert_eq!(stats.clone_pairs, 3);
        assert_eq!(stats.functions_in_clones, 4);
        // x.py, y.py, z.py => 3 files with clones
        assert_eq!(stats.files_with_clones, 3);

        let sim = stats.similarity.as_ref().expect("should have similarity");
        assert!((sim.min - 0.75).abs() < f64::EPSILON);
        assert!((sim.max - 1.0).abs() < f64::EPSILON);
        let expected_avg = (1.0 + 0.95 + 0.75) / 3.0;
        assert!((sim.avg - expected_avg).abs() < f64::EPSILON);

        assert_eq!(stats.breakdown.exact, 1);
        assert_eq!(stats.breakdown.near, 1);
        assert_eq!(stats.breakdown.similar, 1);
    }

    #[test]
    fn stats_same_file_clones() {
        let report = CloneReport {
            files_scanned: 1,
            functions: vec![fragment("a", "same.py", 0, 10), fragment("b", "same.py", 20, 30)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.85)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        assert_eq!(stats.files_with_clones, 1);
        assert_eq!(stats.functions_in_clones, 2);
    }

    #[test]
    fn stats_functions_not_in_clones() {
        let report = CloneReport {
            files_scanned: 3,
            functions: vec![
                fragment("a", "a.py", 0, 10),
                fragment("b", "b.py", 0, 10),
                fragment("c", "c.py", 0, 10),
            ],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.9)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        assert_eq!(stats.functions_extracted, 3);
        assert_eq!(stats.functions_in_clones, 2);
        assert_eq!(stats.files_with_clones, 2);
    }

    // --- Formatter tests ---

    #[test]
    fn text_format_includes_key_stats() {
        let report = CloneReport {
            files_scanned: 5,
            functions: vec![fragment("a", "a.py", 0, 10), fragment("b", "b.py", 0, 10)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        let text = format_stats_text(&stats);
        assert!(text.contains("Files scanned:        5"));
        assert!(text.contains("Functions extracted:   2"));
        assert!(text.contains("Clone pairs:          1"));
        assert!(text.contains("Exact clones"));
        assert!(text.contains("Avg: 0.95"));
    }

    #[test]
    fn text_format_no_clones_shows_zero() {
        let report = CloneReport {
            files_scanned: 3,
            functions: vec![fragment("a", "a.py", 0, 10), fragment("b", "b.py", 0, 10)],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        let text = format_stats_text(&stats);
        assert!(text.contains("Clone pairs:          0"));
        assert!(text.contains("Clone clusters:       0"));
        assert!(!text.contains("Avg:"), "should not show similarity section when no pairs");
    }

    #[test]
    fn json_format_valid() {
        let report = CloneReport {
            files_scanned: 2,
            functions: vec![fragment("a", "a.py", 0, 10), fragment("b", "b.py", 0, 10)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.9)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        let json = format_stats_json(&stats).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["files_scanned"], 2);
        assert_eq!(parsed["functions_extracted"], 2);
        assert_eq!(parsed["clone_pairs"], 1);
        assert!(parsed["similarity"]["avg"].is_f64());
    }

    #[test]
    fn json_format_no_pairs_null_similarity() {
        let report = CloneReport {
            files_scanned: 1,
            functions: vec![fragment("a", "a.py", 0, 10)],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        let json = format_stats_json(&stats).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert!(parsed["similarity"].is_null());
    }

    #[test]
    fn clone_percentage_computed_correctly() {
        let report = CloneReport {
            files_scanned: 2,
            functions: vec![
                fragment("a", "a.py", 0, 10),
                fragment("b", "b.py", 0, 10),
                fragment("c", "c.py", 0, 10),
                fragment("d", "d.py", 0, 10),
            ],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.9)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let stats = compute_stats(&report);
        let text = format_stats_text(&stats);
        // 2 out of 4 functions = 50.0%
        assert!(text.contains("50.0%"), "text was: {text}");
    }
}
