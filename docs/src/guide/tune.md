# Tune

Reference for suppression and policy. Every key here is repo-wide: changing one
changes what biston reports everywhere, so change it deliberately.

**Suppression.** Directives are comments on their own line; trailing text after
the directive is your reason and is ignored by the parser.

- `# biston: ignore -- <reason>` suppresses one function. Put it in the body or
  on the line directly above `def` / `async def`.
- `# biston: ignore-file -- <reason>` suppresses a whole file. It must appear in
  the first 5 lines.
- Glob patterns suppress generated or vendored trees, in `biston.toml`:

```toml
[suppress]
files = ["generated/**", "migrations/**"]
```

- `biston scan --tests-only .` flips the scope to test files instead of
  excluding them.

Suppressed findings are still counted in the `Suppressed:` summary line.

**Acceptance tiers.** A pair is reported when either tier accepts it.

| Tier | Accepted when | Move it when |
|---|---|---|
| `exact` | trees hash identically, shorter function has >= `scan.exact_min_lines` (5) executable lines, both bodies have >= `scan.exact_min_stmts` (3) statements | short idiomatic wrappers dominate the report: raise `scan.exact_min_stmts` |
| `similar` | similarity >= `scan.threshold` (0.85) and shorter function has >= `scan.similar_min_lines` (9) executable lines | too many near-misses: raise `scan.threshold`. Too few, on a tree you know is duplicated: lower `scan.similar_min_lines` |

`scan.min_lines` is a retained alias setting both line floors at once.
`scan.include` and `scan.exclude` decide which files are read at all.

**Containment** is off by default. Turn it on with `--containment` or
`containment.enabled = true`, then:

- `containment.exact_min_fragment_lines` (10) and
  `containment.similar_min_fragment_lines` (15): executable lines a matched run
  needs, per tier. `containment.min_fragment_lines` is the alias for both.
- `containment.threshold` (0.85): minimum containment coefficient.
- `containment.min_ratio` (0.30): contained function size over container size.
- `containment.size_balance` (1.25): largest tolerated size ratio between the
  contained function and the matched run.
- `containment.max_run_fraction` (0.85): largest share of the container's
  statements a run may span.
- `containment.max_probes_per_function` (12): candidate probes per function.

**Precedence.** CLI flag > `biston.toml` > `pyproject.toml [tool.biston]` >
defaults. Setting an alias alongside the keys that supersede it warns and the
alias is ignored. These stop the run outright: `scan.exact_min_lines` above
`scan.similar_min_lines`, `containment.exact_min_fragment_lines` above
`containment.similar_min_fragment_lines`, either exact floor below 1, or
`scan.exact_min_stmts` below 1.

next: run `biston scan .`
