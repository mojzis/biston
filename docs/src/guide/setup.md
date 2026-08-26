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
every finding it reports before wiring biston into any gate; run
`biston guide triage` for how. Adding a scanner to a dirty repo's gate gets the
scanner removed, not the duplication.

**4. Integrate.** Add `biston scan .` to the check aggregator this repo already
has. The aggregator runs the full tree and has no diff context. With poethepoet:

```toml
[tool.poe.tasks]
clones = "biston scan ."
check = ["lint", "typecheck", "clones"]
```

`just`, `make` and `nox` are the same shape: one recipe, one command. Both
`scan` and `stats` exit 1 on findings, 0 when clean and 2 when biston could not
run. To gate on a count instead, redirect `biston stats --format json .` to a
file, ignore its exit code, and test `.clone_pairs` in that file with jq.

For a commit hook, with pre-commit or prek:

```yaml
- repo: https://github.com/mojzis/biston
  rev: v0.7.0
  hooks:
    - id: biston
```

For a raw hook, with no framework:

```bash
git diff --name-only --diff-filter=ACM -- '*.py' | biston scan --files-from - .
```

**5. Cost.** A full scan runs in ~320 ms per 100K lines of Python and ~1.1 s
over CPython's `Lib/` (355K lines), so it fits a per-commit gate.

next: run `biston scan .`
