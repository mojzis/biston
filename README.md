# biston

A structural clone detector for Python code. Written in Rust.

It parses Python files with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), normalizes the AST, and finds functions that are structurally similar to each other.

## Install

```
pip install biston
```

Or build from source:

```
cargo build --release
```

## Usage

```
biston scan [path]
```

Options:

- `--threshold <0.0-1.0>` — similarity threshold (higher = stricter)
- `--min-lines <n>` — minimum function length to consider
- `--format <text|json|sarif>` — output format
- `--suggest` — show abstraction suggestions for similar pairs
- `--config <dir>` — directory containing `biston.toml` or `pyproject.toml`

## Configuration

Settings can go in `biston.toml` or under `[tool.biston]` in `pyproject.toml`.

## License

MIT
