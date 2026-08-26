<!-- Mermaid pitfalls (so you don't have to debug them again):
     - Timeline: colons in labels (e.g. "00:00") break parsing because ":"
       is the delimiter.  Use "0 sec", "2 sec" etc. instead.
     - Gantt + dateFormat X: "after taskId, 200" treats 200 as an absolute
       timestamp, not a duration.  Use explicit start/end: "taskId, 800, 1000".
     - Keep labels simple — no unescaped parentheses or pipes inside node
       text, they break layout in some renderers. -->

# How It Works

biston is a pipeline of small passes. Each pass has a single job: discover files, parse them, extract functions, normalize, hash, bucket by locality-sensitive-hashing, compare within buckets, optionally anti-unify matched pairs, and render the report. Nothing talks across pass boundaries except through plain data types, which makes the whole thing easy to test and cheap to run in parallel.

## Pipeline overview

```mermaid
graph LR
    A[discovery] --> B[parse]
    B --> C[extract]
    C --> D[normalize]
    D --> E[hash + LSH]
    E --> F[similarity]
    F --> G[anti-unify]
    G --> H[report]
```

Each stage lives in its own module:

| Stage | Module | What it does |
|-------|--------|--------------|
| discovery | `src/discovery.rs` | Walks the tree with the `ignore` crate; respects `.gitignore`, include/exclude globs. Test directories and migrations are excluded by default. |
| parse | `src/parse.rs` | Feeds each file into tree-sitter-python, yields a concrete syntax tree. |
| extract | `src/extract.rs` | Slices out every `function_definition` as a `FunctionFragment`, keeping the ones with enough *executable* lines for some tier to accept later. |
| normalize | `src/normalize.rs` | Converts each fragment into a `NormalizedNode` tree — a canonical form. |
| hash | `src/hash.rs` | xxhash3 over the normalized tree: one full-depth root hash for exact matching, plus a set of depth-truncated subtree hashes as the fingerprint. |
| similarity | `src/similarity.rs` | MinHash over the fingerprint, banded LSH for bucketing, exact Jaccard for scoring. |
| containment | `src/containment.rs` | Finds functions that already implement a leading or trailing run of another's body (opt-in via `--containment`). |
| anti-unify | `src/antiunify.rs` | Merges matched pairs into a template with typed holes (Phase 2, opt-in via `--suggest`). |
| report | `src/report.rs` | Emits `CloneReport` as text / JSON / SARIF. |

Supporting modules:

| Module | Role |
|--------|------|
| `src/config.rs` | TOML config loader (`biston.toml` or `[tool.biston]` in `pyproject.toml`). |
| `src/suppress.rs` | Config-level file globs plus inline `# biston: ignore` comments. |
| `src/measure.rs` | The one definition of an executable line and an executable statement — the units every size floor is expressed in. |
| `src/tier.rs` | Acceptance tiers: which of `exact` / `similar` admits a finding, if either does. |
| `src/stats.rs` | Aggregate counts used by the `stats` subcommand. |
| `src/lib.rs` | Public `scan()` API; `src/main.rs` wraps it with a `clap` CLI. |

## Normalization

Two functions can be "the same shape" and still differ in all the surface details — local variable names, literal values, the order of operands to a commutative operator. Normalization strips those details so the hash of a canonical tree is invariant under them.

What the pass does by default:

- Replaces local names with canonical placeholders (`v0`, `v1`, …).
- Drops comments and docstrings entirely — they leave no node behind, so two functions differing only in prose hash identically.
- Drops decorators and type annotations.
- Optionally anonymizes literals and sorts commutative operators (toggled in config).
- Records the kind of each node as a `&'static str` so comparisons stay cheap.

Before (two clearly "the same" functions that differ only in naming and literal values):

```python
def total_price(items):
    total = 0
    for item in items:
        total = total + item.price * 1.2
    return total

def sum_scores(entries):
    acc = 0
    for entry in entries:
        acc = acc + entry.value * 1.5
    return acc
```

After normalization (schematic — both functions now map to the same shape):

```text
function_definition
  parameters(v0)
  body
    assign(v1, literal)
    for(v2 in v0)
      assign(v1, binary(add, v1, binary(mul, attr(v2, v3), literal)))
    return(v1)
```

With `anonymize_literals = true` and `sort_commutative = true` the two fragments hash to the same value. Without them they still land in the same LSH bucket because most of their structure coincides.

## Similarity via MinHash and LSH bands

Comparing every function pairwise is O(n²) and unaffordable on a real repo. biston folds the problem into a locality-sensitive hash:

1. `src/hash.rs` walks the normalized tree bottom-up and collects a **set** of depth-truncated subtree hashes — one per subtree of at least five nodes, each capturing three levels of structure below it. That set is the function's fingerprint. There is no token stream and no ordering: the fingerprint is a set, so reordering a body's statements barely changes it.
2. `src/similarity.rs` reduces each fingerprint to a 128-entry MinHash signature.
3. The signature is cut into contiguous **bands**. Two functions that agree on *any one* band land in the same bucket.
4. Pairs are scored only within buckets, using exact Jaccard over the full fingerprints.

The band layout is derived from `threshold` rather than configured directly, and the permutation count is internal — there is no user-facing band knob. A larger band count means more candidate pairs (recall up, precision down); longer bands mean fewer hits (recall down, precision up).

Scoring is not the last word. A scored pair still has to clear an **acceptance
tier** — required evidence scales inversely with the strength of the match, so a
short *exact* duplicate is reported while a short *fuzzy* one is not, and every
reported finding is tagged with the tier that accepted it. See
[How acceptance works](acceptance.md).

### What is not reportable

A function whose body is only a docstring, `pass`, `...` or comments is skipped before either phase. Normalization drops the prose outright, so what is left of such a body is the function outline over statements that do nothing — however different the text was. There is no logic in them to extract, so pairing them is noise rather than a finding.

## Anti-unification

With `--suggest` (or `[suggest] enabled = true` in config) biston takes each matched pair and **anti-unifies** them: it walks both normalized trees in lockstep and replaces every position where they disagree with a typed *hole*.

Holes are classified by what varied:

- `literal` — a constant differs (e.g. `1.2` vs `1.5`).
- `identifier` — a name differs that survived normalization (e.g. a global or attribute).
- `subtree` — a whole subexpression differs.

Each template gets a quality score based on how much shared structure survived vs. how many holes were introduced. Templates with too many holes, or whose coverage falls below `min_quality`, are dropped — a template that is mostly holes is no better than the original clone.

A worked example. Given these two matched fragments:

```python
def clamp_int(x, lo, hi):
    if x < lo:
        return lo
    if x > hi:
        return hi
    return x

def clamp_float(value, floor, ceiling):
    if value < floor:
        return floor
    if value > ceiling:
        return ceiling
    return value
```

The renderer produces a template such as:

```python
def <hole:name>(<hole:id:a>, <hole:id:b>, <hole:id:c>):
    if <hole:id:a> < <hole:id:b>:
        return <hole:id:b>
    if <hole:id:a> > <hole:id:c>:
        return <hole:id:c>
    return <hole:id:a>
```

That's a ready-made extraction target: three identifier holes, no literal or subtree holes, high coverage score.

## Output

The report format is selected with `--format` or the `[output]` config section:

- `text` — the default, grouped by clone family, with source context.
- `json` — structured dump of `CloneReport`; easy to post-process.
- `sarif` — [SARIF 2.1.0](https://sarifweb.azurewebsites.net/), for uploading to GitHub code-scanning, GitLab, or other CI dashboards.

The `stats` subcommand shares the pipeline but emits aggregate counts instead of individual findings.

## Configuration & suppression

Config lives in `biston.toml` or under `[tool.biston]` in `pyproject.toml`. CLI flags override config values. File-level and function-level suppression is available via config globs or inline `# biston: ignore` / `# biston: ignore-file` comments. Run `biston guide tune` for the reference on every suppression mechanism and every policy key; the foot of any `scan` or `overview` that reports clones points at `biston guide triage`. The full key-by-key reference lives in the [project README](https://github.com/mojzis/biston#configuration).

## Scanning tests

Test suites accumulate their own kind of duplication — near-identical cases that could collapse into `@pytest.mark.parametrize`, copy-pasted arrange/act/assert blocks, repeated fixture plumbing — but that noise usually drowns out production-code findings when mixed into the same report. biston splits the two:

- **By default**, the `scan.exclude` globs (`tests/**`, `**/conftest.py`, `migrations/**`) drop test files at the discovery stage, so `biston scan` and `biston stats` only see your application code.
- **`--tests-only`** (on both `scan` and `stats`) inverts the scope: `include` is replaced with common Python test patterns (`**/test_*.py`, `**/*_test.py`, `**/conftest.py`, `tests/**/*.py`, `**/tests/**/*.py` — the last covering monorepo layouts like `backend/tests/helpers.py`), and `exclude` is cleared. Other knobs (the size floors, `threshold`, normalization) are untouched; tune them in `biston.toml` if your tests want a different baseline than your production code.

Run the two passes separately (e.g. two CI steps, or two cached runs against the same repo) to keep the signal clean.

## Focus scanning

For commit hooks and CI steps that only care about the diff, `scan` and `stats` accept `--files <PATH>` (repeatable) and `--files-from <PATH|->` (list from file or stdin). Discovery and analysis still run over the whole tree — so a newly-introduced clone of an untouched helper is still found — but only pairs where at least one side lives in the focus set make it to the report. See [Commit-hook integration](commit-hooks.md) for the `git diff` recipe.

## The llms.txt surface

Every page on this site is also served as raw Markdown at its source path (for this page, `how-it-works.md`). Two roll-up files round it out:

- [`llms.txt`](llms.txt) — index following the [llms.txt](https://llmstxt.org) convention.
- [`llms-full.txt`](llms-full.txt) — all pages concatenated.

That way an LLM can ingest the full docs without scraping HTML, and the links stay stable across deploys.
