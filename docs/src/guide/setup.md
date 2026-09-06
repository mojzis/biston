# Setup

biston is not configured in this repository yet. Run everything below at the
repository root, in this order.

**1. Install.** Add it as a dev dependency: `uv add --dev biston`. Alternatives:
`pip install biston`, or `uvx biston scan .` for a one-off look. Prefer the dev
dependency; reaching for `uvx` on every run pins no version.

**2. Configure.** The defaults are conservative. Keep them until you have a
clean baseline. Set only what this tree actually needs, in `pyproject.toml`:

```toml
[tool.biston.scan]
exclude = ["tests/**", "**/conftest.py", "migrations/**", "vendor/**"]
```

A `biston.toml` at the root is the alternative; it wins over `pyproject.toml`.

**3. Baseline before you gate.** Run `biston scan .` and resolve or suppress
every finding before wiring biston into any gate; `biston guide triage` says
how. A scanner gating a dirty repo gets removed, not the clones.

**4. Integrate.** As a madoqua pre-commit step, in `pyproject.toml`:

```toml
[tool.madoqua]
extend_check = [{ name = "biston", cmd = "biston scan --focus-args" }]
```

madoqua appends the staged Python files to `biston scan --focus-args`, which
reads every positional as a focus file with the scan root at `.`, so only
pairs touching a staged file are reported. `biston scan . a.py b.py` is not
valid: without `--focus-args` at most one positional is accepted, the root.
`pass_files = false` with `biston scan .` works too, but scans the whole tree.

In the check aggregator this repo already has, which runs the full tree and
has no diff context, the same one command. With poethepoet:

```toml
[tool.poe.tasks]
clones = "biston scan ."
check = ["lint", "typecheck", "clones"]
```

Both `scan` and `stats` exit 1 on findings, 0 when clean and 2 when biston
could not run; to gate on a count instead, write `biston stats --format json .`
to a file and test `.clone_pairs` with jq. With pre-commit or prek:

```yaml
- repo: https://github.com/mojzis/biston
  rev: v0.7.2
  hooks:
    - id: biston
```

**5. Cost.** A full scan runs in ~320 ms per 100K lines of Python and ~1.1 s
over CPython's `Lib/` (355K lines), so it fits a per-commit gate.

next: run `biston scan .`
