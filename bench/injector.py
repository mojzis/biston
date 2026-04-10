"""Inject synthetic duplicates into a Python corpus at controlled similarity levels."""

from __future__ import annotations

import ast
import copy
import logging
import random
import string
import textwrap
from dataclasses import dataclass
from pathlib import Path

from bench import Manifest, ManifestEntry

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Source function metadata
# ---------------------------------------------------------------------------

@dataclass
class SourceFunction:
    """A function extracted from the corpus suitable for duplication."""

    file_path: str  # relative to corpus dir
    name: str
    source_text: str
    start_line: int  # 1-indexed
    end_line: int  # 1-indexed


# ---------------------------------------------------------------------------
# AST helpers
# ---------------------------------------------------------------------------

class _NameCollector(ast.NodeVisitor):
    """Collect local variable and parameter names from a function AST."""

    def __init__(self) -> None:
        self.locals: set[str] = set()

    def visit_FunctionDef(self, node: ast.FunctionDef) -> None:  # noqa: N802
        # Collect parameter names.
        for arg in node.args.args + node.args.posonlyargs + node.args.kwonlyargs:
            self.locals.add(arg.arg)
        if node.args.vararg:
            self.locals.add(node.args.vararg.arg)
        if node.args.kwarg:
            self.locals.add(node.args.kwarg.arg)
        self.generic_visit(node)

    visit_AsyncFunctionDef = visit_FunctionDef  # noqa: N815

    def visit_Name(self, node: ast.Name) -> None:  # noqa: N802
        if isinstance(node.ctx, ast.Store):
            self.locals.add(node.id)


class _NameRenamer(ast.NodeTransformer):
    """Rename identifiers according to a mapping."""

    def __init__(self, rename_map: dict[str, str]) -> None:
        self._map = rename_map

    def visit_FunctionDef(self, node: ast.FunctionDef) -> ast.FunctionDef:  # noqa: N802
        for arg in node.args.args + node.args.posonlyargs + node.args.kwonlyargs:
            if arg.arg in self._map:
                arg.arg = self._map[arg.arg]
        if node.args.vararg and node.args.vararg.arg in self._map:
            node.args.vararg.arg = self._map[node.args.vararg.arg]
        if node.args.kwarg and node.args.kwarg.arg in self._map:
            node.args.kwarg.arg = self._map[node.args.kwarg.arg]
        self.generic_visit(node)
        return node

    visit_AsyncFunctionDef = visit_FunctionDef  # noqa: N815

    def visit_Name(self, node: ast.Name) -> ast.Name:  # noqa: N802
        if node.id in self._map:
            node.id = self._map[node.id]
        return node


class _IfElseSwapper(ast.NodeTransformer):
    """Swap the first if/else branch found in a function body."""

    def __init__(self, rng: random.Random) -> None:
        self._rng = rng
        self._swapped = False

    def visit_If(self, node: ast.If) -> ast.If:  # noqa: N802
        self.generic_visit(node)
        if not self._swapped and node.orelse and not any(
            isinstance(n, ast.If) for n in node.orelse
        ):
            # Negate test, swap body and orelse.
            node.test = ast.UnaryOp(op=ast.Not(), operand=node.test)
            node.body, node.orelse = node.orelse, node.body
            self._swapped = True
        return node


def _collect_locals(tree: ast.AST) -> set[str]:
    collector = _NameCollector()
    collector.visit(tree)
    return collector.locals


def _rename_locals(tree: ast.AST, rng: random.Random) -> ast.AST:
    """Rename all local variables/params to random names."""
    tree = copy.deepcopy(tree)
    names = _collect_locals(tree)
    # Don't rename the function name itself — only locals and params.
    func_node = tree.body[0] if isinstance(tree, ast.Module) else tree
    if isinstance(func_node, (ast.FunctionDef, ast.AsyncFunctionDef)):
        names.discard(func_node.name)

    rename_map: dict[str, str] = {}
    for name in sorted(names):  # sort for determinism
        new = "v_" + "".join(rng.choices(string.ascii_lowercase, k=4))
        while new in rename_map.values():
            new = "v_" + "".join(rng.choices(string.ascii_lowercase, k=4))
        rename_map[name] = new

    renamer = _NameRenamer(rename_map)
    renamer.visit(tree)
    ast.fix_missing_locations(tree)
    return tree


def _swap_if_else(tree: ast.AST, rng: random.Random) -> ast.AST:
    """Swap the first if/else branch in the function body."""
    tree = copy.deepcopy(tree)
    swapper = _IfElseSwapper(rng)
    swapper.visit(tree)
    ast.fix_missing_locations(tree)
    return tree


def _reorder_stmts(tree: ast.AST, rng: random.Random) -> ast.AST:
    """Reorder independent consecutive assignment statements."""
    tree = copy.deepcopy(tree)

    func_node = tree.body[0] if isinstance(tree, ast.Module) else tree
    if not isinstance(func_node, (ast.FunctionDef, ast.AsyncFunctionDef)):
        return tree

    body = func_node.body
    # Find pairs of consecutive simple assignments that are independent.
    candidates: list[int] = []
    for i in range(len(body) - 1):
        a, b = body[i], body[i + 1]
        if isinstance(a, ast.Assign) and isinstance(b, ast.Assign):
            a_targets = {
                n.id for n in ast.walk(a) if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Store)
            }
            b_values = {
                n.id for n in ast.walk(b.value) if isinstance(n, ast.Name)
            }
            if not a_targets & b_values:
                candidates.append(i)

    if candidates:
        idx = rng.choice(candidates)
        body[idx], body[idx + 1] = body[idx + 1], body[idx]

    ast.fix_missing_locations(tree)
    return tree


def _augment_insert(tree: ast.AST, rng: random.Random) -> ast.AST:
    """Insert 1-2 benign statements into the function body."""
    tree = copy.deepcopy(tree)

    func_node = tree.body[0] if isinstance(tree, ast.Module) else tree
    if not isinstance(func_node, (ast.FunctionDef, ast.AsyncFunctionDef)):
        return tree

    body = func_node.body
    n_inserts = rng.randint(1, 2)
    fillers = [
        ast.parse("_guard = True").body[0],
        ast.parse("_log = None").body[0],
        ast.Pass(),
    ]
    for _ in range(n_inserts):
        stmt = copy.deepcopy(rng.choice(fillers))
        # Insert at a random position, but not at index 0 (before docstring).
        pos = rng.randint(1, max(1, len(body) - 1))
        body.insert(pos, stmt)

    ast.fix_missing_locations(tree)
    return tree


def _augment_delete(tree: ast.AST, rng: random.Random) -> ast.AST:
    """Delete one non-critical statement from the function body."""
    tree = copy.deepcopy(tree)

    func_node = tree.body[0] if isinstance(tree, ast.Module) else tree
    if not isinstance(func_node, (ast.FunctionDef, ast.AsyncFunctionDef)):
        return tree

    body = func_node.body
    # Candidates: simple Assign or Expr statements, not first or last.
    candidates = [
        i
        for i in range(1, len(body) - 1)
        if isinstance(body[i], (ast.Assign, ast.Expr))
    ]
    if candidates:
        idx = rng.choice(candidates)
        del body[idx]

    ast.fix_missing_locations(tree)
    return tree


# ---------------------------------------------------------------------------
# Semantic tier — template bank
# ---------------------------------------------------------------------------

_TEMPLATES: list[str] = [
    textwrap.dedent("""\
    def filter_collection(items, predicate):
        \"\"\"Filter items matching a predicate.\"\"\"
        result = []
        skipped = 0
        processed = 0
        for item in items:
            processed += 1
            if predicate(item):
                result.append(item)
            else:
                skipped += 1
        total = len(result)
        ratio = total / max(processed, 1)
        return result
    """),
    textwrap.dedent("""\
    def build_mapping(pairs, key_fn, value_fn):
        \"\"\"Build a dictionary from pairs using key/value functions.\"\"\"
        mapping = {}
        duplicates = 0
        processed = 0
        for pair in pairs:
            processed += 1
            k = key_fn(pair)
            v = value_fn(pair)
            if k in mapping:
                duplicates += 1
            mapping[k] = v
        total = len(mapping)
        dup_ratio = duplicates / max(processed, 1)
        return mapping
    """),
    textwrap.dedent("""\
    def accumulate_values(records, extractor, initial):
        \"\"\"Accumulate values from records using an extractor function.\"\"\"
        total = initial
        count = 0
        errors = 0
        for record in records:
            count += 1
            try:
                value = extractor(record)
                total = total + value
            except Exception:
                errors += 1
        success_rate = (count - errors) / max(count, 1)
        avg = total / max(count - errors, 1)
        return total
    """),
    textwrap.dedent("""\
    def validate_entries(entries, rules):
        \"\"\"Validate entries against a list of rules.\"\"\"
        valid = []
        invalid = []
        skipped = 0
        for entry in entries:
            passed = True
            for rule in rules:
                if not rule(entry):
                    passed = False
                    break
            if passed:
                valid.append(entry)
            else:
                invalid.append(entry)
        total = len(valid) + len(invalid)
        pass_rate = len(valid) / max(total, 1)
        return valid, invalid
    """),
    textwrap.dedent("""\
    def read_and_parse_lines(filepath, parser, skip_blank):
        \"\"\"Read a file line by line and parse each line.\"\"\"
        results = []
        line_num = 0
        errors = 0
        with open(filepath) as fh:
            for raw_line in fh:
                line_num += 1
                stripped = raw_line.strip()
                if skip_blank and not stripped:
                    continue
                try:
                    parsed = parser(stripped)
                    results.append(parsed)
                except Exception:
                    errors += 1
        success = line_num - errors
        ratio = success / max(line_num, 1)
        return results
    """),
    textwrap.dedent("""\
    def retry_operation(fn, max_attempts, delay_base):
        \"\"\"Retry a callable with exponential back-off.\"\"\"
        import time
        last_err = None
        attempt = 0
        succeeded = False
        while attempt < max_attempts:
            attempt += 1
            try:
                result = fn()
                succeeded = True
                return result
            except Exception as exc:
                last_err = exc
                wait = delay_base * (2 ** (attempt - 1))
                time.sleep(wait)
        if not succeeded:
            raise RuntimeError("exhausted retries") from last_err
        return None
    """),
    textwrap.dedent("""\
    def group_by_key(items, key_fn):
        \"\"\"Group items into lists keyed by key_fn result.\"\"\"
        groups = {}
        ungrouped = 0
        total = 0
        for item in items:
            total += 1
            k = key_fn(item)
            if k is None:
                ungrouped += 1
                continue
            if k not in groups:
                groups[k] = []
            groups[k].append(item)
        grouped_count = total - ungrouped
        ratio = grouped_count / max(total, 1)
        return groups
    """),
    textwrap.dedent("""\
    def flatten_nested(nested, depth_limit):
        \"\"\"Flatten a nested list up to a given depth.\"\"\"
        result = []
        stack = [(nested, 0)]
        iterations = 0
        while stack:
            iterations += 1
            current, depth = stack.pop()
            if isinstance(current, list) and depth < depth_limit:
                for item in reversed(current):
                    stack.append((item, depth + 1))
            else:
                result.append(current)
        total = len(result)
        ratio = total / max(iterations, 1)
        return result
    """),
    textwrap.dedent("""\
    def deduplicate_items(items, key_fn):
        \"\"\"Remove duplicate items based on a key function.\"\"\"
        seen = set()
        unique = []
        duplicates = 0
        total = 0
        for item in items:
            total += 1
            k = key_fn(item)
            if k in seen:
                duplicates += 1
                continue
            seen.add(k)
            unique.append(item)
        dup_rate = duplicates / max(total, 1)
        unique_count = len(unique)
        return unique
    """),
    textwrap.dedent("""\
    def merge_sorted_lists(left, right, key_fn):
        \"\"\"Merge two sorted lists into one sorted list.\"\"\"
        result = []
        i = 0
        j = 0
        comparisons = 0
        while i < len(left) and j < len(right):
            comparisons += 1
            if key_fn(left[i]) <= key_fn(right[j]):
                result.append(left[i])
                i += 1
            else:
                result.append(right[j])
                j += 1
        result.extend(left[i:])
        result.extend(right[j:])
        total = len(result)
        return result
    """),
]


# ---------------------------------------------------------------------------
# Mutation pipeline per tier
# ---------------------------------------------------------------------------

def _mutate_exact(source: str, _rng: random.Random) -> str:
    """Tier 1: verbatim copy."""
    return source


def _mutate_renamed(source: str, rng: random.Random) -> str:
    """Tier 2: rename local variables and parameters."""
    tree = ast.parse(source)
    tree = _rename_locals(tree, rng)
    return ast.unparse(tree)


def _mutate_restructured(source: str, rng: random.Random) -> str:
    """Tier 3: rename + mild structural edits."""
    tree = ast.parse(source)
    tree = _rename_locals(tree, rng)
    tree = _swap_if_else(tree, rng)
    tree = _reorder_stmts(tree, rng)
    return ast.unparse(tree)


def _mutate_augmented(source: str, rng: random.Random) -> str:
    """Tier 4: restructured + insert/delete statements."""
    tree = ast.parse(source)
    tree = _rename_locals(tree, rng)
    tree = _swap_if_else(tree, rng)
    tree = _reorder_stmts(tree, rng)
    tree = _augment_insert(tree, rng)
    tree = _augment_delete(tree, rng)
    return ast.unparse(tree)


_TIER_MUTATORS: dict[str, object] = {
    "exact": _mutate_exact,
    "renamed": _mutate_renamed,
    "restructured": _mutate_restructured,
    "augmented": _mutate_augmented,
}


# ---------------------------------------------------------------------------
# DuplicateInjector
# ---------------------------------------------------------------------------

class DuplicateInjector:
    """Inject synthetic duplicates into a corpus directory."""

    def __init__(self, corpus_dir: Path, seed: int = 42) -> None:
        self._corpus_dir = corpus_dir
        self._rng = random.Random(seed)
        self._sources: list[SourceFunction] = []

    # -- public API ----------------------------------------------------------

    def inject(self, n_per_tier: int = 10) -> Manifest:
        """Inject duplicates and return the ground-truth manifest."""
        self._discover_sources()
        needed = n_per_tier * len(_TIER_MUTATORS) + n_per_tier  # 4 mutated + semantic
        if len(self._sources) < needed:
            logger.warning(
                "Only %d source functions available (need %d); reducing n_per_tier",
                len(self._sources),
                needed,
            )
            n_per_tier = len(self._sources) // 5 or 1

        self._rng.shuffle(self._sources)

        inject_dir = self._corpus_dir / "_injected"
        inject_dir.mkdir(exist_ok=True)

        manifest: Manifest = []
        idx = 0

        # Tiers 1-4: AST-mutated duplicates.
        for tier_name, mutator in _TIER_MUTATORS.items():
            for i in range(n_per_tier):
                if idx >= len(self._sources):
                    break
                src = self._sources[idx]
                idx += 1

                mutated = mutator(src.source_text, self._rng)  # type: ignore[operator]

                # Validate the result parses.
                try:
                    ast.parse(mutated)
                except SyntaxError:
                    logger.warning(
                        "Tier %s mutation produced invalid Python for %s; skipping",
                        tier_name,
                        src.name,
                    )
                    continue

                target_file_rel = f"_injected/{tier_name}_{i:03d}.py"
                target_path = self._corpus_dir / target_file_rel
                target_path.write_text(mutated, encoding="utf-8")

                # Compute target span (1-indexed) from the AST.
                target_tree = ast.parse(mutated)
                target_func = target_tree.body[0]
                target_end = target_func.end_lineno or target_func.lineno
                target_span = (target_func.lineno, target_end)

                manifest.append(
                    ManifestEntry(
                        source_file=src.file_path,
                        source_span=(src.start_line, src.end_line),
                        target_file=target_file_rel,
                        target_span=target_span,
                        tier=tier_name,
                        description=f"{tier_name} copy of {src.name} from {src.file_path}",
                    )
                )

        # Tier 5: semantic (template bank).
        manifest.extend(self._inject_semantic(n_per_tier, inject_dir))

        logger.info("Injected %d duplicates across all tiers", len(manifest))
        return manifest

    # -- internals -----------------------------------------------------------

    def _discover_sources(self) -> None:
        """Walk the corpus and find functions suitable for duplication."""
        self._sources = []
        for py_file in sorted(self._corpus_dir.rglob("*.py")):
            # Skip test files — biston excludes tests/ by default.
            rel = py_file.relative_to(self._corpus_dir)
            rel_str = str(rel)
            if rel_str.startswith("test") or "/test" in rel_str:
                continue
            if rel_str.startswith("_injected"):
                continue

            try:
                source = py_file.read_text(encoding="utf-8", errors="replace")
                tree = ast.parse(source)
            except (SyntaxError, UnicodeDecodeError):
                continue

            for node in ast.iter_child_nodes(tree):
                if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                start = node.lineno
                end = node.end_lineno or start
                n_lines = end - start + 1
                if n_lines < 12:
                    continue
                if len(node.body) < 3:
                    continue

                # Extract source text for this function.
                lines = source.splitlines(keepends=True)
                func_source = "".join(lines[start - 1 : end])
                # Dedent in case of indentation (top-level should be fine, but be safe).
                func_source = textwrap.dedent(func_source)

                # Verify it parses standalone.
                try:
                    ast.parse(func_source)
                except SyntaxError:
                    continue

                self._sources.append(
                    SourceFunction(
                        file_path=str(rel),
                        name=node.name,
                        source_text=func_source,
                        start_line=start,
                        end_line=end,
                    )
                )

        logger.info("Discovered %d candidate source functions", len(self._sources))

    def _inject_semantic(self, n: int, inject_dir: Path) -> Manifest:
        """Inject tier-5 (semantic) duplicates from the template bank."""
        manifest: Manifest = []
        if not _TEMPLATES:
            logger.warning("No semantic templates available; skipping tier 5")
            return manifest

        templates = list(_TEMPLATES)
        self._rng.shuffle(templates)

        for i in range(min(n, len(templates))):
            template_src = templates[i]

            # Apply minor parametric variation: rename the function.
            tree = ast.parse(template_src)
            func_node = tree.body[0]
            original_name = func_node.name  # type: ignore[union-attr]
            new_name = f"bench_semantic_{i:03d}"
            func_node.name = new_name  # type: ignore[union-attr]

            # Also rename locals for variation.
            tree = _rename_locals(tree, self._rng)
            # Restore function name (rename_locals skips it, but just be sure).
            tree.body[0].name = new_name  # type: ignore[union-attr]

            mutated = ast.unparse(tree)

            try:
                ast.parse(mutated)
            except SyntaxError:
                logger.warning("Semantic template %d produced invalid Python; skipping", i)
                continue

            target_file_rel = f"_injected/semantic_{i:03d}.py"
            target_path = inject_dir / f"semantic_{i:03d}.py"
            target_path.write_text(mutated, encoding="utf-8")

            target_tree = ast.parse(mutated)
            target_func = target_tree.body[0]
            target_end = target_func.end_lineno or target_func.lineno
            target_span = (target_func.lineno, target_end)

            # For semantic tier, source is the original template.
            # We write the original template to a separate file as the "source".
            source_file_rel = f"_injected/semantic_{i:03d}_src.py"
            source_path = inject_dir / f"semantic_{i:03d}_src.py"
            source_path.write_text(template_src, encoding="utf-8")
            source_tree = ast.parse(template_src)
            source_func = source_tree.body[0]
            source_end = source_func.end_lineno or source_func.lineno

            manifest.append(
                ManifestEntry(
                    source_file=source_file_rel,
                    source_span=(source_func.lineno, source_end),
                    target_file=target_file_rel,
                    target_span=target_span,
                    tier="semantic",
                    description=f"semantic rewrite of template {original_name}",
                )
            )

        return manifest
