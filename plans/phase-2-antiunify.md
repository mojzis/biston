# Phase 2 — Anti-unification

The differentiator. Takes similar pairs from Phase 1 and computes *what the
shared abstraction would look like*.

Prerequisite: Phase 1.5 complete (pipeline restructured, `NormalizedNode`
trees stored in `CloneReport`, `kind` interned as `&'static str`).

```
SimilarPair → parallel NormalizedNode walk → template with holes → classify holes → render suggestion
```

## 2.1 First-order anti-unification on `NormalizedNode` trees

Given two `NormalizedNode` trees, walk them in parallel:
- Where nodes match in `kind` and `text` → keep the node
- Where they diverge → insert a **hole** (named variable)

This runs in O(n) time and space (linear in tree size). The result is
a `TemplateNode` tree with holes.

```rust
// src/antiunify.rs

pub enum TemplateNode {
    /// Shared structure — same in both inputs.
    Shared {
        kind: &'static str,
        text: Option<String>,
        children: Vec<TemplateNode>,
    },
    /// Divergence point — different in the two inputs.
    Hole {
        name: String,           // e.g. "$HOLE_0"
        left: NormalizedNode,   // what the first function has here
        right: NormalizedNode,  // what the second function has here
        classification: HoleKind,
    },
}
```

Lives in `src/antiunify.rs` as a module in the existing `biston` crate.
No workspace split needed — all types are local.

## 2.2 Hole classification

For each hole, examine what's inside it on both sides:

| Hole contains | Classification | Refactoring implication |
|--------------|----------------|------------------------|
| Different identifiers | Rename parameter | Extract function with parameter |
| Different literals | Value parameter | Extract function with parameter |
| Different expressions | Expression parameter | Extract with expression param or lambda |
| Different statement blocks | Behavioral parameter | Higher-order function / callback |
| Different method calls | Strategy parameter | Strategy pattern / protocol |
| Missing vs. present nodes | Optional block | Default parameter or conditional |

```rust
pub enum HoleKind {
    Identifier,
    Literal,
    Expression,
    StatementBlock,
    MethodCall,
    OptionalBlock,
}
```

Classification is a simple match on the `NormalizedNode.kind` of the
hole contents.

## 2.3 Abstraction quality scoring

Score = `shared_nodes / total_nodes` (template coverage), penalized by:
- Too many holes (> 5) → likely false pattern
- Very large holes (> 50% of either original) → more different than similar
- Holes at structurally critical positions (e.g., entire function body is a hole)

Only suggest abstraction if score exceeds a quality threshold (configurable,
default 0.6).

## 2.4 Template rendering

Render the template as valid (or near-valid) Python. **Important:** rendering
uses the original source text from `FunctionFragment.source_text` for shared
regions, not the normalized form (which has anonymized identifiers and
stripped annotations). Holes are rendered as descriptive parameter names
derived from classification.

```python
# Suggested abstraction for: process_orders (orders.py:45) and process_returns (returns.py:112)
# Similarity: 0.82 | Holes: 2

def _process_items(items, $FILTER_FN, $OUTPUT_PATH):
    validated = [item for item in items if $FILTER_FN(item)]
    results = []
    for item in validated:
        result = transform(item)
        results.append(result)
    write_output(results, $OUTPUT_PATH)
    return results
```

Rendering approach:
- Walk the `TemplateNode` tree
- For `Shared` nodes: extract corresponding source text from one of the
  original functions (using byte range tracking from Phase 1)
- For `Hole` nodes: emit `$HOLE_NAME` with a comment showing what each
  side had

## Milestones

### "It anti-unifies"

- [ ] Anti-unification algorithm on `NormalizedNode` trees
- [ ] Test: two functions with renamed variables → template with identifier holes

### "It classifies"

- [ ] Hole classification by content type
- [ ] Abstraction quality scoring with penalty heuristics
- [ ] Test: function pair with a block-level difference → behavioral hole detected

### "It suggests"

- [ ] Template rendering as Python source (using original source text)
- [ ] Integration with Phase 1 reporting (text, json, sarif output)
- [ ] `--suggest` flag on CLI
- [ ] **Ship v0.2.0**

## Dependencies

No new crate dependencies beyond Phase 1. Anti-unification operates on
`NormalizedNode` trees already in the `biston` crate.

No workspace split — `src/antiunify.rs` module in the existing crate.

## Configuration additions

```toml
[suggest]
min_quality = 0.6          # minimum template coverage to suggest
max_holes = 5              # suppress suggestions with too many holes
render_python = true       # render template as Python (vs. abstract tree)
```
