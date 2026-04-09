# Phase 2.5 — Near-miss Detection Rework

Replace MinHash/LSH near-miss detection with direct tree comparison using
the anti-unification machinery from Phase 2. The current approach (Jaccard
over bottom-up subtree hashes) has a fundamental cascading problem that
makes it unreliable for real-world near-misses.

Prerequisite: Phase 2 complete (anti-unification produces `TemplateNode`
trees with holes and quality scores).

## Problem

Bottom-up subtree hashing encodes the *entire* subtree at each node. A
single leaf change (one renamed global, one extra statement) invalidates
the hash of every ancestor. For a function with depth d, one change
invalidates ~d subtree hashes. The Jaccard similarity over these hash sets
drops far below what a human would consider "similar":

| Actual structural similarity | Observed Jaccard |
|------------------------------|------------------|
| 95% (one extra line)         | ~0.60–0.75       |
| 90% (one changed branch)     | ~0.40–0.60       |
| 80% (different leaf calls)   | ~0.20–0.30       |

This makes the MinHash/LSH pipeline unreliable — detection probability at
the 0.7 threshold is borderline even for functions that share 90%+ of
their structure. The LSH parameters cannot fix this because the underlying
similarity metric doesn't reflect structural similarity.

## Approach: Anti-unification as the similarity metric

Phase 2 already builds the exact machinery we need: given two
`NormalizedNode` trees, anti-unification walks them in parallel and
computes the shared structure (template) and differences (holes). The
**template coverage score** (`shared_nodes / total_nodes`) is a direct
measure of structural similarity that doesn't suffer from cascading.

The rework replaces the LSH near-miss pipeline with:

```
candidate filtering → anti-unify candidate pairs → score → filter by quality
```

### 2.5.1 Candidate generation (coarse filter)

All-pairs anti-unification is O(n²) — too expensive for large codebases.
We need a cheap filter to prune obviously-unrelated pairs before running
the expensive comparison. Three complementary filters, applied in order:

**Filter 1: Node count ratio.** Skip pairs where the smaller function has
less than 60% the node count of the larger. Two functions with very
different sizes rarely share enough structure to be worth abstracting.
This is O(1) per pair — just compare two integers already stored in
`HashedFunction` (add a `node_count: usize` field).

**Filter 2: Root kind + depth bucket.** Group functions by
`(root_child_kinds, depth_bucket)` where `depth_bucket = depth / 3`.
Functions in different buckets are unlikely to share structure. This
partitions the function set into smaller groups for pairwise comparison.

**Filter 3: Top-k subtree hash overlap.** Use the existing subtree hashes
but only check if the pair shares *any* hashes (intersection > 0), not
Jaccard. Two functions that share zero subtrees cannot be similar. This is
the cheapest set check — just iterate the smaller set and probe the
larger.

After filtering, the surviving candidate pairs are typically <5% of all
pairs for a large codebase, making O(n²) anti-unification tractable.

### 2.5.2 Anti-unification scoring for similarity

For each candidate pair, run anti-unification (already implemented in
Phase 2) and compute the template coverage score:

```rust
fn structural_similarity(a: &NormalizedNode, b: &NormalizedNode) -> f64 {
    let template = antiunify(a, b);
    let shared = count_shared_nodes(&template);
    let total = count_total_nodes(&template);
    shared as f64 / total as f64
}
```

This score directly answers "what fraction of the structure is shared"
without cascading artifacts. A one-line difference in a 30-line function
produces a score of ~0.95, regardless of where the difference is.

Pairs scoring above `config.scan.threshold` become `SimilarPair` entries
in the report.

### 2.5.3 Pipeline integration

The new `find_similar_functions` becomes:

```
1. Exact matches (root hash grouping — unchanged, O(n))
2. Candidate filtering (node count + bucket + hash overlap)
3. Anti-unify surviving pairs → score → threshold filter
```

The anti-unification results (template + holes) are already computed for
near-miss pairs. Store them alongside `SimilarPair` so Phase 2's
rendering step doesn't need to recompute:

```rust
pub struct SimilarPair {
    pub left: usize,
    pub right: usize,
    pub similarity: f64,
    /// Anti-unification template (None for exact matches).
    pub template: Option<TemplateNode>,
}
```

### 2.5.4 Remove MinHash/LSH

Once anti-unification-based scoring is in place:

- Remove `minhash_signature`, `MinHashSignature`, `LshIndex`,
  `lsh_params_for_threshold`, `hash_band` from `similarity.rs`
- Remove `NUM_PERMUTATIONS` constant
- Keep `find_exact_matches` (root hash grouping) — it's fast and correct
- Keep `jaccard_similarity` if useful for diagnostics, otherwise remove
- Keep `subtree_hashes` in `HashedFunction` for the coarse filter
  (intersection > 0 check), but no longer compute MinHash over them

This simplifies `similarity.rs` significantly and removes the
probabilistic detection failure mode.

## What stays the same

- **Exact match detection** (root hash grouping) — this is O(n) and
  reliable, no reason to change it.
- **Bottom-up subtree hashing** — still useful for exact matches and as a
  coarse filter for candidate generation. The hash itself is correct; it's
  only the Jaccard-over-hashes similarity metric that doesn't work for
  near-misses.
- **`sort_commutative`** — still applies to exact match detection.
- **Normalization pipeline** — unchanged.

## Performance considerations

Anti-unification is O(n) per pair where n is tree size (parallel walk).
For a codebase with 1000 functions and 5% candidate survival rate:

- All pairs: 500K
- After node count filter: ~200K
- After bucket filter: ~50K
- After hash overlap filter: ~25K
- Anti-unification: 25K × O(n) where n ≈ 50 nodes avg = 1.25M node comparisons

This is comfortably under 1 second on modern hardware. The coarse filters
are the critical path — if they let too many candidates through,
anti-unification becomes the bottleneck.

For very large codebases (10K+ functions), the bucket filter becomes
essential. Without it, the quadratic blowup dominates.

## Milestones

### "Candidates are cheap"

- [ ] Add `node_count` to `HashedFunction`
- [ ] Implement node count ratio filter
- [ ] Implement root kind + depth bucket filter
- [ ] Implement hash overlap filter (intersection > 0)
- [ ] Test: candidate count is <10% of all pairs on fixtures

### "Anti-unification replaces LSH"

- [ ] Wire anti-unification scoring into `find_similar_functions`
- [ ] Store `TemplateNode` in `SimilarPair`
- [ ] Remove MinHash/LSH code
- [ ] All existing tests pass with new pipeline
- [ ] Near-miss fixture test is deterministic (no probabilistic failures)

### "It's faster"

- [ ] Benchmark: candidate filtering + anti-unification vs. old MinHash/LSH
- [ ] Verify performance targets from Phase 1 still hold
- [ ] **Ship as part of v0.2.x**

## Dependencies

No new crate dependencies. Anti-unification is already in `src/antiunify.rs`
from Phase 2.

## Configuration

No new config fields. The existing `threshold` applies to the
anti-unification coverage score instead of Jaccard similarity. The
semantics change slightly (coverage score is more intuitive than Jaccard
over subtree hashes), but the default 0.7 remains a good starting point.
