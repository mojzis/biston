# biston

A structural clone detector for Python code. Written in Rust.

It parses Python files with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), normalizes the AST, and finds functions that are structurally similar to each other.

## Install

```
uv add biston
```

Or build from source:

```
cargo build --release
```

## Usage

```
biston <COMMAND>
```

### Commands

#### `biston scan`

Scan a directory for code clones.

```
Usage: biston scan [OPTIONS] [PATH]

Arguments:
  [PATH]  Directory to scan [default: .]

Options:
      --format <FORMAT>        Output format [possible values: text, json, sarif]
      --min-lines <MIN_LINES>  Minimum function length in lines
      --threshold <THRESHOLD>  Similarity threshold (0.0 - 1.0)
      --config <CONFIG>        Config file directory (looks for biston.toml or pyproject.toml)
      --suggest                Generate abstraction suggestions for similar pairs
      --tests-only             Restrict the scan to Python test files (overrides include/exclude)
  -h, --help                   Print help
```

#### `biston stats`

Show statistics about scan findings.

```
Usage: biston stats [OPTIONS] [PATH]

Arguments:
  [PATH]  Directory to scan [default: .]

Options:
      --format <FORMAT>        Output format (text or json) [possible values: text, json, sarif]
      --min-lines <MIN_LINES>  Minimum function length in lines
      --threshold <THRESHOLD>  Similarity threshold (0.0 - 1.0)
      --config <CONFIG>        Config file directory (looks for biston.toml or pyproject.toml)
      --tests-only             Restrict the scan to Python test files (overrides include/exclude)
  -h, --help                   Print help
```

##### Scanning tests only

Test suites often accumulate duplication (near-identical cases that could be `@pytest.mark.parametrize`, copy-pasted arrange/act/assert blocks). By default biston excludes test files so production-code findings stay focused. Pass `--tests-only` to flip the scope and scan only test files:

```
biston scan --tests-only
biston stats --tests-only
```

The flag replaces `include` with common Python test patterns (`**/test_*.py`, `**/*_test.py`, `**/conftest.py`, `tests/**/*.py`) and clears `exclude`. Other knobs (`min_lines`, `threshold`, normalization) are left untouched — tune them separately in `biston.toml` if you want different defaults for a test run.

## Configuration

Settings can go in `biston.toml` or under `[tool.biston]` in `pyproject.toml`. If both files exist, `biston.toml` takes priority. CLI flags override config file settings.

### `[scan]`

| Setting | Default | Description |
|---|---|---|
| `min_lines` | `10` | Minimum function length in lines |
| `threshold` | `0.7` | Similarity threshold (0.0–1.0) |
| `exclude` | `["tests/**", "**/conftest.py", "migrations/**"]` | File patterns to exclude |
| `include` | `["**/*.py"]` | File patterns to include |

### `[normalization]`

| Setting | Default | Description |
|---|---|---|
| `anonymize_locals` | `true` | Replace local variable names |
| `anonymize_literals` | `false` | Replace literal values |
| `strip_decorators` | `true` | Remove decorators from AST |
| `strip_type_annotations` | `true` | Remove type hints |
| `sort_commutative` | `false` | Sort commutative operations |

### `[output]`

| Setting | Default | Description |
|---|---|---|
| `format` | `"text"` | Output format (`text`, `json`, or `sarif`) |
| `group_overlapping` | `true` | Group overlapping clones |
| `max_results` | `50` | Maximum number of results |
| `show_source` | `true` | Display source code in output |
| `context_lines` | `3` | Number of context lines around clones |

### `[suggest]`

| Setting | Default | Description |
|---|---|---|
| `enabled` | `false` | Enable suggestion generation |
| `min_quality` | `0.6` | Minimum template coverage score (0.0–1.0) |
| `max_holes` | `5` | Maximum holes before suppressing |
| `render_python` | `true` | Render templates as Python source |

### `[suppress]`

| Setting | Default | Description |
|---|---|---|
| `files` | `[]` | File glob patterns to suppress entirely |

### Example `biston.toml`

```toml
[scan]
min_lines = 15
threshold = 0.8
exclude = ["vendor/"]
include = ["src/**/*.py"]

[normalization]
anonymize_locals = false
anonymize_literals = true

[output]
format = "json"
max_results = 100

[suggest]
enabled = true
min_quality = 0.8
```

### Inline suppression

You can also suppress findings with Python comments:

- `# biston: ignore-file` — suppress the entire file (must appear in the first 5 lines)
- `# biston: ignore` — suppress a single function (place in the function body or on the preceding line)

## License

MIT
