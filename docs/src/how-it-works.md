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
| discovery | `src/discovery.rs` | Walks the tree with the `ignore` crate; respects `.gitignore`, include/exclude globs. |
| parse | `src/parse.rs` | Feeds each file into tree-sitter-python, yields a concrete syntax tree. |
| extract | `src/extract.rs` | Slices out every `function_definition` as a `FunctionFragment`. |
| normalize | `src/normalize.rs` | Converts each fragment into a `NormalizedNode` tree — a canonical form. |
| hash + LSH | `src/hash.rs` | xxhash3 over the normalized tree plus a banded LSH fingerprint. |
| similarity | `src/similarity.rs` | Pairs candidates that share LSH bands, scores them against a threshold. |
| anti-unify | `src/antiunify.rs` | Merges matched pairs into a template with typed holes (Phase 2, opt-in via `--suggest`). |
| report | `src/report.rs` | Emits `CloneReport` as text / JSON / SARIF. |

Supporting modules:

| Module | Role |
|--------|------|
| `src/config.rs` | TOML config loader (`biston.toml` or `[tool.biston]` in `pyproject.toml`). |
| `src/suppress.rs` | Config-level file globs plus inline `# biston: ignore` comments. |
| `src/stats.rs` | Aggregate counts used by the `stats` subcommand. |
| `src/lib.rs` | Public `scan()` API; `src/main.rs` wraps it with a `clap` CLI. |

## Normalization

Two functions can be "the same shape" and still differ in all the surface details — local variable names, literal values, the order of operands to a commutative operator. Normalization strips those details so the hash of a canonical tree is invariant under them.

What the pass does by default:

- Replaces local names with canonical placeholders (`v0`, `v1`, …).
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

## Similarity via LSH bands

Comparing every function pairwise is O(n²) and unaffordable on a real repo. biston folds the problem into a locality-sensitive hash:

1. The normalized tree is serialised into a stream of node-kind tokens.
2. `xxhash3` produces a 64-bit fingerprint over that stream, plus a handful of shorter per-band hashes over slices of the same sequence.
3. Fragments whose fingerprints agree on *any one* band land in the same bucket.
4. Pairs are scored only within buckets.

A higher band count means more candidate pairs (recall up, precision down); a longer band means fewer hits (recall down, precision up). The defaults are tuned so that a 1 000-file repo produces hundreds of candidate pairs, not millions.

Only pairs whose similarity meets the `threshold` (default `0.7`) end up in the report.

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

Config lives in `biston.toml` or under `[tool.biston]` in `pyproject.toml`. CLI flags override config values. File-level and function-level suppression is available via config globs or inline `# biston: ignore` / `# biston: ignore-file` comments. The full key-by-key reference lives in the [project README](https://github.com/mojzis/biston#configuration).

## Focus scanning

For commit hooks and CI steps that only care about the diff, `scan` and `stats` accept `--files <PATH>` (repeatable) and `--files-from <PATH|->` (list from file or stdin). Discovery and analysis still run over the whole tree — so a newly-introduced clone of an untouched helper is still found — but only pairs where at least one side lives in the focus set make it to the report. See [Commit-hook integration](commit-hooks.md) for the `git diff` recipe.

## The llms.txt surface

Every page on this site is also served as raw Markdown at its source path (for this page, `how-it-works.md`). Two roll-up files round it out:

- [`llms.txt`](llms.txt) — index following the [llms.txt](https://llmstxt.org) convention.
- [`llms-full.txt`](llms-full.txt) — all pages concatenated.

That way an LLM can ingest the full docs without scraping HTML, and the links stay stable across deploys.
