# What is biston?

biston is a structural clone detector and refactor suggester for Python. It parses your code with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), normalizes each function into a canonical AST, and finds groups of functions that are structurally similar — even when local names, literals, and argument order differ. For each match it can also propose an anti-unified template with typed "holes" that you could extract into a shared helper.

Written in Rust and distributed as a Python package, biston runs fast enough to drop into CI pipelines.

## Who it's for

- **Python teams** tracking copy-paste drift across modules as a codebase grows.
- **CI pipelines** that want SARIF output wired into code-quality dashboards.
- **AI coding agents** (and the humans reviewing their PRs) where boilerplate tends to accumulate function by function.

## Working with a coding agent

biston explains itself, so you do not have to. Hand your agent one line:

> Run `uvx biston guide` at the root of this repo and follow what it says.

It prints what to do at the moment you are actually in — [Setup](guide/setup.md)
when the repo has no biston config, [Triage](guide/triage.md) when it does — and
a scan that reports clones ends with a footer pointing back at
`biston guide triage`. [Tune](guide/tune.md) is the reference for suppression
and the policy keys. The CLI and these pages serve the same bytes.

## Next

- [How It Works](how-it-works.md) — the pipeline, from discovery to anti-unified templates.

## Machine-readable docs

Every page on this site is also served as raw Markdown, following the [llms.txt](https://llmstxt.org) convention:

- [`llms.txt`](llms.txt) — compact index with links to every page as `.md`.
- [`llms-full.txt`](llms-full.txt) — all pages concatenated into a single document.

Drop either one into an LLM context window to give the model the full picture without scraping HTML.
