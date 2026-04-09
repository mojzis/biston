# Phase 1 — Structural Clone Detection

The core. No ML dependencies, no network, pure computation. This alone is a
useful tool — ship it before starting Phase 2.

```
Python files → tree-sitter parse → normalize → hash → similarity → report
```

## 1.1 File discovery and parsing

Walk directories with `walkdir`, respect `.gitignore` via `ignore` crate (or
simple glob exclude patterns from config). Memory-map files with `memmap2`,
pass byte slices to `tree-sitter` with `tree-sitter-python` grammar.

tree-sitter always produces a tree, even for broken syntax. Parse errors become
`ERROR` nodes — skip them, don't crash.

**Output**: `Vec<ParsedFile>` where each contains the tree, source bytes,
and file path.

## 1.2 Function extraction

Use tree-sitter queries to extract all `function_definition` and
`decorated_definition` nodes. For each, record:

- Function name, file path, line range
- The AST subtree rooted at the function body
- Source text (for display in results)

**Minimum size filter**: skip functions shorter than `min_lines` (default 10).
This eliminates `__init__`, one-liner properties, and trivial wrappers that
would drown results in noise.

**Output**: `Vec<FunctionFragment>` across all files.

## 1.3 AST normalization

This is where detection quality lives. Normalize each function's AST before
hashing:

**Always normalize:**
- Local identifiers (variables, parameters, loop targets) → positional
  placeholders (`$0`, `$1`, ...). Track scope: `def`, `for`, `with`,
  `as`, assignment targets define locals.
- Comments and docstrings → strip entirely
- Type annotations → strip (they vary between otherwise-identical functions)
- Decorators → strip (configurable — some users may want decorator-aware matching)
- Whitespace/formatting → irrelevant (tree-sitter operates on structure)

**Preserve (by default):**
- Global identifiers (module-level names, imports, dotted access) — anonymizing
  these causes false matches between unrelated code
- Literals — anonymizing them caused too many false positives in AdaCore's
  experience. Make this configurable (`anonymize_literals = true` for users
  who want it)
- Function/method names in calls — `df.groupby()` vs `df.merge()` should
  not match

**Configurable:**
- `sort_commutative`: reorder children of `+`, `*`, `==`, `and`, `or`, `|`
  so `a + b` matches `b + a`. Off by default (subtle — can cause false matches
  with non-commutative operators in complex expressions).

## 1.4 Bottom-up subtree hashing

Single DFS pass, O(N) for the entire tree:

```
hash(leaf) = xxh3(node_kind + normalized_text)
hash(node) = xxh3(node_kind + hash(child_0) + hash(child_1) + ...)
```

Use `xxhash-rust` with xxh3 — fastest non-cryptographic hash available in Rust.
Store as `u64`.

Each function produces two things:
- Its **root hash** (quick equality check for Type-1/2 clones)
- Its **subtree hash set** (all subtrees with ≥ `min_subtree_nodes` nodes,
  default 5) for Jaccard similarity

## 1.5 Similarity detection

**Fast path — exact matches**: group functions by root hash. Any group with
≥ 2 members is a Type-1/2 clone set. Trivially O(N) via `FxHashMap`.

**Near-miss detection — MinHash + LSH**:
- Compute MinHash signatures (128 permutations) over each function's subtree
  hash set
- LSH bucketing with band size tuned to target threshold (e.g., 20 bands of
  6 rows for ~0.7 Jaccard threshold)
- For candidate pairs from LSH, compute exact Jaccard similarity
- Filter by configured threshold

**Output**: `Vec<SimilarPair>` with similarity score, both function references.

## 1.6 Reporting

Output formats:
- **text**: human-readable, file:line references, source snippets
- **json**: machine-readable, for downstream tooling
- **sarif**: for CI integration (GitHub code scanning, etc.)

Group overlapping clones (A~B, B~C → {A, B, C} cluster). Sort by
similarity descending, then by clone size descending.

## 1.7 Configuration

`biston.toml` at project root:

```toml
[scan]
min_lines = 10
threshold = 0.7
exclude = ["tests/", "**/conftest.py", "migrations/"]
include = ["**/*.py"]

[normalization]
anonymize_locals = true
anonymize_literals = false
strip_decorators = true
strip_type_annotations = true
sort_commutative = false

[output]
format = "text"
group_overlapping = true
max_results = 50
show_source = true
context_lines = 3
```

Also read from `[tool.biston]` in `pyproject.toml`. CLI flags override config
file. Config file overrides defaults.

## Milestones

### "It parses"

- [ ] Project scaffolded from ty-find
- [ ] tree-sitter + tree-sitter-python compiles and parses a .py file
- [ ] File walker with include/exclude
- [ ] Function extraction via tree-sitter queries
- [ ] Basic test: parse a fixture, extract N functions

### "It hashes"

- [ ] AST normalization (local identifier anonymization)
- [ ] Bottom-up subtree hashing with xxh3
- [ ] Exact-match grouping by root hash
- [ ] Test: two identical functions → detected as clones

### "It finds near-misses"

- [ ] MinHash signatures over subtree hash sets
- [ ] LSH bucketing for candidate pair generation
- [ ] Exact Jaccard computation for candidates
- [ ] Threshold filtering
- [ ] Test: two functions differing by variable names → detected

### "It reports"

- [ ] Text output formatter
- [ ] JSON output formatter
- [ ] CLI with clap (scan subcommand)
- [ ] Config file loading (biston.toml)
- [ ] **Ship v0.1.0**

## Dependencies

```
tree-sitter          0.25    # parsing
tree-sitter-python   0.25    # Python grammar
xxhash-rust          0.8     # subtree hashing (xxh3)
rustc-hash           2.1     # FxHashMap for hot paths
rayon                1.10    # parallel file processing
walkdir              2       # file discovery
memmap2              0.9     # zero-copy file reading
clap                 4       # CLI
serde + toml         1 / 0.8 # config
```

## Performance targets

| Codebase size | Target |
|---------------|--------|
| 10K LOC       | < 1s   |
| 100K LOC      | < 5s   |
| 500K LOC      | < 20s  |

Benchmark on every PR. Regression = blocked merge.

## Quality bar

- Zero crashes on any Python file (including syntax errors, encoding issues,
  zero-byte files, symlink loops).
- Deterministic output: same input → same results, regardless of thread count.
- Every reported pair exceeds the configured threshold. No silent erosion.
- Benchmarked on every release against a reference corpus.
