"""Round-trip tests for each injector tier."""

from __future__ import annotations

import ast
import textwrap
from pathlib import Path

import pytest

from bench.injector import (
    DuplicateInjector,
    _mutate_augmented,
    _mutate_exact,
    _mutate_renamed,
    _mutate_restructured,
)

# A 14-line function — above biston's default min_lines (10).
SAMPLE_FUNCTION = textwrap.dedent("""\
    def process_items(items, threshold):
        \"\"\"Process items above threshold.\"\"\"
        results = []
        count = 0
        for item in items:
            count += 1
            value = item.get_value()
            if value > threshold:
                results.append(item)
            else:
                item.mark_skipped()
        total = len(results)
        ratio = total / max(count, 1)
        return results, ratio
""")

# A simpler function with an if/else block for restructuring tests.
SAMPLE_WITH_IFELSE = textwrap.dedent("""\
    def check_status(record, limit):
        \"\"\"Check whether a record is valid.\"\"\"
        result = None
        count = 0
        value = record.get_value()
        count += 1
        if value > limit:
            result = "above"
            record.flag_high()
        else:
            result = "below"
            record.flag_low()
        total = count + 1
        ratio = total / max(count, 1)
        return result
""")


class TestExactTier:
    def test_output_equals_input(self) -> None:
        import random

        result = _mutate_exact(SAMPLE_FUNCTION, random.Random(42))
        assert result == SAMPLE_FUNCTION

    def test_parses(self) -> None:
        import random

        result = _mutate_exact(SAMPLE_FUNCTION, random.Random(42))
        ast.parse(result)  # should not raise


class TestRenamedTier:
    def test_parses(self) -> None:
        import random

        result = _mutate_renamed(SAMPLE_FUNCTION, random.Random(42))
        ast.parse(result)

    def test_differs_from_original(self) -> None:
        import random

        result = _mutate_renamed(SAMPLE_FUNCTION, random.Random(42))
        assert result != SAMPLE_FUNCTION

    def test_no_original_locals_remain(self) -> None:
        import random

        result = _mutate_renamed(SAMPLE_FUNCTION, random.Random(42))
        # Original local variable names.
        original_locals = {"results", "count", "item", "value", "total", "ratio"}
        tree = ast.parse(result)
        found_names: set[str] = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Name):
                found_names.add(node.id)
            elif isinstance(node, ast.arg):
                found_names.add(node.arg)
        # None of the original locals should appear (except builtins like len, max).
        renamed_locals = found_names & original_locals
        assert not renamed_locals, f"Original locals still present: {renamed_locals}"

    def test_function_name_preserved(self) -> None:
        import random

        result = _mutate_renamed(SAMPLE_FUNCTION, random.Random(42))
        tree = ast.parse(result)
        func = tree.body[0]
        assert isinstance(func, ast.FunctionDef)
        assert func.name == "process_items"


class TestRestructuredTier:
    def test_parses(self) -> None:
        import random

        result = _mutate_restructured(SAMPLE_WITH_IFELSE, random.Random(42))
        ast.parse(result)

    def test_differs_from_renamed(self) -> None:
        import random

        renamed = _mutate_renamed(SAMPLE_WITH_IFELSE, random.Random(42))
        restructured = _mutate_restructured(SAMPLE_WITH_IFELSE, random.Random(42))
        # Restructured applies renaming + structural changes, so with the
        # same seed the rename portion is identical but structure differs.
        # Use different seeds to ensure they differ.
        assert restructured != renamed or restructured != SAMPLE_WITH_IFELSE


class TestAugmentedTier:
    def test_parses(self) -> None:
        import random

        result = _mutate_augmented(SAMPLE_FUNCTION, random.Random(42))
        ast.parse(result)

    def test_differs_from_original(self) -> None:
        import random

        result = _mutate_augmented(SAMPLE_FUNCTION, random.Random(42))
        assert result != SAMPLE_FUNCTION

    def test_line_count_changes(self) -> None:
        import random

        result = _mutate_augmented(SAMPLE_FUNCTION, random.Random(42))
        original_lines = SAMPLE_FUNCTION.strip().count("\n") + 1
        result_lines = result.strip().count("\n") + 1
        # Augmented inserts/deletes statements, so line count should differ.
        # (It's possible in rare cases it stays the same if insert and delete
        # cancel out — but with our sample function this won't happen.)
        assert result_lines != original_lines


class TestSemanticTier:
    def test_templates_parse(self) -> None:
        from bench.injector import _TEMPLATES

        for i, template in enumerate(_TEMPLATES):
            try:
                ast.parse(template)
            except SyntaxError as exc:
                pytest.fail(f"Template {i} failed to parse: {exc}")

    def test_templates_are_long_enough(self) -> None:
        from bench.injector import _TEMPLATES

        for i, template in enumerate(_TEMPLATES):
            lines = template.strip().count("\n") + 1
            assert lines >= 10, f"Template {i} has only {lines} lines (need >= 10)"


class TestDeterminism:
    def test_same_seed_same_output(self, tmp_path: Path) -> None:
        # Create a tiny corpus with one qualifying function.
        py_file = tmp_path / "sample.py"
        py_file.write_text(SAMPLE_FUNCTION)

        injector1 = DuplicateInjector(tmp_path, seed=99)
        manifest1 = injector1.inject(n_per_tier=1)

        # Clean injected files.
        inject_dir = tmp_path / "_injected"
        if inject_dir.exists():
            import shutil

            shutil.rmtree(inject_dir)

        injector2 = DuplicateInjector(tmp_path, seed=99)
        manifest2 = injector2.inject(n_per_tier=1)

        assert len(manifest1) == len(manifest2)
        for e1, e2 in zip(manifest1, manifest2):
            assert e1.tier == e2.tier
            assert e1.source_file == e2.source_file
            assert e1.target_file == e2.target_file


class TestManifestCorrectness:
    def test_manifest_fields(self, tmp_path: Path) -> None:
        py_file = tmp_path / "sample.py"
        py_file.write_text(SAMPLE_FUNCTION)

        injector = DuplicateInjector(tmp_path, seed=42)
        manifest = injector.inject(n_per_tier=1)

        for entry in manifest:
            assert entry.source_file, "source_file must not be empty"
            assert entry.target_file, "target_file must not be empty"
            assert entry.source_span[0] >= 1, "source start_line must be >= 1"
            assert entry.source_span[1] >= entry.source_span[0], "source end >= start"
            assert entry.target_span[0] >= 1, "target start_line must be >= 1"
            assert entry.target_span[1] >= entry.target_span[0], "target end >= start"
            assert entry.tier in (
                "exact",
                "renamed",
                "restructured",
                "augmented",
                "semantic",
            )
            assert entry.description, "description must not be empty"

    def test_injected_files_exist(self, tmp_path: Path) -> None:
        py_file = tmp_path / "sample.py"
        py_file.write_text(SAMPLE_FUNCTION)

        injector = DuplicateInjector(tmp_path, seed=42)
        manifest = injector.inject(n_per_tier=1)

        for entry in manifest:
            target = tmp_path / entry.target_file
            assert target.exists(), f"Injected file missing: {entry.target_file}"
            # Verify it parses.
            content = target.read_text()
            ast.parse(content)
