# Phase 3 — Embedding-based Similarity (optional)

Catches conceptually similar code that differs too much structurally for
Phase 1 to find. This is opt-in — it requires a model download and adds
processing time.

Prerequisite: Phase 1 complete. Phase 2 is independent — both can run on
embedding-discovered pairs.

```
Function source → tokenize → embed (ONNX) → HNSW index → nearest neighbors → merge with Phase 1
```

## 3.1 Model selection

UniXcoder (125M params, ONNX export) via `ort` crate. Best accuracy/size
tradeoff for code similarity: highest mean F1 (0.918) with lowest variance
across benchmarks, 56x smaller than CodeLlama yet outperforms it on similarity
tasks.

~200MB model download, one-time.

Important caveat: embedding models predominantly encode surface-level features,
especially identifier names. They do not robustly capture algorithmic structure.
This means they find functions that *talk about similar things*, not functions
that *do the same thing differently*. Still useful — catches patterns that
structural methods miss entirely.

## 3.2 Embedding pipeline

- Extract function source text (with minimal normalization — models use
  identifier names as signal, unlike Phase 1 which anonymizes them)
- Tokenize → embed via ONNX Runtime → mean pool over last hidden layer →
  768-dim vector per function
- Build HNSW index via `hnsw_rs`
- Query nearest neighbors with cosine similarity
- Start at threshold ≥ 0.9 for high precision, tune down to ~0.7 for
  broader recall

For a monorepo with 10K–100K functions, this is trivially scalable — brute-force
search on 100K 768-dim vectors takes milliseconds.

## 3.3 Integration with Phase 1 results

- Functions already grouped by Phase 1 structural similarity → skip
- For remaining ungrouped functions, find embedding neighbors
- Merge into unified results with both structural and embedding scores
- Phase 2 anti-unification runs on embedding-discovered pairs too
- Results tagged with detection method (structural / embedding / both)

## 3.4 Model management

First run downloads model to `~/.cache/biston/models/`. Subsequent runs
use cached model.

- `BISTON_MODEL_PATH` env var for offline/CI use
- `biston model download` subcommand for explicit pre-download
- `biston model info` to show cached model path and size

## Milestones

### "It embeds"

- [ ] ONNX Runtime integration via `ort`
- [ ] UniXcoder model loading and single-function inference
- [ ] Test: embed two semantically similar but structurally different functions →
      high cosine similarity

### "It indexes"

- [ ] HNSW index construction over all function embeddings
- [ ] Nearest-neighbor search with cosine similarity
- [ ] Score merging with Phase 1 structural similarity

### "It ships deep mode"

- [ ] Model download / cache management
- [ ] `--deep` flag on CLI
- [ ] `biston model download` subcommand
- [ ] Integration with Phase 2 anti-unification for embedding-discovered pairs
- [ ] **Ship v0.3.0**

## Dependencies

New dependencies (Phase 3 only — behind a cargo feature flag `embed`):

```
ort                  2       # ONNX Runtime bindings
hnsw_rs              0.3     # approximate nearest neighbor search
```

New crate in workspace: `biston-embed`.

The CLI and Python bindings conditionally depend on `biston-embed`:

```toml
[features]
default = []
embed = ["biston-embed"]
```

Users who don't want the ONNX dependency get a smaller binary. `pip install biston`
includes embeddings; `pip install biston-lite` (or a feature flag) does not.

## Configuration additions

```toml
[embed]
enabled = false                        # opt-in
model = "unixcoder-base"               # model identifier
model_path = "~/.cache/biston/models"  # override cache location
threshold = 0.85                       # cosine similarity threshold
```

## Performance targets

| Codebase size | Phase 1+3 | Phase 1+2+3 |
|---------------|-----------|-------------|
| 10K LOC       | < 10s     | < 12s       |
| 100K LOC      | < 30s     | < 40s       |
| 500K LOC      | < 2min    | < 2.5min    |

Embedding is the bottleneck — dominated by model inference time, not search.
Batch inference (multiple functions per forward pass) is critical for performance.
