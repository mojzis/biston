"""CLI entry point: ``python -m bench``."""

from __future__ import annotations

import argparse
import json
import logging
import shutil
import sys
import tempfile
from dataclasses import asdict
from pathlib import Path

from tabulate import tabulate

from bench.corpus import clone_cpython, prepare_corpus
from bench.injector import DuplicateInjector
from bench.runner import run_biston
from bench.scorer import BenchmarkResult, score


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="python -m bench",
        description="Benchmark biston's precision and recall against injected duplicates.",
    )
    p.add_argument(
        "--n-per-tier",
        type=int,
        default=10,
        help="Number of duplicates to inject per tier (default: 10)",
    )
    p.add_argument(
        "--seed",
        type=int,
        default=42,
        help="Random seed for deterministic injection (default: 42)",
    )
    p.add_argument(
        "--corpus-dir",
        type=Path,
        default=Path("bench/.corpus"),
        help="Directory for the cached CPython clone (default: bench/.corpus)",
    )
    p.add_argument(
        "--iou",
        type=float,
        default=0.5,
        help="IoU threshold for matching biston pairs to manifest (default: 0.5)",
    )
    p.add_argument(
        "--json-out",
        type=Path,
        default=None,
        help="Write detailed results as JSON to this file",
    )
    p.add_argument(
        "--threshold",
        type=float,
        default=0.5,
        help="Similarity threshold passed to biston (default: 0.5)",
    )
    p.add_argument(
        "--min-lines",
        type=int,
        default=8,
        help="Minimum function lines passed to biston (default: 8)",
    )
    p.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Enable verbose logging",
    )
    return p


def _print_table(result: BenchmarkResult) -> None:
    headers = ["Tier", "Injected", "TP", "FP", "FN", "Precision", "Recall", "F1"]
    rows: list[list[object]] = []

    for tr in result.tiers:
        rows.append([
            tr.tier,
            tr.injected,
            tr.tp,
            tr.fp,
            tr.fn,
            f"{tr.precision:.3f}",
            f"{tr.recall:.3f}",
            f"{tr.f1:.3f}",
        ])

    agg = result.aggregate
    rows.append([
        agg.tier,
        agg.injected,
        agg.tp,
        agg.fp,
        agg.fn,
        f"{agg.precision:.3f}",
        f"{agg.recall:.3f}",
        f"{agg.f1:.3f}",
    ])

    print(tabulate(rows, headers=headers, tablefmt="simple"))


def _write_json(result: BenchmarkResult, path: Path) -> None:
    data = asdict(result)
    path.write_text(json.dumps(data, indent=2), encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    """Run the benchmark pipeline."""
    args = _build_parser().parse_args(argv)

    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(levelname)s: %(message)s",
    )
    logger = logging.getLogger(__name__)

    # 1. Prepare corpus.
    corpus_dir: Path = args.corpus_dir.resolve()
    clone_cpython(corpus_dir)

    work_dir = Path(tempfile.mkdtemp(prefix="biston_bench_"))
    try:
        logger.info("Working directory: %s", work_dir)
        prepare_corpus(corpus_dir, work_dir)

        # 2. Inject duplicates.
        injector = DuplicateInjector(work_dir, seed=args.seed)
        manifest = injector.inject(n_per_tier=args.n_per_tier)
        logger.info("Injected %d duplicates", len(manifest))

        # 3. Run biston.
        pairs = run_biston(work_dir, threshold=args.threshold, min_lines=args.min_lines)
        logger.info("Biston reported %d pairs", len(pairs))

        # 4. Score.
        result = score(pairs, manifest, iou_threshold=args.iou)

        # 5. Report.
        print()
        _print_table(result)
        print()

        if args.json_out:
            _write_json(result, args.json_out)
            logger.info("JSON results written to %s", args.json_out)

    finally:
        shutil.rmtree(work_dir, ignore_errors=True)

    return 0


if __name__ == "__main__":
    sys.exit(main())
