# How acceptance works

Finding a candidate pair and reporting it are different decisions. Everything up to
and including scoring is about *what looks alike*; acceptance is about *what is worth
a developer's attention*. This page is the second decision.

Acceptance has two tiers, `exact` and `similar`. A finding is reported when either
tier admits it, and every reported finding is tagged with the tier that did.

## Why two tiers

A single length floor plus a single similarity threshold cannot express a sensible
policy, because the two kinds of evidence are not comparable:

- A short **exact** duplicate is strong evidence. Two six-line functions with the
  same normalized tree are the same code, and saying so is useful.
- A short **fuzzy** duplicate is noise. Jaccard over a handful of subtrees is a
  coarse statistic that jumps on small edits; over six lines it says almost nothing.
- A larger function justifies acceptance at the same similarity, because the
  evidence base under the score is bigger.

So the required evidence scales *inversely* with the strength of the match. The
tiers are discrete steps rather than a size/similarity curve on purpose: a formula
is neither explainable in a report nor tunable by the owner of a repository, and two
labelled steps are both.

## Whole-function pairs

| Tier | Accepted when |
|---|---|
| `exact` | the two normalized trees hash identically **and** the shorter function has ≥ `scan.exact_min_lines` executable lines **and** both bodies have ≥ `scan.exact_min_stmts` statements |
| `similar` | similarity ≥ `scan.threshold` **and** the shorter function has ≥ `scan.similar_min_lines` executable lines |

Defaults: `exact_min_lines = 5`, `similar_min_lines = 9`, `exact_min_stmts = 3`,
`threshold = 0.85`.

The size gates read the **shorter** of the two functions. A pair is only as
well-evidenced as its smaller half; reading the larger one would let a 200-line
function vouch for a 3-line one.

### The exact tier's statement guard

After normalization — locals anonymized, comments, docstrings and annotations gone —
short bodies collide on *idiom* rather than on content. A delegation wrapper, a
guard-return pair, `try: ... except: pass`: these hash identically because the idiom
is identical, and there is nothing in either copy to extract. The exact tier
therefore also asks that the body hold at least `exact_min_stmts` **top-level**
statements that survive normalization. Nesting deliberately does not count: it is
the shape of the whole body that is at issue, and counting a `try` block's contents
would let exactly these shapes clear the floor they are meant to fail.

The guard applies to the exact tier only. A long identical pair that fails it can
still be admitted by the fuzzy rule — a similarity of 1.0 clears any threshold — and
is then reported as `similar`, which is an honest record of the weaker evidence it
was accepted on.

## Contained runs

| Tier | Accepted when |
|---|---|
| `exact` | the run's fingerprint and the contained function's are identical **and** the run spans ≥ `containment.exact_min_fragment_lines` executable lines |
| `similar` | the containment coefficient ≥ `containment.threshold` **and** the run spans ≥ `containment.similar_min_fragment_lines` executable lines |

Defaults: `exact_min_fragment_lines = 10`, `similar_min_fragment_lines = 15`,
`threshold = 0.85`.

The floors are higher than the whole-function ones because a fragment carries less
context: a reader looking at a run of statements has no signature, no name and no
return to tell them what it is for.

Every other containment guard — size balance, minimum ratio, maximum run fraction,
leading/trailing-only, not-lexically-nested — is unchanged and **composes** with the
tiers. A run both tiers would take is still dropped if it fails one of them. See
[Containment](containment.md).

## Executable lines

Every floor on this page is counted in **executable lines**, never in raw source
lines.

> An executable line is a distinct source line holding at least one token that
> survives AST normalization.

Consequently:

- Comment-only lines, docstring lines (single- and multi-line) and blank lines never
  count. Neither does a line holding nothing but a delimiter — the `)` closing a
  multi-line call.
- Two statements on one line (`a = 1; b = 2`) are **one** executable line and **two**
  executable statements.
- A line continuation counts every line that holds a token.
- A decorator is not part of what is compared, so it is not part of what is measured.
  The reported span still starts at the first decorator.

This is why a function padded to twenty lines with a licence header, a long
docstring and blank lines does not clear a nine-line floor. The measure is defined
once, in `src/measure.rs`, and every gate calls it.

## Extraction versus acceptance

Extraction keeps every function with at least `min(exact_min_lines,
similar_min_lines)` executable lines — the shortest a tier could later accept. The
tier gates run when a pair is *scored*, not when a function is indexed: a function
dropped at extraction cannot be matched at all, so extracting on the stricter floor
would make the exact tier unable to see the short duplicates it exists to report.

The cost is a larger indexed population — properties, dunders, small wrappers — which
is a measured cost, not a guessed one; see the benchmark notes in the changelog.

## Configuration

```toml
[scan]
exact_min_lines = 5        # executable lines; floor for exact whole-function matches
similar_min_lines = 9      # executable lines; floor for fuzzy whole-function matches
exact_min_stmts = 3        # statements surviving normalization; exact tier only
threshold = 0.85           # Jaccard floor for the fuzzy tier

[containment]
exact_min_fragment_lines = 10
similar_min_fragment_lines = 15
threshold = 0.85           # containment coefficient floor for the fuzzy tier
```

Each key has a CLI flag of the same name (`--exact-min-lines`, …), and CLI beats
config beats defaults.

### `min_lines`, and `min_fragment_lines`

Both are retained aliases, and are not deprecated. Set on its own, each still means
what it always meant — one floor applied to both tiers:

```toml
[scan]
min_lines = 10             # exact and fuzzy alike need ten executable lines
```

Set *alongside* a tier key, the tier keys win and a single warning names both. Note
that even under the alias, the floor is now measured in executable lines: a function
that used to clear `min_lines = 10` on padding no longer does.

### Validation

These are hard errors, not warnings — an inverted pair of floors turns the whole
policy upside down, and silently reordering them would hide the mistake behind
plausible-looking results:

- `exact_min_lines` ≤ `similar_min_lines`
- `exact_min_fragment_lines` ≤ `similar_min_fragment_lines`
- every floor ≥ 1, and `exact_min_stmts` ≥ 1

## Reading the tier in output

`text` names it in the cluster header and in the containment detail line:

```text
Clone cluster #1 (tier: exact, similarity: 1.00, 2 functions)
```

`json` carries a `tier` field on every cluster and every containment (schema
version 3). A cluster's tier is the **weakest** among its pairs, the same reading as
its `similarity`: one exact pair does not vouch for the fuzzy ones grouped with it.

`sarif` puts it in the result message and in `properties.tier`.

`stats` counts findings by tier under `Accepted by tier`, alongside the existing
breakdown by score — which is a different question: a pair can score 1.0 and still be
a `similar`-tier finding, when it cleared the fuzzy rule rather than the exact one.
