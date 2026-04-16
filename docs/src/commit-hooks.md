# Commit-hook integration

biston is designed to scan a whole repository, but when you wire it into a pre-commit hook you usually don't want every unrelated pair in the codebase to fail a commit — only clones that involve the files the committer touched. The `--files` / `--files-from` flags narrow the **report** to those files while still scanning the whole tree, so cross-file clones between a changed file and the rest of the repo are still detected.

## The recipe

```bash
git diff --name-only --diff-filter=ACM -- '*.py' \
  | biston scan --files-from - .
```

What each piece does:

- `git diff --name-only --diff-filter=ACM -- '*.py'` — list Python files that are **A**dded, **C**opied, or **M**odified in the current index (swap in `HEAD~1..HEAD` or `--cached` depending on hook timing).
- `biston scan --files-from -` — read that list from stdin (one path per line). Paths are resolved relative to the current working directory.
- The positional `.` — root of the scan. biston still discovers and parses everything under it; the focus list only restricts which pairs make it into the report.

An empty list (no Python files changed) correctly emits no pairs — the hook passes silently. That's why `--files-from -` is the right shape for hooks: `--files $(git diff --name-only)` silently expands to nothing when the diff is empty, which reverts to a full-repo scan and can trip the hook on pre-existing clones unrelated to the commit.

## Semantics

Given a repo with clones `A ↔ B` (inside the committer's change) and `C ↔ D` (elsewhere):

| Invocation | Pairs emitted |
|---|---|
| `biston scan .` | `A↔B`, `C↔D` |
| `biston scan --files A.py .` | `A↔B` (and any `A↔X` with X anywhere in the repo) |
| `biston scan --files-from - .` with empty stdin | *(none)* |

A focus path that can't be resolved (e.g. a file deleted in the same changeset) is warned about and skipped, not treated as a fatal error — the scan continues with whatever focus paths did resolve.

## Tips

- Use `--diff-filter=ACM` to avoid passing deleted files. biston tolerates them, but it's clearer at the hook level.
- Combine with `--format sarif` if your CI wants to upload findings as annotations — the SARIF output is filtered the same way.
- `stats` supports the same flags, so you can gate a hook on a numeric threshold: `biston stats --files-from - --format json . | jq '.clone_pairs'`.
- For a dry run, drop `--files-from` to see the full-repo report and confirm the focused scan isn't hiding surprises.

## Repeated `--files`

For one-off use outside a hook, `--files` is repeatable:

```bash
biston scan --files src/auth.py --files src/session.py .
```

Each `--files` takes a single path; repeat the flag to add more. This form conflicts with `--files-from` — pick one per invocation.
