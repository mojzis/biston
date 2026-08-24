# CLAUDE.md

biston — structural clone detector for Python. One Rust crate (lib + bin), shipped to PyPI as a prebuilt binary wheel via maturin (`bindings = "bin"` — there is no PyO3 / native-extension layer).

## Repo map

- `src/lib.rs` — pipeline orchestration (`scan`, `scan_focused`), per-file parallelism, focus-file filtering
- `src/main.rs` — clap CLI: `scan`, `overview`, `stats`, `usage`, `completions`; CLI→config override precedence
- `src/discovery.rs` — file walk (`ignore` crate, `.gitignore`) + include/exclude globs, sorted output
- `src/parse.rs` — tree-sitter-python parse → `ParsedFile { path, source, tree }`
- `src/extract.rs` — tree-sitter query → one `FunctionFragment` per `function_definition`, `min_lines` floor
- `src/normalize.rs` — AST → `NormalizedNode`; renames locals, drops comments/docstrings/decorators/annotations
- `src/hash.rs` — xxh3 root hash + depth-truncated subtree fingerprint; `body_statements`, inert-body detection
- `src/similarity.rs` — MinHash (128 perms) + banded LSH candidates, exact Jaccard scoring → `SimilarPair`
- `src/containment.rs` — directed "A already implements a prefix/suffix run of B's body" (opt-in)
- `src/antiunify.rs` — anti-unify two normalized trees → `TemplateNode` with typed holes; scoring, Python rendering
- `src/report.rs` — `CloneReport`, pair clustering, text/JSON/SARIF formatting
- `src/overview.rs` — file-centric rendering for the `overview` subcommand
- `src/stats.rs` — aggregate counts for `stats` (the CI-gating surface)
- `src/suppress.rs` — `# biston: ignore[-file]` comments, `[suppress] files` globs, `biston usage` help text
- `src/config.rs` — `Config` + all defaults; loads `biston.toml`, else `pyproject.toml [tool.biston]`, else defaults
- `tests/scan.rs` — library-level pipeline tests over `tests/fixtures/`; `tests/cli.rs` — end-to-end CLI via `assert_cmd`
- `tests/common/mod.rs` — shared Python source generators for focus-file tests
- `bench/` — Python precision/recall harness: `corpus.py` (CPython `Lib/`), `injector.py` (5 tiers), `runner.py`, `scorer.py`
- `docs/` — mdBook source; `plans/` — constitution and phase plans

## Commands

```bash
cargo build --release                              # binary at target/release/biston
cargo test --all-features                          # full suite: inline unit tests + tests/scan.rs + tests/cli.rs
cargo test --lib normalize::                       # unit tests of one module (they live inline in src/)
cargo test --test scan                             # one integration target (scan | cli)
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
make review                                        # fmt + clippy + test + cargo-audit + cargo-deny
maturin build --release                            # Python wheel → target/wheels/
python3 -m pytest bench/tests -q                   # bench harness's own tests (needs pytest)
make bench BENCH_ARGS="--n-per-tier 5"             # precision/recall run; clones CPython Lib/ on first use (network)
```

## Architecture in one breath

discover (`discovery.rs`) → parse (`parse.rs`) → extract (`extract.rs`) → suppress (`suppress.rs`) → normalize (`normalize.rs`) → hash (`hash.rs`) → similarity: MinHash/LSH + Jaccard (`similarity.rs`) → containment (`containment.rs`, opt-in) → anti-unify (`antiunify.rs`, opt-in) → report (`report.rs` / `overview.rs` / `stats.rs`), all driven by `scan_focused` in `src/lib.rs`.

## Invariants & gotchas

- Suppression reads raw source and must stay ahead of normalization — `# biston: ignore` is gone once `normalize.rs` drops comments. Order lives in `lib.rs::process_file`.
- Comments and docstrings must leave **no node at all**, not an empty placeholder: a placeholder perturbs every ancestor hash. See `normalize::leaves_no_trace`.
- `lib.rs::body_statement_spans` must skip exactly what normalization drops, or containment reports the wrong lines. Guarded by `containment_span_skips_the_prose_it_no_longer_normalizes`.
- Determinism: files run through `par_iter`, so `scan_focused` re-sorts by (file, start_line) and only *then* assigns `fragment_index`; pairs sort by similarity desc, left, right. A new stage must be order-independent or sort at the end — output must never depend on thread count.
- Containment supersedes the symmetric pair for the same two functions, and that suppression must run *before* suggestions: an anti-unified template for a containment pair is a hole-riddled artefact.
- Fingerprint resolution must match everywhere: `min_subtree_nodes = 5` (`lib.rs`, `containment::MIN_SUBTREE_NODES`) and `hash::SUBTREE_HASH_DEPTH = 3`.
- Precision over recall is the tiebreaker for every detection decision (`plans/constitution.md`). The degeneracy filters in `similarity.rs` (`has_executable_body`, non-empty `subtree_hashes`) exist because without them 7021 of 14389 pairs on CPython's `Lib/` were noise.
- Off by default, deliberately — do not flip silently: `containment.enabled`, `suggest.enabled`, `normalization.anonymize_literals`, `normalization.sort_commutative`. Asserted in `config.rs` tests.
- `stats.clone_pairs` excludes containment findings; CI gates read that field, so never fold a new kind of finding into it.
- `.pre-commit-hooks.yaml` needs `require_serial: true` — parallel batching would silently miss cross-batch clones.
- Config precedence: `biston.toml` > `pyproject.toml [tool.biston]` > defaults (`Config::load`); partial tables fill from defaults via `#[serde(default)]`.
- No `.unwrap()` / `.expect()` / `panic!` outside tests — clippy denies them repo-wide (`clippy.toml` allows them in tests only). Use `.context()` with `?`; any `#[allow]` needs a `reason = "..."`.

## Testing conventions

- TDD, red-green-refactor: no implementation without a failing test first; every bug fix ships a regression test.
- Fixtures are `tests/fixtures/*.py`, each self-documenting: its header states what must (or must not) be reported and which knob enforces that — see `containment_trivial.py`.
- Negative cases are first-class and named for what must *not* fire: `no_clones.py`, `short_functions.py`, `docstring_only.py`, `no_logic_bodies.py`, `comment_noise.py`, `containment_trivial.py`, `containment_prose.py`.
- Scope a test to one fixture with `config_for_file("x.py")` (or `_with_suggest` / `_with_containment`) in `tests/scan.rs` — it sets `include` to that file and clears `exclude`.
- A new detection feature ships with: a positive fixture, a negative fixture proving the guard that keeps precision, a default-off assertion if it is opt-in, and a `tests/cli.rs` case if it adds a flag.
- Assert on values, never "doesn't panic". Never weaken an assertion to make it pass — diagnose first and default to the test being right. A flaky test needs a root cause, not a looser bound.

## Before completing work

- `prek` hooks run fmt / clippy / `cargo test --all-features --bins` on commit (`prek.toml`); run `make review` before pushing.
- Run `/rust-review` on any significant change and clear all 🔴 items; merge main first (`git fetch origin main && git merge origin/main`), then re-run the checks.
- When stuck: say so explicitly, stop after 3 failed approaches and report what was tried, don't edit unrelated code hoping it helps, revert if things got worse.

## Docs pointers

- `plans/constitution.md` — goals, values, non-goals. Read before changing detection behaviour.
- `plans/phase-1-structural.md`, `phase-1.5-hardening.md`, `phase-1-deviations.md`, `phase-2-antiunify.md`, `phase-2.5-near-miss-rework.md`, `phase-3-embeddings.md` — phase plans.
- `docs/src/how-it-works.md`, `containment.md`, `commit-hooks.md` — user-facing detail (mdBook, published to <https://mojzis.github.io/biston/>).
- `README.md` — CLI reference. `CHANGELOG.md` — released behaviour changes.
