# Triage

A scan reported findings. Work them one at a time.

**Read the finding.** Tier `exact` means the normalized trees hash identically;
tier `similar` means the printed similarity cleared `scan.threshold`. A cluster
is a set of whole functions that match each other; a containment finding is
directed - a run inside one function is already implemented by another. The
`path:start-end` span is 1-indexed and inclusive of both ends.

**Apply the remedy ladder.** Take the first rule that applies; do not skip ahead.

1. Containment finding: delete the reported run from the container and call the
   named function instead. There is nothing to design.
2. `exact` cluster: extract one helper and make every member call it.
3. `similar` cluster whose `biston scan --suggest .` template has only
   `identifier` or `literal` holes: extract it, passing those holes as
   parameters.
4. `similar` with `subtree` holes, or with no template at all: stop. Report the
   file, the span and the similarity to a human. This is a judgement call, not
   agent work.
5. Duplication that is intentional - performance, isolation, generated code,
   test clarity - and only then: suppress it with
   `# biston: ignore -- <reason>` on its own line above the `def`, or
   `# biston: ignore-file -- <reason>` in the file's first 5 lines. The reason
   is required.

**Around every edit.** Run the test suite before you touch anything. Refactor.
Run the test suite again. Then re-scan the files you changed with
`biston scan --focus-args <changed files>`: the finding must be gone and no new
finding may appear. If either fails, revert the edit.

**Do not:**

- Do not raise `scan.threshold`, `scan.exact_min_lines`,
  `scan.similar_min_lines` or `scan.exact_min_stmts` to make a finding
  disappear. Those are repo-wide policy, decided by humans, and changing one
  hides every other finding it covers. See `biston guide tune`.
- Do not add `# biston: ignore` without a reason.
- Do not suppress a finding to pass a gate.

next: run `biston scan --focus-args <changed files>`
