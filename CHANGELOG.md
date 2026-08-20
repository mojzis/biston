# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
