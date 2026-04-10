"""Score biston's output against the ground-truth injection manifest."""

from __future__ import annotations

from dataclasses import dataclass, field

from bench import TIERS, Manifest, ManifestEntry
from bench.runner import BistonPair


# ---------------------------------------------------------------------------
# Result types
# ---------------------------------------------------------------------------

@dataclass
class TierResult:
    """Precision / recall / F1 for a single tier (or aggregate)."""

    tier: str
    injected: int = 0
    tp: int = 0
    fp: int = 0
    fn: int = 0
    precision: float = 0.0
    recall: float = 0.0
    f1: float = 0.0


@dataclass
class BenchmarkResult:
    """Full benchmark result across all tiers."""

    tiers: list[TierResult] = field(default_factory=list)
    aggregate: TierResult = field(default_factory=lambda: TierResult(tier="TOTAL"))


# ---------------------------------------------------------------------------
# IoU on 1-indexed line ranges
# ---------------------------------------------------------------------------

def _line_iou(span_a: tuple[int, int], span_b: tuple[int, int]) -> float:
    """Compute Intersection-over-Union on two 1-indexed [start, end] line ranges."""
    a_start, a_end = span_a
    b_start, b_end = span_b

    inter_start = max(a_start, b_start)
    inter_end = min(a_end, b_end)
    intersection = max(0, inter_end - inter_start + 1)

    union = (a_end - a_start + 1) + (b_end - b_start + 1) - intersection
    if union <= 0:
        return 0.0
    return intersection / union


# ---------------------------------------------------------------------------
# Matching
# ---------------------------------------------------------------------------

def _pair_matches_entry(
    pair: BistonPair,
    entry: ManifestEntry,
    iou_threshold: float,
) -> bool:
    """Check whether a biston pair matches a manifest entry.

    Files must match (in either order) and both line-range IoUs must
    meet the threshold.
    """
    left_file = pair.left.file
    right_file = pair.right.file
    src_file = entry.source_file
    tgt_file = entry.target_file

    # Try both orientations.
    if left_file == src_file and right_file == tgt_file:
        iou_src = _line_iou((pair.left.start_line, pair.left.end_line), entry.source_span)
        iou_tgt = _line_iou((pair.right.start_line, pair.right.end_line), entry.target_span)
    elif left_file == tgt_file and right_file == src_file:
        iou_tgt = _line_iou((pair.left.start_line, pair.left.end_line), entry.target_span)
        iou_src = _line_iou((pair.right.start_line, pair.right.end_line), entry.source_span)
    else:
        return False

    return iou_src >= iou_threshold and iou_tgt >= iou_threshold


def _compute_prf(tp: int, fp: int, fn: int) -> tuple[float, float, float]:
    precision = tp / (tp + fp) if (tp + fp) > 0 else 0.0
    recall = tp / (tp + fn) if (tp + fn) > 0 else 0.0
    f1 = 2 * precision * recall / (precision + recall) if (precision + recall) > 0 else 0.0
    return precision, recall, f1


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def score(
    pairs: list[BistonPair],
    manifest: Manifest,
    iou_threshold: float = 0.5,
) -> BenchmarkResult:
    """Score biston's detected pairs against the injection manifest.

    A manifest entry is a **true positive** when at least one biston pair
    matches it.  A biston pair that matches no manifest entry is a
    **false positive**.

    Returns per-tier and aggregate results.
    """
    # Track which manifest entries are matched.
    matched_entries: set[int] = set()
    # Track which biston pairs matched something.
    matched_pairs: set[int] = set()

    for pi, pair in enumerate(pairs):
        for mi, entry in enumerate(manifest):
            if _pair_matches_entry(pair, entry, iou_threshold):
                matched_entries.add(mi)
                matched_pairs.add(pi)

    # Per-tier scoring.
    tier_results: list[TierResult] = []
    total_tp = 0
    total_fn = 0

    for tier in TIERS:
        tier_indices = [i for i, e in enumerate(manifest) if e.tier == tier]
        injected = len(tier_indices)
        tp = sum(1 for i in tier_indices if i in matched_entries)
        fn = injected - tp
        total_tp += tp
        total_fn += fn

        precision, recall, f1 = _compute_prf(tp, 0, fn)  # FP not split per tier
        tier_results.append(
            TierResult(
                tier=tier,
                injected=injected,
                tp=tp,
                fp=0,
                fn=fn,
                precision=precision,
                recall=recall,
                f1=f1,
            )
        )

    # Aggregate.
    total_fp = sum(1 for i in range(len(pairs)) if i not in matched_pairs)
    agg_precision, agg_recall, agg_f1 = _compute_prf(total_tp, total_fp, total_fn)
    aggregate = TierResult(
        tier="TOTAL",
        injected=len(manifest),
        tp=total_tp,
        fp=total_fp,
        fn=total_fn,
        precision=agg_precision,
        recall=agg_recall,
        f1=agg_f1,
    )

    return BenchmarkResult(tiers=tier_results, aggregate=aggregate)
