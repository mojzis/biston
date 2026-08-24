# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Acceptance is now two size-aware tiers, `exact` and `similar`
  (behavior-changing under default config).** A single `min_lines` plus a single
  `threshold` could not express a sensible policy: a short *exact* duplicate is
  strong evidence and worth reporting, while a short *fuzzy* one is Jaccard over a
  handful of subtrees — a coarse, jumpy statistic that says almost nothing. The
  required evidence now scales inversely with the strength of the match.

  A whole-function pair is reported when **either**:

  | Tier | Accepted when |
  |---|---|
  | `exact` | the normalized trees hash identically, the shorter function has ≥ `scan.exact_min_lines` (5) executable lines, and both bodies have ≥ `scan.exact_min_stmts` (3) statements |
  | `similar` | similarity ≥ `scan.threshold` (0.85) and the shorter function has ≥ `scan.similar_min_lines` (9) executable lines |

  A contained run is reported when **either**:

  | Tier | Accepted when |
  |---|---|
  | `exact` | the run's fingerprint and the contained function's are identical, and the run spans ≥ `containment.exact_min_fragment_lines` (10) executable lines |
  | `similar` | the containment coefficient ≥ `containment.threshold` (0.85) and the run spans ≥ `containment.similar_min_fragment_lines` (15) executable lines |

  Every existing containment guard — size balance, minimum ratio, maximum run
  fraction, leading/trailing-only, not-lexically-nested — is unchanged and composes
  with the tiers.

  **Users with `min_lines` unset get new behavior.** Exact matches are now reported
  down to five executable lines, where the old floor was ten lines. On CPython's
  `Lib/` (699 files, ~355K lines, tests excluded) the default configuration moves
  from 141 reported pairs to 60 — 49 accepted as `exact`, 11 as `similar` — because
  what the tiers add at the short end is more than offset by what they and the new
  threshold remove at the fuzzy end. Two changes drive that:

  - the fuzzy tier now needs nine *executable* lines, which drops pairs that used to
    clear a ten-line floor on comments, docstrings and blank lines;
  - `scan.threshold` moves from `0.7` to `0.85`.

  A CI gate tuned against the old counts will see a different number. To keep the
  old shape of the policy, set `min_lines = 10` and `threshold = 0.7` — but note that
  even under the alias the floor is now measured in executable lines.

  The full before/after comparison — indexed functions, LSH candidate pairs, runtime,
  reported pairs by tier, and a read of every newly reported short exact pair — is in
  [`bench/results-size-aware-tiers.md`](bench/results-size-aware-tiers.md).

- **Size floors are measured in executable lines.** An executable line is a distinct
  source line holding at least one token that survives AST normalization; comments,
  docstrings and blank lines never count, two statements on one line are one line,
  and a decorator is not part of what is measured. A function padded to twenty lines
  with prose no longer clears a nine-line floor. The definition lives in
  `src/measure.rs` and every gate calls it — there is no raw `line_range` arithmetic
  left at any acceptance site.

- **`scan.threshold` default is now `0.85`** (was `0.7`), matching the containment
  threshold. LSH band parameters are unchanged by the move.

- **Extraction keeps every function with at least `min(exact_min_lines,
  similar_min_lines)` executable lines**, so the exact tier can see the short
  duplicates it exists to report. The indexed population on the corpus above grows
  from 6,622 functions to 8,622 (+30%); LSH candidate pairs are effectively
  unchanged (3,164 → 3,191, +0.9%), because the same change *removes* the prose-heavy
  functions whose outline-shaped fingerprints collided with everything. Runtime on a
  100K-line corpus grows 12–15%, inside the 15% budget set for this change.

### Added

- **Every reported finding names the tier that accepted it.** `text` prints it in
  the cluster header and the containment detail line, `json` carries a `tier` field
  on every cluster and containment, and `sarif` puts it in the message and in
  `properties.tier`. `stats` gains an `Accepted by tier` breakdown, which answers a
  different question from the existing breakdown by score: a pair can score 1.0 and
  still be a `similar`-tier finding, when it cleared the fuzzy rule rather than the
  exact one.

- **JSON schema version 3.** Version 2 added `containments`; version 3 adds `tier`
  to every cluster and every containment. A cluster's tier is the weakest among its
  pairs, the same reading as its `similarity`.

- **New config keys and matching CLI flags**: `scan.exact_min_lines`,
  `scan.similar_min_lines`, `scan.exact_min_stmts`,
  `containment.exact_min_fragment_lines`, `containment.similar_min_fragment_lines`.
  `min_lines` and `min_fragment_lines` are **retained aliases**, not deprecated: set
  on its own, each still means one floor for both tiers. Set alongside a tier key,
  the tier keys win and a single warning names both. Contradictory floors
  (`exact_min_lines` above `similar_min_lines`, a floor of zero) are hard errors
  rather than silently reordered.

### Fixed

- **Comments and docstrings now leave no trace in the normalized tree
  (behavior-changing).** Normalization used to replace each comment and docstring
  with an empty placeholder node that stayed in the tree. The placeholders
  perturbed the bottom-up hashes, so two functions that were identical apart from
  an inline comment or a docstring produced different root hashes and were never
  reported as exact clones — they could only be recovered through the near-miss
  path, where they might fall below the threshold or be lost in LSH. Both are now
  dropped outright, at construction, so a function with comments and a docstring
  normalizes to exactly the tree the same function without them produces.

  **This changes scan results.** Pairs previously scored as near-misses — or
  missed entirely — are now exact matches, so a repository will report more
  clones than it did before, at higher similarity. That is the correct behavior,
  but it can flip a CI gate that was tuned against the old counts. Nothing else
  about normalization changed: decorator and type-annotation placeholders,
  docstring detection (still the first statement of a block, skipping leading
  comments), and suppression via `# biston: ignore` are all untouched.

  A body left with no statements at all — `def f(): """doc"""` — is a valid tree
  and stays one. No `pass` is synthesized for it, so it keeps a hash distinct
  from `def f(): pass`, and it remains unreportable for the same reason it was
  before: there is no logic in it to extract.
