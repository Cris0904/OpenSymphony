#!/usr/bin/env python3
"""LOC-25 ``validate`` coverage for the converter's repo-routing contract.

These tests build minimal on-disk task packages from the fixture helpers
and exercise the full ``load_package`` -> ``validate_repo_routing`` path
so we can prove:

* a parent/leaf wave with inventory-matching slugs is accepted,
* out-of-inventory slugs are rejected with a stable error,
* missing leaf repos are rejected,
* parent-with-repo is rejected,
* exact-case slugs survive the parse unchanged,
* ``areas: ["repo:<slug>"]`` misuse is rejected,
* an explicit ``--project-set`` override is honoured rather than
  silently depending on the developer's local workspace.

No Linear I/O happens here; the assertions inspect the validation
errors produced before any publish call would fire.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

import yaml

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
sys.path.insert(0, str(REPO_ROOT / ".agents/skills/convert-tasks-to-linear/scripts"))
sys.path.insert(0, str(SCRIPT_DIR))

import convert_tasks_to_linear as ctl  # noqa: E402

from fixtures.wave_fixture import (  # noqa: E402
    TempRepoRoot,
    copy_fixture_project_set,
    write_manifest,
    write_task,
)


def _build_repo_root(
    tasks: list[dict[str, object]],
    *,
    project_set_source: str | None = "project-set-fixture.yaml",
) -> Path:
    """Materialise a fixture repo root with the supplied tasks.

    Returns the path to a directory owned by the caller. Tests must
    invoke :func:`_cleanup` (typically in a ``finally`` block) so the
    temp tree is removed even on failure.
    """

    import tempfile

    repo_root = Path(tempfile.mkdtemp(prefix="loc25-fixture-"))
    repo_root.mkdir(parents=True, exist_ok=True)
    entries: list[tuple[str, str]] = []
    for spec in tasks:
        file_path = write_task(
            repo_root=repo_root,
            task_id=str(spec["id"]),
            title=str(spec.get("title") or f"{spec['id']} title"),
            milestone=str(spec.get("milestone", "M1")),
            parent=spec.get("parent"),  # type: ignore[arg-type]
            areas=list(spec.get("areas") or []),  # type: ignore[arg-type]
            repo=spec.get("repo"),  # type: ignore[arg-type]
            filename=str(spec.get("filename") or f"docs/tasks/{spec['id']}.md"),
        )
        entries.append((str(spec["id"]), file_path))
    write_manifest(repo_root, entries)
    if project_set_source:
        copy_fixture_project_set(project_set_source, repo_root)
    return repo_root


def _cleanup(path: Path) -> None:
    import shutil

    shutil.rmtree(path, ignore_errors=True)


class ValidateRepoRoutingTests(unittest.TestCase):
    """Exercise ``load_package`` against fixture waves."""

    def _load(self, repo_root: Path, *, project_set: Path | None = None) -> ctl.Package:
        return ctl.load_package(
            repo_root,
            repo_root / "task-package.yaml",
            project_set_path=project_set,
        )

    def _expect_error(
        self, repo_root: Path, *, fragment: str, project_set: Path | None = None
    ) -> None:
        with self.assertRaises(ctl.ValidationError) as ctx:
            self._load(repo_root, project_set=project_set)
        self.assertIn(fragment, str(ctx.exception))

    # ---- valid parent/leaf shape ----------------------------------------

    def test_valid_parent_and_leaf_shape_passes(self) -> None:
        """A wave with two parents and two leaves (mixed-case slug) validates."""

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent one"},
                {"id": "TASK-2", "title": "Parent two"},
                {"id": "TASK-3", "title": "Leaf three", "parent": "TASK-1", "repo": "opensymphony"},
                {"id": "TASK-4", "title": "Leaf four", "parent": "TASK-2", "repo": "OpenSymphony-Config"},
            ],
        )
        try:
            package = self._load(repo_root)
            self.assertIsNotNone(package.repo_inventory)
            self.assertEqual(package.repo_inventory, {"opensymphony", "OpenSymphony-Config"})
            self.assertEqual(
                {slug for slug in (t.repo for t in package.tasks.values()) if slug},
                {"opensymphony", "OpenSymphony-Config"},
            )
        finally:
            _cleanup(repo_root)

    # ---- out-of-inventory slug ------------------------------------------

    def test_out_of_inventory_slug_is_rejected(self) -> None:
        """A leaf slug outside the project-set inventory fails fast."""

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent"},
                {"id": "TASK-2", "title": "Leaf", "parent": "TASK-1", "repo": "ghost-repo"},
            ],
        )
        try:
            with self.assertRaises(ctl.ValidationError) as ctx:
                self._load(repo_root)
            message = str(ctx.exception)
            self.assertIn("ghost-repo", message)
            self.assertIn("project-set inventory", message)
        finally:
            _cleanup(repo_root)

    # ---- missing leaf repo ----------------------------------------------

    def test_missing_leaf_repo_is_rejected(self) -> None:
        """A leaf that omits ``repo:`` is rejected."""

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent"},
                {"id": "TASK-2", "title": "Leaf without repo", "parent": "TASK-1", "repo": None},
            ],
        )
        try:
            self._expect_error(repo_root, fragment="TASK-2 is a leaf")
        finally:
            _cleanup(repo_root)

    # ---- parent-with-repo -----------------------------------------------

    def test_parent_with_repo_is_rejected(self) -> None:
        """A parent/review task carrying ``repo:`` is rejected."""

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent", "repo": "opensymphony"},
                {"id": "TASK-2", "title": "Leaf", "parent": "TASK-1", "repo": "opensymphony"},
            ],
        )
        try:
            self._expect_error(repo_root, fragment="TASK-1 is a parent/review task")
        finally:
            _cleanup(repo_root)

    def test_parent_with_empty_repo_is_rejected(self) -> None:
        """A parent with ``repo: ""`` is rejected (aligns Python with Rust).

        Review feedback (LOC-25 PR #15): the Rust manifest validator
        treats any non-``None`` declared repo on a parent as an error;
        Python previously passed ``repo: ""`` silently.
        """

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent", "repo": ""},
                {"id": "TASK-2", "title": "Leaf", "parent": "TASK-1", "repo": "opensymphony"},
            ],
        )
        try:
            self._expect_error(repo_root, fragment="TASK-1 is a parent/review task")
        finally:
            _cleanup(repo_root)

    def test_parent_with_whitespace_repo_is_rejected(self) -> None:
        """A parent with ``repo: "   "`` is rejected (same rationale)."""

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent", "repo": "   "},
                {"id": "TASK-2", "title": "Leaf", "parent": "TASK-1", "repo": "opensymphony"},
            ],
        )
        try:
            self._expect_error(repo_root, fragment="TASK-1 is a parent/review task")
        finally:
            _cleanup(repo_root)

    # ---- exact-case slug preservation -----------------------------------

    def test_exact_case_slug_is_preserved(self) -> None:
        """Mixed-case slug survives the parser character-for-character."""

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent"},
                {"id": "TASK-2", "title": "Leaf", "parent": "TASK-1", "repo": "OpenSymphony-Config"},
            ],
        )
        try:
            package = self._load(repo_root)
            self.assertEqual(package.tasks["TASK-2"].repo, "OpenSymphony-Config")
        finally:
            _cleanup(repo_root)

    # ---- reserved-namespace misuse in areas -----------------------------

    def test_repo_namespace_misuse_in_areas_is_rejected(self) -> None:
        """``areas: ["repo:<slug>"]`` is rejected as reserved-namespace misuse."""

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent", "areas": ["planning"]},
                {
                    "id": "TASK-2",
                    "title": "Leaf",
                    "parent": "TASK-1",
                    "areas": ["repo:opensymphony"],
                    "repo": "opensymphony",
                },
            ],
        )
        try:
            with self.assertRaises(ctl.ValidationError) as ctx:
                self._load(repo_root)
            message = str(ctx.exception)
            self.assertIn("reserved non-area namespace 'repo:'", message)
        finally:
            _cleanup(repo_root)

    # ---- explicit --project-set override --------------------------------

    def test_explicit_project_set_override_is_used(self) -> None:
        """An explicit ``--project-set`` path is honoured even when the developer's
        default ``.opensymphony/project-set.yaml`` contains a different inventory."""

        import tempfile

        repo_root = Path(tempfile.mkdtemp(prefix="loc25-override-"))
        try:
            # Default workspace inventory only contains ``opensymphony``.
            copy_fixture_project_set("project-set-limited.yaml", repo_root)
            write_task(
                repo_root=repo_root,
                task_id="TASK-1",
                title="Parent",
                parent=None,
                areas=[],
                repo=None,
            )
            write_task(
                repo_root=repo_root,
                task_id="TASK-2",
                title="Mixed-case leaf",
                parent="TASK-1",
                areas=[],
                repo="OpenSymphony-Config",
            )
            write_manifest(
                repo_root,
                [
                    ("TASK-1", "docs/tasks/task-1.md"),
                    ("TASK-2", "docs/tasks/task-2.md"),
                ],
            )

            # 1. Without the override the mixed-case leaf must fail because
            #    the default workspace inventory only knows ``opensymphony``.
            with self.assertRaises(ctl.ValidationError) as ctx:
                self._load(repo_root)
            self.assertIn("OpenSymphony-Config", str(ctx.exception))

            # 2. With the override pointing at the two-repo fixture the
            #    package now validates. The package must also surface the
            #    override path so dry-run output can prove the gating
            #    inventory came from the override, not the workspace.
            override = REPO_ROOT / "tests/python/fixtures/project-set-fixture.yaml"
            package = self._load(repo_root, project_set=override)
            self.assertEqual(
                package.repo_inventory,
                {"opensymphony", "OpenSymphony-Config"},
            )
            self.assertEqual(
                package.repo_inventory_source.resolve(),
                override.resolve(),
            )
        finally:
            _cleanup(repo_root)


class ProjectSetLoaderTests(unittest.TestCase):
    """Exercise ``load_project_set_inventory`` directly."""

    def test_default_path_loads_when_present(self) -> None:
        """Default ``<repo>/.opensymphony/project-set.yaml`` loads with slug set."""

        with TempRepoRoot() as repo_root:
            copy_fixture_project_set("project-set-fixture.yaml", repo_root)
            errors: list[str] = []
            inventory, source = ctl.load_project_set_inventory(
                repo_root, None, errors
            )
            self.assertEqual(errors, [])
            self.assertEqual(inventory, {"opensymphony", "OpenSymphony-Config"})
            self.assertEqual(
                source,
                (repo_root / ctl.DEFAULT_PROJECT_SET_PATH).resolve(),
            )

    def test_missing_default_file_appends_error(self) -> None:
        """Missing default inventory is fail-fast: an error is appended so a
        planning wave cannot silently skip the out-of-inventory check.

        Review feedback (LOC-25 PR #15): previously the loader returned
        ``(None, source)`` with no error, so ``validate_repo_routing``
        silently skipped the inventory-membership check. Now the default
        path is mandatory and an error is appended.
        """

        with TempRepoRoot() as repo_root:
            errors: list[str] = []
            inventory, source = ctl.load_project_set_inventory(
                repo_root, None, errors
            )
            self.assertIsNone(inventory)
            self.assertTrue(source.is_absolute())
            self.assertTrue(
                any("project-set file" in err and "missing" in err for err in errors),
                f"expected a missing-inventory error, got {errors!r}",
            )

    def test_missing_override_file_appends_error(self) -> None:
        """Explicit ``--project-set`` overrides must point at a real file.

        Review feedback (LOC-25 PR #15): a missing override path is also
        an error so operators cannot accidentally disable the inventory
        gate by passing a non-existent path.
        """

        with TempRepoRoot() as repo_root:
            override = repo_root / "does-not-exist.yaml"
            errors: list[str] = []
            inventory, source = ctl.load_project_set_inventory(
                repo_root, override, errors
            )
            self.assertIsNone(inventory)
            self.assertEqual(source, override.resolve())
            self.assertTrue(
                any("override" in err and "does not exist" in err for err in errors),
                f"expected a missing-override error, got {errors!r}",
            )

    def test_malformed_yaml_appends_error(self) -> None:
        """Unreadable inventory surfaces a validation error rather than crashing."""

        with TempRepoRoot() as repo_root:
            config_dir = repo_root / ".opensymphony"
            config_dir.mkdir(parents=True, exist_ok=True)
            (config_dir / "project-set.yaml").write_text(
                "this: is: not: valid: yaml:\n  - oops",
                encoding="utf-8",
            )
            errors: list[str] = []
            inventory, _ = ctl.load_project_set_inventory(repo_root, None, errors)
            self.assertIsNone(inventory)
            self.assertTrue(any("unreadable" in err for err in errors))

    def test_zero_repos_appends_error(self) -> None:
        """An inventory with no repos cannot gate routing."""

        with TempRepoRoot() as repo_root:
            config_dir = repo_root / ".opensymphony"
            config_dir.mkdir(parents=True, exist_ok=True)
            (config_dir / "project-set.yaml").write_text(
                "schema_version: 1\nproject_set:\n  slug: empty\n  name: Empty\n  projects: []\n",
                encoding="utf-8",
            )
            errors: list[str] = []
            inventory, _ = ctl.load_project_set_inventory(repo_root, None, errors)
            self.assertIsNone(inventory)
            self.assertTrue(any("zero repos" in err for err in errors))


class DesiredRepoByTaskTests(unittest.TestCase):
    """``build_desired_repo_by_task`` projects the validator's intent."""

    def test_leaves_get_managed_and_parents_get_cleared(self) -> None:
        leaves = [
            ctl.Task(
                id="TASK-1",
                file="docs/tasks/task-1.md",
                path=Path("docs/tasks/task-1.md"),
                title="t",
                milestone="M1",
                priority=3,
                estimate=1,
                blocked_by=[],
                blocks=[],
                areas=[],
                parent=None,
                body="",
                repo="opensymphony",
            ),
            ctl.Task(
                id="TASK-2",
                file="docs/tasks/task-2.md",
                path=Path("docs/tasks/task-2.md"),
                title="t",
                milestone="M1",
                priority=3,
                estimate=1,
                blocked_by=[],
                blocks=[],
                areas=[],
                parent="TASK-1",
                body="",
                repo="opensymphony",
            ),
        ]
        desired = ctl.build_desired_repo_by_task({t.id: t for t in leaves})
        self.assertEqual(desired["TASK-1"].kind, "cleared")
        self.assertEqual(desired["TASK-2"].kind, "managed")
        self.assertEqual(desired["TASK-2"].slug, "opensymphony")


class DryRunRepoLabelEmissionTests(unittest.TestCase):
    """``dry-run`` proves the wave emits ``repo:<slug>`` labels without Linear writes."""

    def test_dry_run_emits_repo_labels_without_linear_writes(self) -> None:
        """Dry-run output lists every managed slug without contacting Linear."""

        repo_root = _build_repo_root(
            tasks=[
                {"id": "TASK-1", "title": "Parent"},
                {"id": "TASK-2", "title": "Leaf A", "parent": "TASK-1", "repo": "opensymphony"},
                {
                    "id": "TASK-3",
                    "title": "Leaf B (mixed case)",
                    "parent": "TASK-1",
                    "repo": "OpenSymphony-Config",
                },
            ],
        )
        try:
            package = ctl.load_package(
                repo_root,
                repo_root / "task-package.yaml",
            )
            import io
            import contextlib

            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                ctl.print_dry_run(package)
            output = buffer.getvalue()

            self.assertIn("Repo routing (LOC-25):", output)
            self.assertIn("TASK-1 repo=-", output)
            self.assertIn("TASK-2 repo=opensymphony", output)
            self.assertIn("TASK-3 repo=OpenSymphony-Config", output)
            self.assertIn("Repo labels to publish (managed):", output)
            self.assertIn("- repo:opensymphony", output)
            self.assertIn("- repo:OpenSymphony-Config", output)
        finally:
            _cleanup(repo_root)


if __name__ == "__main__":
    unittest.main()
