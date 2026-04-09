# Biston — Project Constitution

> *Biston betularia*, the peppered moth: the textbook example of adaptation through mimicry.

## Why

Codebases accumulate repeated concepts — logic that was copied, adapted, and never
abstracted. This duplication is invisible to linters, silently multiplies maintenance
cost, and erodes the coherence of a project over time. No practical tool today finds
these patterns and tells you what the abstraction should be.

## What biston is

A code similarity and abstraction detector for Python codebases.

Biston finds code that repeats — not just copy-paste, but logic that shares enough
structure to warrant a common function, base class, or higher-order abstraction.
It tells you *what* was repeated and *what the shared abstraction would look like*.

## What biston brings

- **Detection across a spectrum**: from exact clones through near-miss duplicates
  to structurally similar code that implements the same concept differently.
- **Abstraction suggestions**: not just "these are similar" but "here is the function
  you should have written, and here are the parameters." The tool computes the
  maximal shared structure and classifies the differences.
- **Actionable output**: every reported pair is worth looking at. Results integrate
  into CI workflows and editor tooling.

## Values

**Reliable.** Precision over recall. A developer acts on biston's output without
triaging false positives all day. Conservative defaults. If biston says it's a
duplicate, it is.

**Configurable.** Codebases differ. What counts as duplication in a data pipeline
is noise in a test suite. Meaningful knobs, sensible defaults, no PhD required.

**Fast.** Fast enough to run on every commit, not just nightly. A large monorepo
completes in seconds.

## Non-goals

- Language support beyond Python (for now).
- Automated refactoring. Biston suggests. Humans (or their LLMs) refactor.
- Detecting algorithmic equivalence. Bubble sort vs. quicksort is a research
  problem, not a tool feature.
