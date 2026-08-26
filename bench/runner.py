"""Invoke biston's CLI and parse its JSON output."""

from __future__ import annotations

import json
import logging
import os
import subprocess
from dataclasses import dataclass
from itertools import combinations
from pathlib import Path

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Data types
# ---------------------------------------------------------------------------

@dataclass
class BistonFunction:
    """A function reported by biston."""

    name: str
    file: str  # relative to corpus dir
    start_line: int  # 1-indexed
    end_line: int  # 1-indexed


@dataclass
class BistonPair:
    """A pair of functions detected as similar by biston."""

    left: BistonFunction
    right: BistonFunction
    similarity: float


# ---------------------------------------------------------------------------
# Binary discovery
# ---------------------------------------------------------------------------

def _find_biston_binary() -> str:
    """Locate the biston binary.

    Search order:
    0. ``$BISTON_BIN``  (explicit override, for A/B benchmarking two builds)
    1. ``target/release/biston``  (relative to repo root)
    2. ``target/debug/biston``
    3. ``biston`` on PATH
    """
    override = os.environ.get("BISTON_BIN")
    if override:
        return override

    repo_root = Path(__file__).resolve().parent.parent

    for subpath in ("target/release/biston", "target/debug/biston"):
        candidate = repo_root / subpath
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)

    # Fall back to PATH.
    return "biston"


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

# biston's default ``[output] max_results`` is 50, which silently truncates the
# cluster list.  Truncated output caps recall at an arbitrary ceiling and makes the
# benchmark useless as a regression gate, so the harness writes a config raising it.
_MAX_RESULTS = 1_000_000


def _write_bench_config(corpus_dir: Path) -> None:
    """Write a ``biston.toml`` into *corpus_dir* so results are not truncated.

    ``--config`` defaults to the scan path, so a config placed here is picked up.
    Only ``max_results`` is set; every other knob keeps its default.
    """
    (corpus_dir / "biston.toml").write_text(
        f"[output]\nmax_results = {_MAX_RESULTS}\n", encoding="utf-8"
    )


def run_biston(
    corpus_dir: Path,
    *,
    threshold: float = 0.5,
    min_lines: int = 8,
) -> list[BistonPair]:
    """Run biston against *corpus_dir* and return detected pairs.

    Uses a lower threshold and min-lines than biston's defaults to
    maximize recall for benchmarking.
    """
    _write_bench_config(corpus_dir)
    binary = _find_biston_binary()
    cmd = [
        binary,
        "scan",
        str(corpus_dir),
        "--format",
        "json",
        "--threshold",
        str(threshold),
        "--min-lines",
        str(min_lines),
    ]

    logger.info("Running: %s", " ".join(cmd))
    result = subprocess.run(cmd, capture_output=True, text=True, check=False)

    # 0 is a clean tree and 1 is "findings reported" -- the bench injects clones on
    # purpose, so 1 is the expected outcome, not a failure. Anything else (2, a
    # signal) means biston could not run and the report would be meaningless.
    if result.returncode not in (0, 1):
        logger.error("biston failed (exit %d): %s", result.returncode, result.stderr)
        msg = f"biston exited with code {result.returncode}"
        raise RuntimeError(msg)

    if not result.stdout.strip():
        logger.warning("biston produced no output")
        return []

    report = json.loads(result.stdout)

    return _expand_clusters(report, corpus_dir)


def _expand_clusters(report: dict, corpus_dir: Path) -> list[BistonPair]:
    """Expand biston's cluster-based JSON into a flat list of pairs."""
    pairs: list[BistonPair] = []
    corpus_str = str(corpus_dir)

    for cluster in report.get("clusters", []):
        similarity: float = cluster["similarity"]
        functions: list[BistonFunction] = []

        for func in cluster["functions"]:
            file_path = func["file"]
            # Relativize to corpus dir.
            if file_path.startswith(corpus_str):
                file_path = file_path[len(corpus_str) :].lstrip("/\\")

            functions.append(
                BistonFunction(
                    name=func["name"],
                    file=file_path,
                    start_line=func["start_line"],
                    end_line=func["end_line"],
                )
            )

        # Each pair of functions in the cluster is a detected pair.
        for left, right in combinations(functions, 2):
            pairs.append(BistonPair(left=left, right=right, similarity=similarity))

    logger.info("Biston reported %d pairs from %d clusters", len(pairs), len(report.get("clusters", [])))
    return pairs
