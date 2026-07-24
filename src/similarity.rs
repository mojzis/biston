use rustc_hash::{FxHashMap, FxHashSet};

use crate::hash::HashedFunction;

/// Number of minhash permutations.
const NUM_PERMUTATIONS: usize = 128;

/// A pair of similar functions.
#[derive(Debug, Clone)]
pub struct SimilarPair {
    /// Index of the first function in the fragment list.
    pub left: usize,
    /// Index of the second function in the fragment list.
    pub right: usize,
    /// Similarity score (1.0 for exact matches).
    pub similarity: f64,
}

/// Find all similar function pairs above the given threshold.
///
/// Combines exact matching (root hash) and near-miss detection (minhash + LSH).
pub fn find_similar_functions(functions: &[HashedFunction], threshold: f64) -> Vec<SimilarPair> {
    if functions.is_empty() {
        return vec![];
    }

    // Two independent degeneracy filters, applied before *either* phase.
    //
    // `has_executable_body`: a body of only docstrings, `pass`, `...` or comments
    // normalizes to the same tree as every other such body, so exact root-hash
    // matching pairs them all at 1.0. On CPython's `Lib/` that collapses 119
    // unrelated doctest functions into one cluster — 7021 of 14389 reported pairs.
    //
    // `!subtree_hashes.is_empty()`: `minhash_signature` maps the empty set to
    // all-`u64::MAX`, which collides in *every* band, so each such function would
    // become a candidate against every other one — quadratic work for pairs with
    // nothing to compare.
    let reportable: Vec<&HashedFunction> = functions
        .iter()
        .filter(|f| f.has_executable_body && !f.subtree_hashes.is_empty())
        .collect();
    if reportable.is_empty() {
        return vec![];
    }

    // Phase 1: exact matches
    let mut exact_pairs: FxHashSet<(usize, usize)> = FxHashSet::default();
    let mut pairs = find_exact_matches(&reportable);
    for p in &pairs {
        let key = if p.left < p.right { (p.left, p.right) } else { (p.right, p.left) };
        exact_pairs.insert(key);
    }

    // Phase 2: near-miss detection via minhash + LSH
    let (bands, rows) = lsh_params_for_threshold(threshold);
    let mut lsh = LshIndex::new(bands, rows);
    for (i, func) in reportable.iter().enumerate() {
        lsh.insert(i, &minhash_signature(&func.subtree_hashes));
    }

    let candidates = lsh.candidates();
    for (i, j) in candidates {
        let fi = reportable[i].fragment_index;
        let fj = reportable[j].fragment_index;
        let key = if fi < fj { (fi, fj) } else { (fj, fi) };
        if exact_pairs.contains(&key) {
            continue;
        }

        let sim = jaccard_similarity(&reportable[i].subtree_hashes, &reportable[j].subtree_hashes);
        if sim >= threshold {
            pairs.push(SimilarPair { left: key.0, right: key.1, similarity: sim });
        }
    }

    // Sort by similarity desc, then by left index asc for determinism
    pairs.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.left.cmp(&b.left))
            .then_with(|| a.right.cmp(&b.right))
    });

    pairs
}

/// Find exact matches by grouping functions with identical root hashes.
///
/// Takes a slice of references so callers can pre-filter without cloning.
pub(crate) fn find_exact_matches(functions: &[&HashedFunction]) -> Vec<SimilarPair> {
    let mut groups: FxHashMap<u64, Vec<usize>> = FxHashMap::default();

    for func in functions {
        groups.entry(func.root_hash).or_default().push(func.fragment_index);
    }

    let mut pairs = Vec::new();
    for indices in groups.values() {
        if indices.len() < 2 {
            continue;
        }
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                pairs.push(SimilarPair { left: indices[i], right: indices[j], similarity: 1.0 });
            }
        }
    }

    pairs
}

/// A minhash signature for a set of subtree hashes.
#[derive(Debug, Clone)]
pub(crate) struct MinHashSignature {
    pub(crate) values: [u64; NUM_PERMUTATIONS],
}

/// Compute the minhash signature for a set of hashes.
///
/// Uses `NUM_PERMUTATIONS` different hash functions (xxh3 with different seeds).
pub(crate) fn minhash_signature(hashes: &FxHashSet<u64>) -> MinHashSignature {
    let mut values = [u64::MAX; NUM_PERMUTATIONS];

    for &h in hashes {
        let h_bytes = h.to_le_bytes();
        for (i, val) in values.iter_mut().enumerate() {
            let permuted = xxhash_rust::xxh3::xxh3_64_with_seed(&h_bytes, i as u64);
            if permuted < *val {
                *val = permuted;
            }
        }
    }

    MinHashSignature { values }
}

/// LSH index for candidate pair generation.
struct LshIndex {
    bands: usize,
    rows: usize,
    /// One hash map per band: hash -> list of function indices.
    buckets: Vec<FxHashMap<u64, Vec<usize>>>,
}

impl LshIndex {
    fn new(bands: usize, rows: usize) -> Self {
        Self { bands, rows, buckets: (0..bands).map(|_| FxHashMap::default()).collect() }
    }

    fn insert(&mut self, index: usize, signature: &MinHashSignature) {
        for band in 0..self.bands {
            let start = band * self.rows;
            let end = (start + self.rows).min(NUM_PERMUTATIONS);
            let band_hash = hash_band(&signature.values[start..end]);
            self.buckets[band].entry(band_hash).or_default().push(index);
        }
    }

    /// Get all candidate pairs (deduplicated).
    fn candidates(&self) -> FxHashSet<(usize, usize)> {
        let mut pairs = FxHashSet::default();
        for bucket_map in &self.buckets {
            for indices in bucket_map.values() {
                if indices.len() < 2 {
                    continue;
                }
                for i in 0..indices.len() {
                    for j in (i + 1)..indices.len() {
                        let (a, b) = if indices[i] < indices[j] {
                            (indices[i], indices[j])
                        } else {
                            (indices[j], indices[i])
                        };
                        pairs.insert((a, b));
                    }
                }
            }
        }
        pairs
    }
}

/// Hash a band (slice of minhash values) into a single u64.
fn hash_band(values: &[u64]) -> u64 {
    let mut buf = Vec::with_capacity(values.len() * 8);
    for v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    xxhash_rust::xxh3::xxh3_64(&buf)
}

/// Compute exact Jaccard similarity between two sets.
///
/// Two empty fingerprints score `0.0`, not `1.0`. An empty fingerprint means the
/// function had no subtree large enough to qualify, so there is no evidence of
/// similarity to report — scoring it a perfect match turns absence of data into a
/// clone claim.
pub(crate) fn jaccard_similarity(a: &FxHashSet<u64>, b: &FxHashSet<u64>) -> f64 {
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;

    if union == 0 {
        return 0.0;
    }

    intersection as f64 / union as f64
}

/// Compute LSH parameters (bands, rows) for a target threshold.
///
/// We want: `(1/bands)^(1/rows) ≈ threshold`
/// With 128 permutations, bands * rows <= 128.
fn lsh_params_for_threshold(threshold: f64) -> (usize, usize) {
    // For common thresholds, use known-good parameters
    if threshold >= 0.9 {
        (32, 4) // 32 bands × 4 rows = 128
    } else if threshold >= 0.7 {
        (20, 6) // 20 bands × 6 rows = 120 (uses 120 of 128)
    } else if threshold >= 0.5 {
        (16, 8) // 16 bands × 8 rows = 128
    } else {
        (8, 16) // 8 bands × 16 rows = 128
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hashed(index: usize, root_hash: u64) -> HashedFunction {
        HashedFunction {
            fragment_index: index,
            root_hash,
            subtree_hashes: FxHashSet::default(),
            has_executable_body: true,
        }
    }

    fn make_hashed_with_subtrees(index: usize, root_hash: u64, subtrees: &[u64]) -> HashedFunction {
        HashedFunction {
            fragment_index: index,
            root_hash,
            subtree_hashes: subtrees.iter().copied().collect(),
            has_executable_body: true,
        }
    }

    /// A function whose body holds nothing but a docstring, `pass` or comments.
    fn make_inert(index: usize, root_hash: u64, subtrees: &[u64]) -> HashedFunction {
        HashedFunction {
            has_executable_body: false,
            ..make_hashed_with_subtrees(index, root_hash, subtrees)
        }
    }

    // --- Exact match tests ---

    #[test]
    fn two_identical_functions_detected() {
        let funcs = [make_hashed(0, 100), make_hashed(1, 100)];
        let refs: Vec<&HashedFunction> = funcs.iter().collect();
        let pairs = find_exact_matches(&refs);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].left, 0);
        assert_eq!(pairs[0].right, 1);
        assert!((pairs[0].similarity - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn three_identical_functions_detected() {
        let funcs = [make_hashed(0, 100), make_hashed(1, 100), make_hashed(2, 100)];
        let refs: Vec<&HashedFunction> = funcs.iter().collect();
        let pairs = find_exact_matches(&refs);
        assert_eq!(pairs.len(), 3);
    }

    #[test]
    fn no_matches_when_all_unique() {
        let funcs = [make_hashed(0, 100), make_hashed(1, 200), make_hashed(2, 300)];
        let refs: Vec<&HashedFunction> = funcs.iter().collect();
        let pairs = find_exact_matches(&refs);
        assert!(pairs.is_empty());
    }

    #[test]
    fn multiple_groups() {
        let funcs =
            [make_hashed(0, 100), make_hashed(1, 100), make_hashed(2, 200), make_hashed(3, 200)];
        let refs: Vec<&HashedFunction> = funcs.iter().collect();
        let pairs = find_exact_matches(&refs);
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn empty_input() {
        let pairs = find_exact_matches(&[]);
        assert!(pairs.is_empty());
    }

    // --- minhash tests ---

    #[test]
    fn identical_sets_produce_identical_signatures() {
        let set: FxHashSet<u64> = (0..50).collect();
        let sig1 = minhash_signature(&set);
        let sig2 = minhash_signature(&set);
        assert_eq!(sig1.values, sig2.values);
    }

    #[test]
    fn similar_sets_have_many_matching_values() {
        // 80% overlap
        let set_a: FxHashSet<u64> = (0..100).collect();
        let set_b: FxHashSet<u64> = (20..120).collect();
        let sig_a = minhash_signature(&set_a);
        let sig_b = minhash_signature(&set_b);

        let matching = sig_a.values.iter().zip(&sig_b.values).filter(|(a, b)| a == b).count();
        // With 80/120 ≈ 0.667 Jaccard, we expect ~85 of 128 to match (rough)
        // Just check it's substantially more than random
        assert!(matching > 40, "expected many matches, got {matching}");
    }

    #[test]
    fn disjoint_sets_have_few_matching_values() {
        let set_a: FxHashSet<u64> = (0..100).collect();
        let set_b: FxHashSet<u64> = (1000..1100).collect();
        let sig_a = minhash_signature(&set_a);
        let sig_b = minhash_signature(&set_b);

        let matching = sig_a.values.iter().zip(&sig_b.values).filter(|(a, b)| a == b).count();
        assert!(matching < 20, "expected few matches, got {matching}");
    }

    #[test]
    fn empty_set_signature() {
        let set: FxHashSet<u64> = FxHashSet::default();
        let sig = minhash_signature(&set);
        // All values should be u64::MAX (no element to minimize)
        assert!(sig.values.iter().all(|&v| v == u64::MAX));
    }

    // --- LSH tests ---

    #[test]
    fn identical_signatures_are_candidates() {
        let set: FxHashSet<u64> = (0..50).collect();
        let sig = minhash_signature(&set);

        let mut lsh = LshIndex::new(20, 6);
        lsh.insert(0, &sig);
        lsh.insert(1, &sig);

        let candidates = lsh.candidates();
        assert!(candidates.contains(&(0, 1)));
    }

    #[test]
    fn very_different_signatures_not_candidates() {
        let set_a: FxHashSet<u64> = (0..100).collect();
        let set_b: FxHashSet<u64> = (10000..10100).collect();
        let sig_a = minhash_signature(&set_a);
        let sig_b = minhash_signature(&set_b);

        let mut lsh = LshIndex::new(20, 6);
        lsh.insert(0, &sig_a);
        lsh.insert(1, &sig_b);

        let candidates = lsh.candidates();
        // Very unlikely to collide in any band
        assert!(candidates.is_empty(), "unexpected candidates: {candidates:?}");
    }

    #[test]
    fn candidate_pairs_deduplicated() {
        let set: FxHashSet<u64> = (0..50).collect();
        let sig = minhash_signature(&set);

        let mut lsh = LshIndex::new(20, 6);
        lsh.insert(0, &sig);
        lsh.insert(1, &sig);

        let candidates = lsh.candidates();
        // Even though they collide in all bands, the pair appears only once
        assert_eq!(candidates.len(), 1);
    }

    // --- Jaccard tests ---

    #[test]
    fn jaccard_identical_sets() {
        let set: FxHashSet<u64> = (0..10).collect();
        assert!((jaccard_similarity(&set, &set) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_disjoint_sets() {
        let a: FxHashSet<u64> = (0..10).collect();
        let b: FxHashSet<u64> = (10..20).collect();
        assert!(jaccard_similarity(&a, &b).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_partial_overlap() {
        let a: FxHashSet<u64> = (0..10).collect();
        let b: FxHashSet<u64> = (5..15).collect();
        // Intersection: {5..10} = 5, Union: {0..15} = 15
        let expected = 5.0 / 15.0;
        assert!((jaccard_similarity(&a, &b) - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_both_empty_is_zero() {
        // Two functions with no qualifying subtrees share no structural evidence.
        // Scoring that 1.0 turns an absence of data into a perfect-clone claim.
        let a: FxHashSet<u64> = FxHashSet::default();
        let b: FxHashSet<u64> = FxHashSet::default();
        assert!(jaccard_similarity(&a, &b).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_one_empty_is_zero() {
        let a: FxHashSet<u64> = FxHashSet::default();
        let b: FxHashSet<u64> = (0..10).collect();
        assert!(jaccard_similarity(&a, &b).abs() < f64::EPSILON);
        assert!(jaccard_similarity(&b, &a).abs() < f64::EPSILON);
    }

    // --- Degeneracy tests ---

    #[test]
    fn degenerate_fingerprints_are_not_near_miss_pairs() {
        // Distinct root hashes, both fingerprints empty: nothing to compare.
        let funcs =
            vec![make_hashed_with_subtrees(0, 1, &[]), make_hashed_with_subtrees(1, 2, &[])];
        let pairs = find_similar_functions(&funcs, 0.7);
        assert!(pairs.is_empty(), "empty fingerprints must not pair: {pairs:?}");
    }

    #[test]
    fn degenerate_fingerprints_do_not_blow_up_quadratically() {
        // An all-`u64::MAX` signature collides in every band with every other
        // such signature, so N degenerate functions would yield N*(N-1)/2 pairs.
        let funcs: Vec<_> =
            (0..30).map(|i| make_hashed_with_subtrees(i, i as u64 + 1, &[])).collect();
        let pairs = find_similar_functions(&funcs, 0.7);
        assert!(pairs.is_empty(), "expected 0 pairs, got {}", pairs.len());
    }

    #[test]
    fn degenerate_fingerprints_are_not_reported_even_when_root_hashes_match() {
        // Identical root hashes with an empty fingerprint: the exact phase must be
        // filtered too, not only the LSH phase.
        let funcs =
            vec![make_hashed_with_subtrees(0, 100, &[]), make_hashed_with_subtrees(1, 100, &[])];
        let pairs = find_similar_functions(&funcs, 0.7);
        assert!(pairs.is_empty(), "expected 0 pairs, got {pairs:?}");
    }

    #[test]
    fn inert_bodies_are_not_reported_despite_a_matching_fingerprint() {
        // This is the docstring-only shape as it actually occurs: the fingerprint is
        // non-empty (one hash of the function outline) and the root hashes are equal,
        // so both the exact and the near-miss phase would happily pair them.
        let funcs = vec![make_inert(0, 100, &[7]), make_inert(1, 100, &[7])];
        let pairs = find_similar_functions(&funcs, 0.7);
        assert!(pairs.is_empty(), "expected 0 pairs, got {pairs:?}");
    }

    #[test]
    fn inert_body_does_not_drag_in_a_function_with_real_logic() {
        let subtrees: Vec<u64> = (0..40).collect();
        let funcs = vec![
            make_inert(0, 100, &subtrees),
            make_hashed_with_subtrees(1, 100, &subtrees),
            make_hashed_with_subtrees(2, 100, &subtrees),
        ];
        let pairs = find_similar_functions(&funcs, 0.7);
        assert_eq!(pairs.len(), 1, "only the two executable functions may pair: {pairs:?}");
        assert_eq!((pairs[0].left, pairs[0].right), (1, 2));
    }

    #[test]
    fn find_exact_matches_itself_stays_a_pure_grouping_primitive() {
        // The reportability filter lives in `find_similar_functions`, so this
        // low-level helper must still group purely by root hash.
        let funcs = [make_hashed(0, 100), make_hashed(1, 100)];
        let refs: Vec<&HashedFunction> = funcs.iter().collect();
        assert_eq!(find_exact_matches(&refs).len(), 1);
    }

    #[test]
    fn degenerate_fingerprint_does_not_drag_in_a_real_one() {
        let funcs = vec![
            make_hashed_with_subtrees(0, 1, &[]),
            make_hashed_with_subtrees(1, 2, &(0..100).collect::<Vec<_>>()),
        ];
        let pairs = find_similar_functions(&funcs, 0.7);
        assert!(pairs.is_empty(), "expected 0 pairs, got {pairs:?}");
    }

    // --- Full pipeline tests ---

    #[test]
    fn full_pipeline_includes_exact_matches() {
        // Fingerprints must be non-empty: the pipeline drops functions with no
        // qualifying subtree before either phase runs.
        let subtrees: Vec<u64> = (0..20).collect();
        let funcs = vec![
            make_hashed_with_subtrees(0, 100, &subtrees),
            make_hashed_with_subtrees(1, 100, &subtrees),
        ];
        let pairs = find_similar_functions(&funcs, 0.7);
        assert_eq!(pairs.len(), 1);
        assert!((pairs[0].similarity - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn full_pipeline_near_miss_detected() {
        // Two functions with 90% subtree overlap, different root hashes.
        // Jaccard = 90/110 ≈ 0.818, well above threshold.
        let shared: Vec<u64> = (0..90).collect();
        let mut subtrees_a: Vec<u64> = shared.clone();
        subtrees_a.extend(90..100);
        let mut subtrees_b: Vec<u64> = shared;
        subtrees_b.extend(200..210);

        let funcs = vec![
            make_hashed_with_subtrees(0, 1, &subtrees_a),
            make_hashed_with_subtrees(1, 2, &subtrees_b),
        ];

        let pairs = find_similar_functions(&funcs, 0.5);
        assert!(!pairs.is_empty(), "expected near-miss to be detected");
        assert!(pairs[0].similarity > 0.5);
    }

    #[test]
    fn full_pipeline_below_threshold_filtered() {
        // Two functions with very low overlap
        let funcs = vec![
            make_hashed_with_subtrees(0, 1, &(0..100).collect::<Vec<_>>()),
            make_hashed_with_subtrees(1, 2, &(1000..1100).collect::<Vec<_>>()),
        ];

        let pairs = find_similar_functions(&funcs, 0.7);
        assert!(pairs.is_empty());
    }

    #[test]
    fn full_pipeline_empty_input() {
        let pairs = find_similar_functions(&[], 0.7);
        assert!(pairs.is_empty());
    }

    #[test]
    fn full_pipeline_sorted_by_similarity() {
        let funcs = vec![
            make_hashed_with_subtrees(0, 100, &(0..100).collect::<Vec<_>>()),
            make_hashed_with_subtrees(1, 100, &(0..100).collect::<Vec<_>>()),
            make_hashed_with_subtrees(2, 1, &(0..80).chain(200..220).collect::<Vec<_>>()),
            make_hashed_with_subtrees(3, 2, &(0..80).chain(300..320).collect::<Vec<_>>()),
        ];

        let pairs = find_similar_functions(&funcs, 0.5);
        // Check that pairs are sorted by similarity descending
        for window in pairs.windows(2) {
            assert!(window[0].similarity >= window[1].similarity);
        }
    }
}
