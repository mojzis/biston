"""Download and prepare the CPython corpus for benchmarking."""

from __future__ import annotations

import logging
import shutil
import subprocess
from pathlib import Path

logger = logging.getLogger(__name__)

_CPYTHON_REPO = "https://github.com/python/cpython.git"


def clone_cpython(corpus_dir: Path) -> Path:
    """Shallow-clone CPython's Lib/ directory into *corpus_dir*.

    Uses sparse checkout so only ``Lib/`` is fetched.  Skips the clone
    entirely when the directory already contains a ``Lib/`` tree.

    Returns the path to the ``Lib/`` directory inside the clone.
    """
    lib_path = corpus_dir / "Lib"
    if lib_path.is_dir():
        logger.info("Corpus already present at %s — skipping clone", lib_path)
        return lib_path

    corpus_dir.mkdir(parents=True, exist_ok=True)
    logger.info("Cloning CPython (sparse, depth 1) into %s …", corpus_dir)

    subprocess.run(
        [
            "git",
            "clone",
            "--depth",
            "1",
            "--filter=blob:none",
            "--sparse",
            _CPYTHON_REPO,
            str(corpus_dir),
        ],
        check=True,
        capture_output=True,
        text=True,
    )

    subprocess.run(
        ["git", "sparse-checkout", "set", "Lib/"],
        cwd=corpus_dir,
        check=True,
        capture_output=True,
        text=True,
    )

    logger.info("Corpus ready at %s", lib_path)
    return lib_path


def prepare_corpus(corpus_dir: Path, target_dir: Path) -> Path:
    """Copy the corpus ``Lib/`` tree into *target_dir* for a clean run.

    Returns the *target_dir* path.
    """
    lib_src = corpus_dir / "Lib"
    if not lib_src.is_dir():
        msg = f"Corpus not found at {lib_src} — run clone_cpython first"
        raise FileNotFoundError(msg)

    if target_dir.exists():
        shutil.rmtree(target_dir)

    logger.info("Copying corpus to %s …", target_dir)
    shutil.copytree(lib_src, target_dir)
    return target_dir
