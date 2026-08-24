//! Which tier accepted a finding, and the rules that decide it.
//!
//! Acceptance is two step functions, not a curve. Required evidence scales
//! inversely with the strength of the match:
//!
//! | Tier | Whole-function pair | Contained run |
//! |---|---|---|
//! | `exact` | identical normalized tree, shorter side ≥ `scan.exact_min_lines` executable lines, both bodies ≥ `scan.exact_min_stmts` statements | identical fingerprint, run ≥ `containment.exact_min_fragment_lines` executable lines |
//! | `similar` | similarity ≥ `scan.threshold`, shorter side ≥ `scan.similar_min_lines` executable lines | containment ≥ `containment.threshold`, run ≥ `containment.similar_min_fragment_lines` executable lines |
//!
//! A pair is reported when *either* row admits it. The asymmetry is the point: an
//! exact match of the normalized tree is strong evidence even over few lines, while
//! Jaccard over a handful of subtrees is coarse and jumpy, so a fuzzy match has to
//! bring more lines with it before it is worth a reader's attention.
//!
//! No continuous size/similarity tradeoff is offered, deliberately: a formula is
//! neither explainable to the reader of a report nor tunable by the owner of a
//! repository, and two labelled steps are both.

use serde::Serialize;

use crate::config::{ContainmentConfig, ScanConfig};
use crate::measure::FragmentSize;

/// The tier that accepted a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Structurally identical after normalization.
    Exact,
    /// Similar enough, over a large enough fragment.
    Similar,
}

impl Tier {
    /// Human-readable name, used in every output format.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Similar => "similar",
        }
    }

    /// The weaker of two tiers.
    ///
    /// Used where several findings are reported as one item — a cluster is only an
    /// exact-tier cluster if every pair in it is, the same way its similarity is the
    /// weakest pairwise similarity rather than the strongest.
    #[must_use]
    pub fn weaker(self, other: Self) -> Self {
        if self == Self::Similar || other == Self::Similar {
            Self::Similar
        } else {
            Self::Exact
        }
    }
}

/// Which tier accepts this whole-function pair, if either does.
///
/// `exact_match` is whether the two normalized trees hash identically. The size
/// gates read the *shorter* side: a pair is only as well-evidenced as its smaller
/// half, and reading the larger one would let a 200-line function drag a 3-line one
/// into a report.
#[must_use]
pub fn accept_pair(
    similarity: f64,
    exact_match: bool,
    left: FragmentSize,
    right: FragmentSize,
    config: &ScanConfig,
) -> Option<Tier> {
    let lines = left.executable_lines.min(right.executable_lines);
    if exact_match
        && lines >= config.exact_line_floor()
        && left.executable_stmts.min(right.executable_stmts) >= config.exact_min_stmts
    {
        return Some(Tier::Exact);
    }
    if similarity >= config.threshold && lines >= config.similar_line_floor() {
        return Some(Tier::Similar);
    }
    None
}

/// Which tier accepts this contained run, if either does.
///
/// `exact_match` is whether the run's fingerprint and the contained function's are
/// the same set — the strongest statement the containment stage can make about two
/// pieces of code being the same. `fragment_lines` is the run's executable lines.
///
/// This decides the size/strength tradeoff only. Every other containment guard —
/// size balance, minimum ratio, maximum run fraction — is applied by the caller and
/// composes with whatever this returns: a run both tiers would take is still dropped
/// if it fails one of them.
#[must_use]
pub fn accept_run(
    score: f64,
    exact_match: bool,
    fragment_lines: usize,
    config: &ContainmentConfig,
) -> Option<Tier> {
    if exact_match && fragment_lines >= config.exact_fragment_floor() {
        return Some(Tier::Exact);
    }
    if score >= config.threshold && fragment_lines >= config.similar_fragment_floor() {
        return Some(Tier::Similar);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(lines: usize, stmts: usize) -> FragmentSize {
        FragmentSize { executable_lines: lines, executable_stmts: stmts }
    }

    /// The shipped defaults: exact ≥ 5 lines and ≥ 3 statements, fuzzy ≥ 9 lines.
    fn scan_config() -> ScanConfig {
        ScanConfig::default()
    }

    #[test]
    fn a_short_exact_match_is_accepted_by_the_exact_tier() {
        let tier = accept_pair(1.0, true, size(6, 4), size(6, 4), &scan_config());
        assert_eq!(tier, Some(Tier::Exact));
    }

    #[test]
    fn an_exact_match_below_the_line_floor_is_rejected() {
        assert_eq!(accept_pair(1.0, true, size(3, 3), size(9, 5), &scan_config()), None);
    }

    #[test]
    fn an_exact_match_below_the_statement_floor_is_rejected() {
        // A docstring and a delegating `return`: identical, and not duplication
        // anyone can act on.
        assert_eq!(accept_pair(1.0, true, size(6, 1), size(6, 1), &scan_config()), None);
    }

    #[test]
    fn the_statement_floor_reads_the_smaller_body() {
        // Six executable lines is below the fuzzy floor, so the exact tier is the
        // only way in — and it reads the *smaller* statement count: one substantial
        // body does not vouch for a one-idiom one.
        assert_eq!(accept_pair(1.0, true, size(6, 2), size(6, 8), &scan_config()), None);
        assert_eq!(
            accept_pair(1.0, true, size(6, 3), size(6, 8), &scan_config()),
            Some(Tier::Exact),
            "both bodies clearing the floor is what the guard asks for"
        );
    }

    #[test]
    fn a_long_exact_match_failing_the_statement_guard_still_reaches_the_fuzzy_tier() {
        // Similarity 1.0 clears any threshold, so the second rule admits it on size
        // alone. The tier records that it was accepted on the weaker evidence.
        let tier = accept_pair(1.0, true, size(12, 2), size(12, 2), &scan_config());
        assert_eq!(tier, Some(Tier::Similar));
    }

    #[test]
    fn a_short_fuzzy_match_is_rejected_however_high_it_scores() {
        assert_eq!(
            accept_pair(0.99, false, size(6, 5), size(6, 5), &scan_config()),
            None,
            "the fuzzy tier requires 9 executable lines"
        );
    }

    #[test]
    fn a_long_fuzzy_match_above_the_threshold_is_accepted() {
        assert_eq!(
            accept_pair(0.88, false, size(12, 6), size(14, 7), &scan_config()),
            Some(Tier::Similar)
        );
    }

    #[test]
    fn a_long_fuzzy_match_below_the_threshold_is_rejected() {
        assert_eq!(accept_pair(0.80, false, size(30, 12), size(30, 12), &scan_config()), None);
    }

    #[test]
    fn the_line_floor_reads_the_shorter_function() {
        assert_eq!(
            accept_pair(0.9, false, size(4, 3), size(80, 40), &scan_config()),
            None,
            "a large function must not vouch for a tiny one"
        );
    }

    #[test]
    fn the_alias_collapses_the_two_tiers_onto_one_floor() {
        let config = ScanConfig { min_lines: Some(10), ..ScanConfig::default() };
        assert_eq!(accept_pair(1.0, true, size(6, 4), size(6, 4), &config), None);
        assert_eq!(accept_pair(1.0, true, size(10, 4), size(10, 4), &config), Some(Tier::Exact));
    }

    // --- Contained runs ---

    fn containment_config() -> ContainmentConfig {
        ContainmentConfig::default()
    }

    #[test]
    fn an_exact_run_clears_the_lower_fragment_floor() {
        assert_eq!(accept_run(1.0, true, 11, &containment_config()), Some(Tier::Exact));
    }

    #[test]
    fn an_exact_run_below_the_exact_fragment_floor_is_rejected() {
        assert_eq!(accept_run(1.0, true, 9, &containment_config()), None);
    }

    #[test]
    fn a_fuzzy_run_between_the_two_floors_is_rejected() {
        assert_eq!(
            accept_run(0.86, false, 12, &containment_config()),
            None,
            "a fuzzy containment needs 15 executable lines"
        );
    }

    #[test]
    fn a_long_fuzzy_run_above_the_threshold_is_accepted() {
        assert_eq!(accept_run(0.86, false, 16, &containment_config()), Some(Tier::Similar));
    }

    #[test]
    fn a_long_run_below_the_containment_threshold_is_rejected() {
        assert_eq!(accept_run(0.5, false, 40, &containment_config()), None);
    }

    // --- Aggregation ---

    #[test]
    fn a_cluster_is_exact_only_when_every_pair_is() {
        assert_eq!(Tier::Exact.weaker(Tier::Exact), Tier::Exact);
        assert_eq!(Tier::Exact.weaker(Tier::Similar), Tier::Similar);
        assert_eq!(Tier::Similar.weaker(Tier::Exact), Tier::Similar);
    }

    #[test]
    fn tier_names_are_stable_for_consumers() {
        assert_eq!(Tier::Exact.as_str(), "exact");
        assert_eq!(Tier::Similar.as_str(), "similar");
        assert_eq!(serde_json::to_string(&Tier::Exact).unwrap(), "\"exact\"");
        assert_eq!(serde_json::to_string(&Tier::Similar).unwrap(), "\"similar\"");
    }
}
