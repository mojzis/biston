# Phase 1 — Deviations, Issues, and Notes for Future Phases

Completed 2026-04-10. 75 tests passing, clippy clean (pedantic + nursery).

## Deviations from plan

### Re-parsing every function for normalization

The biggest deviation. `lib.rs::scan()` parses files, extracts functions, then
**re-parses each function's source text individually** for normalization. This
happens because tree-sitter `Node` borrows from `Tree`, and threading those
references through rayon's `par_iter` across ownership boundaries is a lifetime
problem. The plan assumed we'd normalize directly from the original parsed tree.

This works correctly but:
- Does ~N extra parses (one per function)
- Risks subtle differences if a function's source text parses differently in
  isolation vs. in file context (indentation-dependent constructs)

### Function name anonymization

Not in the spec. The plan said to anonymize local identifiers but didn't mention
the function's own name. Without it, `def foo(x): return x` and
`def bar(y): return y` produce different normalized trees. Added `<fn>`
replacement for the function name node.

### `ignore` crate only, no `walkdir`

The plan listed both. `ignore` subsumes `walkdir` (same author, BurntSushi), so
only one dependency was needed.

### `memmap2` deferred entirely

Not even added as a dependency. `std::fs::read` everywhere. The plan anticipated
this as a possibility.

### Decorated function deduplication

The tree-sitter query for both `function_definition` and `decorated_definition`
produces duplicate matches (the inner function matches both patterns). Required
adding a `seen_fn_starts` HashSet to deduplicate by the inner
`function_definition` start byte. Not anticipated in the plan.

### tree-sitter `StreamingIterator` API

tree-sitter 0.25 uses `streaming_iterator::StreamingIterator` for
`QueryMatches`, not std `Iterator`. Required `while let Some(m) = matches.next()`
instead of `for m in matches`. Minor but surprised during implementation.

## Issues to fix before Phase 2

### 1. Re-parsing must be eliminated (critical)

Phase 2 (anti-unification) needs the original AST nodes to compute the maximal
shared structure. If we're working from re-parsed snippets rather than the
original tree, anti-unification results could be subtly wrong.

**Fix:** Restructure the pipeline to process per-file: parse → extract →
normalize+hash all within one scope where the `Tree` is still alive. Collect
`HashedFunction` results, not `FunctionFragment` first.

### 2. `NormalizedNode` is allocation-heavy

Every node allocates a `String` for `kind` (always one of ~50 tree-sitter node
types) and optionally for `text`. Phase 2 will traverse these trees heavily for
structural diff/anti-unification.

**Fix:** Intern `kind` as `&'static str` or a small enum. Use `Cow<'static, str>`
for text.

### 3. `sort_commutative` is a noop

The config field exists and is deserialized, but normalization doesn't implement
it. No code reorders commutative operator children. The plan said "off by
default" so this is not a correctness issue, but it's dead config.

**Fix:** Either implement it in the hash step (sort child hashes before
concatenating at commutative operator nodes) or remove the config field.

## Issues to address (lower priority)

### LSH detection probability is borderline

The initial near-miss test with 80% overlap (Jaccard ≈ 0.667) and threshold 0.5
only had ~48% detection probability with the chosen LSH parameters (16 bands ×
8 rows). Had to bump test to 90% overlap for reliability.

Real-world near-misses in the 0.6–0.7 range may be missed. The
`lsh_params_for_threshold` function uses hardcoded band/row values that could
use empirical tuning.

### No integration tests for the binary

The plan called for `assert_cmd` tests (help, empty dir, known clones, JSON
format). Tested manually but didn't write the automated tests. `assert_cmd` and
`predicates` are already in dev-dependencies.

### `pub` visibility too broad in some modules

The review flagged `find_exact_matches`, `CloneCluster`, and `cluster_pairs` as
`pub` when they're only used internally. Should be `pub(crate)` or private.

## Dependencies added (vs. plan)

| Planned              | Actual                | Notes                          |
|----------------------|-----------------------|--------------------------------|
| tree-sitter 0.25     | tree-sitter 0.25.10   | ✓                              |
| tree-sitter-python    | 0.25.0               | ✓                              |
| xxhash-rust 0.8       | 0.8.15               | ✓ (xxh3 feature)              |
| rustc-hash 2.1        | 2.1.2                | ✓                              |
| rayon 1.10            | 1.11.0               | ✓                              |
| walkdir 2             | not added             | `ignore` subsumes it           |
| memmap2 0.9           | not added             | deferred                       |
| ignore (not planned)  | 0.4.25               | replaces walkdir + gitignore   |
| glob-match (not planned) | 0.2.1             | for include/exclude patterns   |
| toml 0.8              | 0.8.23               | ✓                              |
