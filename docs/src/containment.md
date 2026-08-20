# Containment

Ordinary clone detection is symmetric: it says *these two functions look alike*.
Containment detection is directed. It says something stronger and more actionable:

```text
b.py:42-58 is already implemented by normalize_records at a.py:12 — call it instead.
```

That is a concrete instruction. Delete those lines, call the function that already
exists. There is nothing to design and nothing to name.

Containment is **off by default**. Turn it on with `--containment`, or with
`enabled = true` in the `[containment]` config section.

## What it looks for

Exactly one shape, deliberately: the contained function must match a **leading or
trailing run of top-level statements** in the container's body.

```python
def normalize_records(rows):        # A
    ...

def load_then_normalize(source):    # B — ends by doing everything A does
    rows = parse(source)
    ...
    # ── from here, identical to normalize_records ──
```

Explicitly **not** detected in this phase:

- a function matching a run in the *middle* of another body,
- interleaved or non-contiguous containment,
- anything requiring live-variable analysis to prove the run is actually extractable.

A missed containment costs nothing. A bogus "extract this" costs the tool's
credibility, so every ambiguous case is dropped.

## How it works

The whole feature rests on one decision: **fragments are probes, not index entries.**

biston builds a second LSH index over whole-function *body* fingerprints. Candidate
runs are hashed and used to *query* that index; they are never stored in it. Only
whole-function ↔ fragment comparisons can happen — fragment ↔ fragment comparison is
not filtered out afterwards, it is unrepresentable, because the index is keyed by a
type that only a whole function can produce.

Two consequences:

- the symmetric detector's index is untouched, so its bucket occupancy is unchanged
  (measured: identical, p99 occupancy 2, before and after);
- with containment disabled, nothing is computed at all — the cost is structurally
  zero, not computed-and-discarded.

### Run-relative naming

Normalization numbers local variables with a counter that runs over the whole
function, parameters first. The same code therefore gets *different* placeholders
depending on what precedes it — so a trailing run shares almost nothing with the
standalone function containing the same statements. Measured on the project's own
fixture, that costs a genuine match 0.211 containment against a 0.85 threshold.

Run fingerprints are therefore renumbered relative to the run itself, in
first-encounter order. A run's fingerprint then depends only on its statements, not
on where it sits in the parent body, which is what makes the leading and trailing
cases behave identically.

### Finding the boundary

Candidate generation probes a coarse ladder of run lengths (eighths of the body).
It only has to make the true run *collide*; the exact boundary is then found by
sweeping run lengths with exact set arithmetic and keeping the best-scoring one.
So the reported span is exact regardless of how coarse the ladder is.

## Guards

A finding is reported only if it passes every one of these.

| Guard | Default | Why |
|---|---|---|
| containment coefficient `\|A ∩ F\| / min(\|A\|,\|F\|)` ≥ `threshold` | `0.85` | Separate from, and stricter than, the symmetric `threshold`, which scores with Jaccard. |
| size balance `min/max` ≥ `1 / size_balance` | `1.25` → `0.80` | The coefficient alone **cannot** exclude interior containment: if the run strictly contains the function, the coefficient is 1.0 however much extra the run carries. Requiring comparable sizes is what anchors a match to the run's boundary. |
| `min_fragment_lines` | `15` | Measured over the run's *executable* statements only. Docstrings and comments are excluded from both ends — otherwise a two-line idiom under a sixteen-line docstring clears a fifteen-line floor, which is exactly the boilerplate the guard exists to suppress. |
| `min_ratio` | `0.30` | Below this the contained function is a detail of a much larger one, not an abstraction waiting to be named. |
| `max_run_fraction` | `0.85` | A run covering nearly the whole body is the whole function again — the symmetric detector's job. |
| not lexically nested | — | A nested `def` is extracted in its own right *and* is a statement of its parent, so it always matches a run of the parent. That is not duplication. |

## Interaction with other features

**Containment wins over similarity.** If the same pair is found both ways, the
symmetric pair is suppressed. Reporting both says the same thing twice, and the
directed form is the more useful one.

**`--suggest` emits nothing for a containment finding.** Anti-unification walks two
trees in lockstep with no alignment, so it diverges at the run boundary and turns the
tail into holes. A hole-riddled template is worse than no template.

**Focus scanning** (`--focus-args` / `--files` / `--files-from`) keeps a finding when
**either** side is in the focus set — the container or the contained function.

**Statistics** count containment separately, in `containment_findings`. The existing
`clone_pairs` field still counts only symmetric pairs, so CI gates reading it keep
enforcing what they always enforced.

## Configuration

```toml
[containment]
enabled = false            # or pass --containment
min_fragment_lines = 15    # executable lines in the matched run
min_ratio = 0.30           # contained size / container size
threshold = 0.85           # containment coefficient
size_balance = 1.25        # largest tolerated size ratio between A and the run
max_run_fraction = 0.85    # largest share of the body a run may span
max_probes_per_function = 12
```

## Output

`text` phrases the finding as an instruction, as shown at the top of this page.

`json` gains a `containments` array — and, from this release, a `schema_version`
field. **Version 1 had no version field at all**, so an absent `schema_version`
means pre-containment output. The array is omitted entirely when there are no
findings.

```json
{
  "schema_version": 2,
  "clusters": [],
  "containments": [
    {
      "contained": { "name": "normalize_records", "file": "a.py", "start_line": 12, "end_line": 27 },
      "container": { "name": "load_then_normalize", "file": "b.py", "start_line": 30, "end_line": 58 },
      "role": "suffix",
      "start_line": 42,
      "end_line": 58,
      "statement_count": 4,
      "score": 1.0
    }
  ]
}
```

`sarif` emits rule `biston/containment-detected`. The **primary location is the
container's run** — the code a reader would delete — with the contained function as a
related location, so the direction survives the round trip into a code-scanning UI.
