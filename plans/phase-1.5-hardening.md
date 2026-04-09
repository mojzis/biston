# Phase 1.5 — Hardening

Clean up the Phase 1 foundation before building Phase 2 on top of it.
The re-parsing fix is a prerequisite for anti-unification.

## 1.5.1 Restructure pipeline to per-file processing

Currently `lib.rs::scan()` does:
1. Parse all files (parallel)
2. Extract all functions (collects `Vec<FunctionFragment>`)
3. Re-parse each function's source text individually for normalization

The re-parse happens because tree-sitter `Node` borrows from `Tree`, which
is dropped after step 1. Fix by processing per-file:

```
for each file (parallel via rayon):
    parse file → Tree alive
    extract functions → nodes still valid
    for each function:
        normalize (using original Node) → NormalizedNode
        hash → HashedFunction
    collect (FunctionFragment, NormalizedNode, HashedFunction)
```

This eliminates N re-parses and ensures normalization sees the original
parse context.

## 1.5.2 Store `NormalizedNode` trees in output

Add normalized trees alongside fragments so Phase 2 can anti-unify
without re-normalizing:

```rust
pub struct CloneReport {
    pub functions: Vec<FunctionFragment>,
    pub normalized: Vec<NormalizedNode>,  // parallel to functions
    pub pairs: Vec<SimilarPair>,
}
```

## 1.5.3 Intern `NormalizedNode.kind`

tree-sitter node kinds are a fixed set of ~50 strings. Currently every
`NormalizedNode` allocates a `String` for `kind`. Change to:

```rust
pub struct NormalizedNode {
    pub kind: &'static str,          // interned from tree-sitter
    pub text: Option<String>,
    pub children: Vec<Self>,
}
```

tree-sitter's `Node::kind()` already returns `&'static str`, so this is
just removing the `.to_owned()` calls.

## 1.5.4 CLI integration tests

Write `assert_cmd` tests in `tests/`:
- `biston scan --help` exits 0
- `biston scan` on empty dir prints "No clones detected"
- `biston scan` on fixture dir with known clones detects them
- `biston scan --format json` produces valid JSON
- `biston scan --format sarif` produces valid SARIF

## 1.5.5 End-to-end test with fixtures

Create `tests/fixtures/` with checked-in Python files:
- `simple_clones.py` — two identical functions, different variable names
- `near_miss.py` — two functions differing by one statement
- `no_clones.py` — unrelated functions
- `short_functions.py` — below min_lines threshold

Write a `#[test]` that calls `biston::scan()` on the fixtures directory
and asserts expected clone pairs.

## 1.5.6 Implement or remove `sort_commutative`

The config field exists, normalization accepts it, but nothing acts on it.
Either:
- Implement in `hash.rs`: sort child hashes before concatenating at
  commutative operator nodes (`+`, `*`, `==`, `and`, `or`, `|`)
- Or remove the config field and document it as future work

## 1.5.7 Visibility cleanup

Change to `pub(crate)`:
- `find_exact_matches` in `similarity.rs`
- `CloneCluster` and `cluster_pairs` in `report.rs`
- `minhash_signature`, `jaccard_similarity` in `similarity.rs`

## Milestones

### "Pipeline is clean"
- [ ] Per-file processing, no re-parsing
- [ ] NormalizedNode stored in CloneReport
- [ ] kind interned as &'static str

### "Tests are solid"
- [ ] CLI integration tests with assert_cmd
- [ ] End-to-end scan() test with fixtures
- [ ] sort_commutative resolved

### "API is tight"
- [ ] Visibility cleanup
- [ ] Ship as v0.1.1
