//! Detection of one function already implementing part of another.
//!
//! Scope for this phase is deliberately narrow: the contained function must match a
//! **leading or trailing run of top-level statements** in the container's body.
//! Interior and non-contiguous containment are out of scope.
//!
//! # Why fragments are probes, not index entries
//!
//! Only *whole-function ↔ fragment* comparisons are scored. Fragment ↔ fragment
//! comparison is not filtered out after the fact — it is unrepresentable. The index
//! is keyed by [`WholeFnId`], which is constructed only by [`BodyLshIndex::build`]
//! while it walks whole functions; fragments reach the index solely through
//! [`BodyLshIndex::probe`], which takes a signature and returns [`WholeFnId`]s. No
//! fragment is ever stored, so no bucket can ever hold two of them.
//!
//! A useful consequence: the symmetric detector's index is untouched, so its bucket
//! occupancy is unchanged by this feature.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::config::ContainmentConfig;
use crate::hash::{hash_statement_run, is_inert_statement};
use crate::measure::distinct_line_count;
use crate::normalize::NormalizedNode;
use crate::similarity::{
    hash_band, log_index_occupancy, lsh_params_for_threshold, minhash_signature, MinHashSignature,
};
use crate::tier::{accept_run, Tier};

/// Minimum subtree size for a hash to enter a fingerprint.
///
/// Matches the value the whole-function pipeline uses, so run fingerprints and
/// function fingerprints are built to the same resolution.
const MIN_SUBTREE_NODES: usize = 5;

/// Upper bound on run lengths evaluated while refining one candidate's boundary.
///
/// Refinement is exact set arithmetic with no `MinHash`, so it is cheap, but it is
/// still `O(statements × nodes)`. When more lengths are eligible than this, the range
/// is strided rather than truncated, and the striding is logged rather than silent.
const MAX_REFINE_STEPS: usize = 64;

/// Which end of the container's body the run sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FragmentRole {
    /// The leading run of the body.
    Prefix,
    /// The trailing run of the body.
    Suffix,
}

impl FragmentRole {
    /// Human-readable name, used in reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prefix => "prefix",
            Self::Suffix => "suffix",
        }
    }
}

/// One function's body, prepared for containment analysis.
///
/// Built only when containment is enabled; see [`prepare_bodies`].
pub struct FunctionBody<'a> {
    /// Index into the report's function list.
    pub fragment_index: usize,
    /// Top-level statements of the body, in source order.
    pub statements: &'a [NormalizedNode],
    /// Executable lines (0-indexed, ascending) of each statement, parallel to
    /// `statements`.
    ///
    /// Line *numbers* rather than a span: a run's size is the number of distinct
    /// lines its statements occupy, and two statements sharing a line
    /// (`a = 1; b = 2`) must not count that line twice.
    pub statement_lines: &'a [Vec<usize>],
    /// Line count of the whole function.
    pub function_lines: usize,
    /// Fingerprint of the entire body, with run-relative placeholder numbering.
    fingerprint: FxHashSet<u64>,
}

impl<'a> FunctionBody<'a> {
    /// Prepare one function, returning `None` when it cannot take part.
    fn new(
        fragment_index: usize,
        statements: &'a [NormalizedNode],
        statement_lines: &'a [Vec<usize>],
        function_lines: usize,
        sort_commutative: bool,
    ) -> Option<Self> {
        if statements.len() != statement_lines.len() {
            tracing::warn!(
                "statement/line mismatch for fragment {fragment_index}: {} vs {}",
                statements.len(),
                statement_lines.len()
            );
            return None;
        }
        let fingerprint = hash_statement_run(statements, MIN_SUBTREE_NODES, sort_commutative);
        if fingerprint.is_empty() {
            return None;
        }
        Some(Self { fragment_index, statements, statement_lines, function_lines, fingerprint })
    }

    /// The statement range covered by a run of `length` statements in `role`.
    fn range(&self, role: FragmentRole, length: usize) -> std::ops::Range<usize> {
        match role {
            FragmentRole::Prefix => 0..length,
            FragmentRole::Suffix => self.statements.len() - length..self.statements.len(),
        }
    }

    /// Executable line lists of a run's *executable* statements.
    ///
    /// Statements that do nothing — `pass`, `...`, a bare string — are excluded, as
    /// docstrings and comments already are by normalization dropping them entirely.
    /// They contribute nothing to the fingerprint, and counting them towards the size
    /// floor lets a two-line idiom under a sixteen-line docstring clear a fifteen-line
    /// floor — which is how stock boilerplate slips past a guard meant to suppress
    /// exactly that.
    fn run_lines(&self, role: FragmentRole, length: usize) -> impl Iterator<Item = &[usize]> {
        self.range(role, length)
            .filter(|&i| !is_inert_statement(&self.statements[i]))
            .map(|i| self.statement_lines[i].as_slice())
    }

    /// Line span (0-indexed, inclusive) of a run's executable statements.
    ///
    /// `None` when the run holds no executable line at all.
    fn span(&self, role: FragmentRole, length: usize) -> Option<(usize, usize)> {
        let mut lines = self.run_lines(role, length).flatten().copied();
        let first = lines.next()?;
        Some((first, lines.last().unwrap_or(first)))
    }

    /// Executable lines a run spans, in the units the acceptance tiers use.
    fn line_count(&self, role: FragmentRole, length: usize) -> usize {
        distinct_line_count(self.run_lines(role, length))
    }

    /// Number of executable statements in a run.
    fn executable_count(&self, role: FragmentRole, length: usize) -> usize {
        self.range(role, length).filter(|&i| !is_inert_statement(&self.statements[i])).count()
    }

    /// Fingerprint of a run, numbered relative to the run itself.
    fn run_fingerprint(&self, role: FragmentRole, length: usize, sort: bool) -> FxHashSet<u64> {
        hash_statement_run(&self.statements[self.range(role, length)], MIN_SUBTREE_NODES, sort)
    }
}

/// Identifies a whole function inside the containment index.
///
/// There is deliberately no conversion from a fragment into this type, and
/// [`BodyLshIndex`] exposes no `insert`. Together those make it impossible for a
/// fragment to enter the index, so fragment-to-fragment comparison cannot happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct WholeFnId(usize);

/// Banded LSH over *whole-function body* fingerprints.
struct BodyLshIndex {
    bands: usize,
    rows: usize,
    buckets: Vec<FxHashMap<u64, Vec<WholeFnId>>>,
}

impl BodyLshIndex {
    /// Build the index. This is the only way to populate it, and it only ever walks
    /// whole functions.
    fn build(bodies: &[FunctionBody<'_>], threshold: f64) -> Self {
        let (bands, rows) = lsh_params_for_threshold(threshold);
        let mut index =
            Self { bands, rows, buckets: (0..bands).map(|_| FxHashMap::default()).collect() };
        for (position, body) in bodies.iter().enumerate() {
            let signature = minhash_signature(&body.fingerprint);
            for band in 0..index.bands {
                let key = index.band_key(&signature, band);
                index.buckets[band].entry(key).or_default().push(WholeFnId(position));
            }
        }
        index
    }

    fn band_key(&self, signature: &MinHashSignature, band: usize) -> u64 {
        let start = band * self.rows;
        let end = (start + self.rows).min(signature.values.len());
        hash_band(&signature.values[start..end])
    }

    /// Look up whole functions sharing a band with this fragment signature.
    fn probe(&self, signature: &MinHashSignature) -> FxHashSet<WholeFnId> {
        let mut hits = FxHashSet::default();
        for band in 0..self.bands {
            if let Some(bucket) = self.buckets[band].get(&self.band_key(signature, band)) {
                hits.extend(bucket.iter().copied());
            }
        }
        hits
    }

    /// Occupancy of every non-empty bucket, for reporting index pressure.
    fn bucket_sizes(&self) -> Vec<usize> {
        self.buckets.iter().flat_map(|band| band.values().map(Vec::len)).collect()
    }
}

/// One function found to already implement a run of another's body.
#[derive(Debug, Clone)]
pub struct ContainmentFinding {
    /// Fragment index of the function that is already implemented elsewhere.
    pub contained: usize,
    /// Fragment index of the function whose body contains it.
    pub container: usize,
    /// Which end of the container's body the run sits at.
    pub role: FragmentRole,
    /// First line of the run in the container (0-indexed).
    pub start_line: usize,
    /// Last line of the run in the container (0-indexed, inclusive).
    pub end_line: usize,
    /// Number of *executable* top-level statements the run spans.
    ///
    /// Docstrings and comments are excluded.
    pub statement_count: usize,
    /// Containment coefficient, `|A ∩ F| / min(|A|, |F|)`.
    pub score: f64,
    /// The acceptance tier that admitted this finding.
    pub tier: Tier,
}

/// Prepare per-function bodies for analysis.
///
/// `statement_lines` supplies the executable lines of each top-level statement,
/// parallel to the statements of the corresponding normalized function.
#[must_use]
pub fn prepare_bodies<'a>(
    fragment_indices: &[usize],
    statements: &[&'a [NormalizedNode]],
    statement_lines: &[&'a [Vec<usize>]],
    function_lines: &[usize],
    sort_commutative: bool,
) -> Vec<FunctionBody<'a>> {
    debug_assert!(
        fragment_indices.len() == statements.len()
            && statements.len() == statement_lines.len()
            && statement_lines.len() == function_lines.len(),
        "prepare_bodies requires four parallel slices; `zip` would silently truncate"
    );
    fragment_indices
        .iter()
        .zip(statements)
        .zip(statement_lines)
        .zip(function_lines)
        .filter_map(|(((&index, &stmts), &stmt_lines), &lines)| {
            FunctionBody::new(index, stmts, stmt_lines, lines, sort_commutative)
        })
        .collect()
}

/// Find every function that already implements a leading or trailing run of another.
///
/// Results are sorted by score descending, then by container and contained index, so
/// output is deterministic regardless of hash-map iteration order.
#[must_use]
pub fn find_containments(
    bodies: &[FunctionBody<'_>],
    config: &ContainmentConfig,
    sort_commutative: bool,
) -> Vec<ContainmentFinding> {
    if bodies.len() < 2 {
        return vec![];
    }

    let index = BodyLshIndex::build(bodies, config.threshold);
    log_index_occupancy("containment", || index.bucket_sizes());

    // (container position, role, contained position) — container-major so every
    // candidate sharing a host and role lands in one contiguous group, and each
    // group's run fingerprints can be built once rather than once per candidate.
    // Sorting also keeps the nondeterministic probe order out of the output.
    let mut candidates: Vec<(usize, FragmentRole, WholeFnId)> = Vec::new();
    for (container_pos, container) in bodies.iter().enumerate() {
        for role in [FragmentRole::Prefix, FragmentRole::Suffix] {
            for length in probe_ladder(container, role, config) {
                let fingerprint = container.run_fingerprint(role, length, sort_commutative);
                if fingerprint.is_empty() {
                    continue;
                }
                for hit in index.probe(&minhash_signature(&fingerprint)) {
                    if hit.0 != container_pos {
                        candidates.push((container_pos, role, hit));
                    }
                }
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();

    let mut findings = Vec::new();
    for group in candidates.chunk_by(|a, b| a.0 == b.0 && a.1 == b.1) {
        let (container_pos, role, _) = group[0];
        let host = &bodies[container_pos];
        // `run_fingerprint` depends only on (host, role, length) — never on the
        // contained function — so build the sweep once for the whole group.
        let runs = refinement_runs(host, role, config, sort_commutative);
        findings.extend(group.iter().filter_map(|&(_, _, contained)| {
            score_candidate(&bodies[contained.0], host, role, &runs, config)
        }));
    }

    keep_best_per_pair(&mut findings);
    findings.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.container.cmp(&b.container))
            .then_with(|| a.contained.cmp(&b.contained))
    });
    findings
}

/// Run lengths worth probing for one function and role.
///
/// A geometric ladder at eighths of the statement count. Exactness is not required
/// here — this stage only has to make the true run *collide*, and the boundary is
/// then found exactly by [`score_candidate`]. A ladder step is within a factor of
/// 1.14 of any true length, which puts the LSH collision probability near certainty.
fn probe_ladder(
    body: &FunctionBody<'_>,
    role: FragmentRole,
    config: &ContainmentConfig,
) -> Vec<usize> {
    let mut lengths: Vec<usize> = (1..=6)
        .map(|numerator| (body.statements.len() * numerator).div_ceil(8).max(1))
        .filter(|&length| is_eligible_run(body, role, length, config))
        .collect();
    lengths.sort_unstable();
    lengths.dedup();
    lengths.truncate((config.max_probes_per_function / 2).max(1));
    lengths
}

/// Whether a run is long enough to be worth reporting and short enough to be a part.
fn is_eligible_run(
    body: &FunctionBody<'_>,
    role: FragmentRole,
    length: usize,
    config: &ContainmentConfig,
) -> bool {
    let total = body.statements.len();
    if length == 0 || length >= total {
        // A run covering the whole body is the whole function again.
        return false;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "statement counts are far below f64's exact-integer range"
    )]
    let fraction = length as f64 / total as f64;
    if fraction > config.max_run_fraction {
        return false;
    }
    // The *lowest* floor either tier could accept: a run below it can never be
    // reported, and one above it is decided by `accept_run`, not here. Checking the
    // stricter floor here would put a second, older gate in front of the tiers.
    body.line_count(role, length) >= config.candidate_fragment_floor()
}

/// Every run length worth evaluating for one host and role, with its fingerprint.
///
/// Built once per (host, role) rather than per candidate: a run's fingerprint depends
/// only on the statements it spans, never on which function it is compared against.
///
/// When more lengths are eligible than [`MAX_REFINE_STEPS`], the range is **strided**
/// rather than truncated. Truncating kept the shortest runs, so a candidate whose true
/// boundary lay past step 64 could never be refined and was dropped entirely; striding
/// keeps coverage spread across the whole range.
fn refinement_runs(
    host: &FunctionBody<'_>,
    role: FragmentRole,
    config: &ContainmentConfig,
    sort_commutative: bool,
) -> Vec<(usize, FxHashSet<u64>)> {
    let eligible: Vec<usize> = (1..host.statements.len())
        .filter(|&length| is_eligible_run(host, role, length, config))
        .collect();

    let stride = eligible.len().div_ceil(MAX_REFINE_STEPS).max(1);
    if stride > 1 {
        tracing::debug!(
            "containment: striding {} eligible run lengths by {stride} for fragment {}",
            eligible.len(),
            host.fragment_index
        );
    }

    eligible
        .into_iter()
        .step_by(stride)
        .map(|length| (length, host.run_fingerprint(role, length, sort_commutative)))
        .collect()
}

/// Find the exact run boundary for one candidate and apply the reporting guards.
///
/// Returns `None` unless every guard passes.
fn score_candidate(
    contained: &FunctionBody<'_>,
    host: &FunctionBody<'_>,
    role: FragmentRole,
    runs: &[(usize, FxHashSet<u64>)],
    config: &ContainmentConfig,
) -> Option<ContainmentFinding> {
    // Ratio guard: below this the contained function is a detail of a much larger
    // one rather than an abstraction worth naming.
    #[allow(
        clippy::cast_precision_loss,
        reason = "line counts are far below f64's exact-integer range"
    )]
    let ratio = contained.function_lines as f64 / host.function_lines as f64;
    if ratio < config.min_ratio {
        return None;
    }

    let balance_floor = config.size_balance_floor();
    let mut best: Option<(f64, f64, usize, Tier)> = None;
    for (length, fragment) in runs {
        let length = *length;
        let Some((score, balance)) = compare(&contained.fingerprint, fragment) else {
            continue;
        };
        // The size-balance guard is independent of the tiers and composes with them:
        // a run either tier would take is still dropped when it fails here.
        if balance < balance_floor {
            continue;
        }
        // Identical fingerprints of identical size is the strongest statement this
        // stage can make that the run *is* the function, which is what buys the
        // exact tier's lower floor.
        let exact = (score - 1.0).abs() < f64::EPSILON && (balance - 1.0).abs() < f64::EPSILON;
        let Some(tier) = accept_run(score, exact, host.line_count(role, length), config) else {
            continue;
        };
        // Prefer the highest containment, then the most balanced, then the shortest
        // run — the shortest is the tightest description of what was duplicated.
        let better = best.is_none_or(|(best_score, best_balance, _, _)| {
            (score, balance) > (best_score, best_balance)
        });
        if better {
            best = Some((score, balance, length, tier));
        }
    }

    let (score, _, length, tier) = best?;
    // `is_eligible_run` already rejected runs with no executable statement.
    let (start_line, end_line) = host.span(role, length)?;
    Some(ContainmentFinding {
        contained: contained.fragment_index,
        container: host.fragment_index,
        role,
        start_line,
        end_line,
        statement_count: host.executable_count(role, length),
        score,
        tier,
    })
}

/// Containment coefficient and size balance between a function body and a run.
///
/// `None` when either side is empty, which carries no evidence either way.
fn compare(function: &FxHashSet<u64>, fragment: &FxHashSet<u64>) -> Option<(f64, f64)> {
    if function.is_empty() || fragment.is_empty() {
        return None;
    }
    let shared = function.intersection(fragment).count();
    let smaller = function.len().min(fragment.len());
    let larger = function.len().max(fragment.len());
    #[allow(
        clippy::cast_precision_loss,
        reason = "fingerprint sizes are far below f64's exact-integer range"
    )]
    let result = (shared as f64 / smaller as f64, smaller as f64 / larger as f64);
    Some(result)
}

/// Keep only the strongest finding for each *unordered* pair of functions.
///
/// The same pair can surface under both roles when a body is short enough that its
/// leading and trailing runs overlap, and under both *directions* when the two
/// functions are near-identical — each is then a run of the other. Either way there
/// is one duplication to act on, so it should be stated once, in the direction the
/// evidence supports best.
fn keep_best_per_pair(findings: &mut Vec<ContainmentFinding>) {
    let mut best: FxHashMap<(usize, usize), usize> = FxHashMap::default();
    for (position, finding) in findings.iter().enumerate() {
        let key = if finding.contained < finding.container {
            (finding.contained, finding.container)
        } else {
            (finding.container, finding.contained)
        };
        match best.get(&key) {
            Some(&existing) if findings[existing].score >= finding.score => {}
            _ => {
                best.insert(key, position);
            }
        }
    }
    let mut keep: Vec<usize> = best.into_values().collect();
    keep.sort_unstable();
    let mut kept = Vec::with_capacity(keep.len());
    for (position, finding) in findings.drain(..).enumerate() {
        if keep.binary_search(&position).is_ok() {
            kept.push(finding);
        }
    }
    *findings = kept;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_rejects_empty_sides() {
        let empty = FxHashSet::default();
        let full: FxHashSet<u64> = (0..10).collect();
        assert!(compare(&empty, &full).is_none());
        assert!(compare(&full, &empty).is_none());
        assert!(compare(&empty, &empty).is_none());
    }

    #[test]
    fn compare_scores_a_strict_subset_as_full_containment_but_unbalanced() {
        let function: FxHashSet<u64> = (0..10).collect();
        let fragment: FxHashSet<u64> = (0..40).collect();
        let (score, balance) = compare(&function, &fragment).unwrap();
        assert!((score - 1.0).abs() < f64::EPSILON, "subset should score 1.0, got {score}");
        assert!((balance - 0.25).abs() < f64::EPSILON, "balance should be 10/40, got {balance}");
    }

    #[test]
    fn compare_scores_identical_sets_perfectly() {
        let set: FxHashSet<u64> = (0..25).collect();
        let (score, balance) = compare(&set, &set).unwrap();
        assert!((score - 1.0).abs() < f64::EPSILON);
        assert!((balance - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn size_balance_floor_is_the_reciprocal() {
        let config = ContainmentConfig { size_balance: 1.25, ..ContainmentConfig::default() };
        assert!((config.size_balance_floor() - 0.8).abs() < 1e-12);
    }

    #[test]
    fn size_balance_floor_of_zero_disables_the_guard() {
        let config = ContainmentConfig { size_balance: 0.0, ..ContainmentConfig::default() };
        assert!(config.size_balance_floor().abs() < f64::EPSILON);
    }

    #[test]
    fn ladder_covers_eighths_and_respects_the_probe_cap() {
        // 16 executable statements, no line-length floor in the way.
        let statements: Vec<NormalizedNode> = (0..16)
            .map(|_| NormalizedNode {
                kind: "return_statement",
                text: None,
                children: vec![],
                byte_range: None,
            })
            .collect();
        let lines: Vec<Vec<usize>> = (0..16).map(|i| (i * 40..i * 40 + 40).collect()).collect();
        let body = FunctionBody {
            fragment_index: 0,
            statements: &statements,
            statement_lines: &lines,
            function_lines: 640,
            fingerprint: (0..5).collect(),
        };
        let config =
            ContainmentConfig { min_fragment_lines: Some(1), ..ContainmentConfig::default() };
        let ladder = probe_ladder(&body, FragmentRole::Prefix, &config);
        assert_eq!(ladder, vec![2, 4, 6, 8, 10, 12], "eighths of 16");
        assert!(ladder.len() <= config.max_probes_per_function / 2);
    }

    #[test]
    fn ladder_excludes_runs_covering_the_whole_body() {
        let statements: Vec<NormalizedNode> = (0..4)
            .map(|_| NormalizedNode {
                kind: "return_statement",
                text: None,
                children: vec![],
                byte_range: None,
            })
            .collect();
        let lines: Vec<Vec<usize>> = (0..4).map(|i| (i * 40..i * 40 + 40).collect()).collect();
        let body = FunctionBody {
            fragment_index: 0,
            statements: &statements,
            statement_lines: &lines,
            function_lines: 160,
            fingerprint: (0..5).collect(),
        };
        let config =
            ContainmentConfig { min_fragment_lines: Some(1), ..ContainmentConfig::default() };
        let ladder = probe_ladder(&body, FragmentRole::Prefix, &config);
        assert!(!ladder.contains(&4), "a 4-of-4 run is the whole body: {ladder:?}");
        assert!(ladder.iter().all(|&k| k < 4));
    }

    #[test]
    fn keep_best_per_pair_retains_only_the_strongest() {
        let make = |contained, container, score, role| ContainmentFinding {
            contained,
            container,
            role,
            start_line: 0,
            end_line: 10,
            statement_count: 3,
            score,
            tier: Tier::Similar,
        };
        let mut findings = vec![
            make(0, 1, 0.9, FragmentRole::Prefix),
            make(0, 1, 0.95, FragmentRole::Suffix),
            make(2, 3, 0.88, FragmentRole::Prefix),
        ];
        keep_best_per_pair(&mut findings);
        assert_eq!(findings.len(), 2);
        let pair = findings.iter().find(|f| (f.contained, f.container) == (0, 1)).unwrap();
        assert!((pair.score - 0.95).abs() < f64::EPSILON);
        assert_eq!(pair.role, FragmentRole::Suffix);
    }

    #[test]
    fn keep_best_per_pair_states_a_mutual_finding_once() {
        // Near-identical functions are each a run of the other. That is one
        // duplication, and it should be reported once, in the better-supported
        // direction — not twice, pointing both ways.
        let make = |contained, container, score| ContainmentFinding {
            contained,
            container,
            role: FragmentRole::Prefix,
            start_line: 0,
            end_line: 10,
            statement_count: 3,
            score,
            tier: Tier::Similar,
        };
        let mut findings = vec![make(4, 7, 0.88), make(7, 4, 0.93)];
        keep_best_per_pair(&mut findings);
        assert_eq!(findings.len(), 1, "got {findings:?}");
        assert_eq!((findings[0].contained, findings[0].container), (7, 4));
        assert!((findings[0].score - 0.93).abs() < f64::EPSILON);
    }
}
