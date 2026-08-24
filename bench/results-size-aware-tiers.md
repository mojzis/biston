# Size-aware tiers: before/after

Measured while implementing the acceptance tiers, against the release build of the
commit before them (`v0.5.5`, `49df6c0`) and the release build of the change.

## Corpora

| Name | What it is | Files | Lines |
|---|---|---|---|
| `Lib` | CPython `Lib/`, shallow clone, test directories excluded (`**/test/**`, `**/tests/**`, `**/idle_test/**`) | 699 scanned | ~355,000 |
| `Lib-100k` | the same tree truncated at 100,000 lines by sorted path | 232 | 100,000 |

Both binaries ran the same corpus with the same `biston.toml` (the excludes above
plus `max_results`), each otherwise on its own defaults — that is the shipped
behavior change, not an isolated one.

## Detection

`Lib`, default configuration:

| Metric | Before | After | Δ |
|---|---|---|---|
| Indexed functions | 6,622 | 8,622 | **+30.2%** |
| LSH index entries | 132,160 | 172,420 | +30.5% |
| LSH candidate pairs | 3,164 | 3,191 | **+0.9%** |
| Reported pairs | 141 | 60 | −57% |
| — of which `exact` tier | — | 49 | |
| — of which `similar` tier | — | 11 | |
| Clone clusters | 77 | 36 | |
| Containment findings (`--containment`) | 19 | 17 | |

`Lib-100k`, default configuration:

| Metric | Before | After | Δ |
|---|---|---|---|
| Indexed functions | 1,961 | 2,528 | +28.9% |
| LSH index entries | 39,140 | 50,560 | +29.2% |
| LSH candidate pairs | 619 | 442 | **−28.6%** |
| Reported pairs | 68 | 26 (19 exact / 7 similar) | |

**A 30% larger index costs no candidate pairs.** The two populations are not
nested: the lower floor adds short, dense functions, and the switch to executable
lines *removes* prose-heavy ones that used to clear a ten-raw-line floor on
docstrings and comments. Those are precisely the functions with small, outline-shaped
fingerprints that collide with everything, so dropping them offsets — on `Lib-100k`,
more than offsets — the ones the lower floor adds.

**Fewer pairs are reported, not more.** Isolating the two default changes on `Lib`:

| Configuration | Pairs | exact | similar |
|---|---|---|---|
| Before (`min_lines = 10` raw, `threshold = 0.7`) | 141 | — | — |
| After, `--threshold 0.7` | 97 | 49 | 48 |
| After, defaults (`threshold = 0.85`) | 60 | 49 | 11 |

So the tiers themselves account for 141 → 97 (the fuzzy tier's nine *executable*
lines dropping padded pairs), and the threshold default for 97 → 60.

## Runtime

Interleaved A/B, alternating binaries, after a warm-up run each. The machine is a
shared 4-core VM, so the minimum is the most stable statistic; medians are given too.

| Corpus | Before (min / p25 / median) | After (min / p25 / median) | Δ min | Δ p25 | Δ median |
|---|---|---|---|---|---|
| `Lib-100k`, n=20 | 273 / 284 / 288 ms | 313 / 318 / 326 ms | **+14.7%** | +12.3% | +13.2% |
| `Lib`, n=16 | 951 / 970 / 996 ms | 1088 / 1117 / 1160 ms | +14.4% | +15.2% | +16.6% |

**The 15% gate on the 100K-LOC corpus is not tripped** (+12.3% to +14.7% depending on
the statistic). On the 3.5× corpus the regression sits right at the line, and run-to-
run noise on this machine is ±3%, so treat "about 15%" as the honest number.

Where it comes from, measured by holding the population down (`--min-lines 10`, which
under the change means ten *executable* lines → 4,365 functions, a third fewer than
before): that run costs about what the baseline does. The regression is therefore the
larger indexed population — decision 6's stated, accepted cost — rather than the
measuring itself, which was brought down by dropping a hash insert per token and a
heap-allocated cursor per internal node from the executable-line walk.

## Injected-duplicate harness (`python -m bench`)

Whole `Lib/` including tests, 10 injected duplicates per tier, seed 42, the harness's
own settings (`--threshold 0.5 --min-lines 8`):

| Injection tier | Before recall | After recall | After, `--min-lines 5` |
|---|---|---|---|
| exact | 1.00 | 0.90 | 1.00 |
| renamed | 0.70 | 0.60 | 0.70 |
| restructured | 0.40 | 0.30 | 0.40 |
| augmented | 0.00 | 0.00 | 0.00 |
| semantic | 0.90 | 0.90 | 0.90 |
| **total** | **0.60** | **0.54** | **0.60** |

The drop is the change of *unit*, not a detection regression: `--min-lines 8` now
means eight executable lines, and the three missed injections are functions whose
raw span clears eight only on prose. At `--min-lines 5` the after build reproduces
the baseline injection-for-injection. The harness's default should move to 5 if it is
meant to keep measuring the same thing.

(The harness's precision column is not meaningful here — it scores every genuine
CPython clone as a false positive, since only injected pairs are ground truth.)

## Precision sample: short exact-tier pairs

The gate: inspect the newly reported `exact`-tier pairs in the 5–8 executable-line
range and check they are not dominated by idiomatic boilerplate. On `Lib` there are
41 of them (of 49 exact-tier pairs in total) — found by diffing the default run
against one with `--exact-min-lines 9`. All 41 were read.

| Kind | Pairs | Verdict |
|---|---|---|
| Cross-module copy-paste | 8 | Actionable. `bz2.read1`/`lzma.read1`, `bz2.tell`/`lzma.tell`, `inspect._get_code_position`/`traceback._get_code_position`, `pathlib._os._get_copy_blocksize`/`shutil._determine_linux_fastcopy_blocksize`, `argparse.__replace__`/`optparse.__replace__`, `profile.print_stats`/`profiling.tracing.print_stats`, `multiprocessing.dummy.__repr__`/`managers.__repr__`, `concurrent.futures`/`ctypes._layout` |
| Same-file duplication | 11 | Actionable. `ssl` verified/unverified chain accessors duplicated across `SSLSocket` and `SSLObject`, `_pydatetime.__format__` ×2, `collections.fromkeys` ×2, `asyncio.streams.factory` ×2, `idlelib.configdialog.init_validators` ×2, `unittest.mock.__call__` ×2, `pydoc.__init__` ×2, `profile.trace_dispatch_i`/`_l` |
| Comparison-dunder families | 16 | Defensible. `_pydecimal` and `xmlrpc.client` `__lt__`/`__le__`/`__gt__`/`__ge__` — the honest reading is "use `functools.total_ordering`". They collapse into one cluster each in the report, so the reader sees two findings, not sixteen. |
| `throw`/`athrow` idiom | 3 | Borderline. `_collections_abc` repeats the same three-branch re-raise across three ABCs; deliberate, but genuinely duplicated. |
| Attribute-assignment constructors | 3 | Weakest. `subprocess.CalledProcessError.__init__` / `TimeoutExpired.__init__` and neighbours: four `self.x = x` lines. A reviewer would likely say "dataclass, or leave it". |

Not dominated by boilerplate: about half are copy-paste a maintainer would act on, and
the weakest category is 3 of 41 pairs (one cluster). Combined with the overall pair
count falling from 141 to 60, the defaults do not need changing on this evidence.

## Reproducing

```bash
# corpus
git clone --depth 1 --filter=blob:none --sparse https://github.com/python/cpython.git /tmp/cpython
git -C /tmp/cpython sparse-checkout set Lib/

# per-binary numbers
biston stats /tmp/cpython/Lib --format json          # indexed functions, pairs, tiers
RUST_LOG=debug biston scan /tmp/cpython/Lib | grep "candidate pair"

# injected-duplicate harness
BISTON_BIN=./target/release/biston python -m bench --corpus-dir /tmp/cpython --seed 42
```
