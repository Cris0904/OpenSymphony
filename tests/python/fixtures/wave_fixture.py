#!/usr/bin/env python3
"""Build on-disk fixture task packages for the LOC-25 converter tests.

Each builder writes a temporary directory containing:

* ``task-package.yaml`` referencing the task files,
* one or more ``docs/tasks/<id>.md`` task files with explicit
  ``areas`` and ``repo`` frontmatter (matching the contract the
  planning skill publishes), and
* an optional ``.opensymphony/project-set.yaml`` so ``validate`` can
  load the inventory without the operator having to set up a real
  project set.

The fixtures live in dedicated helpers rather than ``setUp`` methods
so individual tests can describe exactly the shape they exercise:
parent-with-repo, missing leaf repo, out-of-inventory slug, etc.
"""

from __future__ import annotations

import shutil
import tempfile
import textwrap
from pathlib import Path
from typing import Any


TASK_TEMPLATE = textwrap.dedent(
    """\
    ---
    id: {task_id}
    title: "{title}"
    milestone: "{milestone}"
    priority: 3
    estimate: 1
    blockedBy: []
    blocks: []
    parent: {parent_yaml}
    areas: {areas_yaml}
    repo: {repo_yaml}
    ---

    ## Summary

    {task_id} body.

    ## Scope

    ### In scope

    - TBD

    ### Out of scope

    - TBD

    ## Deliverables

    - TBD

    ## Acceptance Criteria

    - [ ] TBD

    ## Test Plan

    - TBD

    ## Context

    - TBD

    ## Definition of Ready

    - [ ] TBD
    """
)


def _format_yaml_value(value: Any) -> str:
    """Render a Python value as a YAML scalar suitable for the fixture template."""

    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, str):
        escaped = value.replace("\\", "\\\\").replace('"', '\\"')
        return f'"{escaped}"'
    if isinstance(value, list):
        if not value:
            return "[]"
        return "[" + ", ".join(_format_yaml_value(item) for item in value) + "]"
    raise TypeError(f"unsupported fixture value: {value!r}")


def write_task(
    *,
    repo_root: Path,
    task_id: str,
    title: str | None = None,
    milestone: str = "M1",
    parent: str | None = None,
    areas: list[str] | None = None,
    repo: str | None = None,
    filename: str | None = None,
) -> str:
    """Write a single task file and return the manifest-relative path."""

    relative = filename or f"docs/tasks/{task_id.lower()}.md"
    body = TASK_TEMPLATE.format(
        task_id=task_id,
        title=title or f"{task_id} title",
        milestone=milestone,
        parent_yaml=_format_yaml_value(parent),
        areas_yaml=_format_yaml_value(areas if areas is not None else []),
        repo_yaml=_format_yaml_value(repo),
    )
    full = repo_root / relative
    full.parent.mkdir(parents=True, exist_ok=True)
    full.write_text(body, encoding="utf-8")
    return relative


def write_manifest(repo_root: Path, task_entries: list[tuple[str, str]]) -> None:
    """Write a minimal ``task-package.yaml`` referencing the given tasks."""

    manifest = ["planningWave: test-wave", "tasksDir: docs/tasks", "milestones:", "  - \"M1\"", "tasks:"]
    for task_id, file_path in task_entries:
        manifest.append(f"  - id: {task_id}")
        manifest.append(f"    file: {file_path}")
    (repo_root / "task-package.yaml").write_text(
        "\n".join(manifest) + "\n", encoding="utf-8"
    )


def write_project_set(repo_root: Path, repos: list[tuple[str, str]]) -> Path:
    """Write a minimal project-set inventory file and return the path."""

    config_dir = repo_root / ".opensymphony"
    config_dir.mkdir(parents=True, exist_ok=True)
    path = config_dir / "project-set.yaml"
    lines = ["schema_version: 1", "", "project_set:", "  slug: fixture", "  name: Fixture"]
    lines.append("  projects:")
    lines.append("    - slug: opensymphony")
    lines.append("      name: OpenSymphony")
    lines.append("      repos:")
    for slug, url in repos:
        lines.append(f"        - slug: {slug}")
        lines.append(f"          url: {url}")
        lines.append("          default_branch: main")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


class TempRepoRoot:
    """Context manager that yields a temp directory and cleans it up on exit."""

    def __init__(self) -> None:
        self._tmp: tempfile.TemporaryDirectory | None = None
        self.path: Path | None = None

    def __enter__(self) -> Path:
        self._tmp = tempfile.TemporaryDirectory()
        self.path = Path(self._tmp.__enter__())
        return self.path

    def __exit__(self, *exc_info: Any) -> None:
        if self._tmp is not None:
            self._tmp.__exit__(*exc_info)
            self._tmp = None
            self.path = None


def copy_fixture_project_set(source_name: str, dest_root: Path) -> Path:
    """Copy a fixture ``project-set.yaml`` into ``<dest>/.opensymphony/``."""

    fixtures_dir = Path(__file__).resolve().parent
    src = fixtures_dir / source_name
    if not src.is_file():
        raise FileNotFoundError(f"missing fixture project-set: {src}")
    config_dir = dest_root / ".opensymphony"
    config_dir.mkdir(parents=True, exist_ok=True)
    dest = config_dir / "project-set.yaml"
    shutil.copyfile(src, dest)
    return dest
