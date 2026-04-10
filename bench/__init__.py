"""Benchmark harness for measuring biston's precision and recall against known duplicates."""

from __future__ import annotations

from dataclasses import dataclass

TIERS: list[str] = ["exact", "renamed", "restructured", "augmented", "semantic"]


@dataclass
class ManifestEntry:
    """A single ground-truth record of an injected duplicate."""

    source_file: str  # path relative to corpus dir
    source_span: tuple[int, int]  # (start_line, end_line), 1-indexed
    target_file: str  # path relative to corpus dir
    target_span: tuple[int, int]  # (start_line, end_line), 1-indexed
    tier: str  # one of TIERS
    description: str


Manifest = list[ManifestEntry]
