#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["PyYAML>=6.0.2"]
# ///
"""Validate and publish OpenSymphony task packages to Linear."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from collections import defaultdict, deque
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import yaml

# Make sibling ``label_merge`` importable when this script is run via
# ``uv run --script`` (PEP 723). The skill scripts folder is on sys.path so
# tests and direct ``python3`` invocations both work.
_SCRIPT_DIR = Path(__file__).resolve().parent
if str(_SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPT_DIR))

from label_merge import (  # noqa: E402  - intentional sys.path tweak above
    AREA_PREFIX,
    REPO_PREFIX,
    DesiredRepo,
    merge_label_ids,
)


REQUIRED_FRONTMATTER = [
    "id",
    "title",
    "milestone",
    "priority",
    "estimate",
    "blockedBy",
    "blocks",
    "parent",
]
REQUIRED_SECTIONS = [
    "Summary",
    "Scope",
    "Deliverables",
    "Acceptance Criteria",
    "Test Plan",
    "Context",
    "Definition of Ready",
]
PRIORITY_NAMES = {
    0: "No priority",
    1: "Urgent",
    2: "High",
    3: "Normal",
    4: "Low",
}


@dataclass(frozen=True)
class ManifestTask:
    id: str
    file: str


@dataclass
class Task:
    id: str
    file: str
    path: Path
    title: str
    milestone: str
    priority: int
    estimate: int
    blocked_by: list[str]
    blocks: list[str]
    areas: list[str]
    parent: str | None
    body: str
    # LOC-25: the project-set repo slug (exact inventory key) for leaf
    # tasks, or None for parent/review tasks. The validator enforces the
    # leaf-vs-parent contract; ``None`` here means the frontmatter did
    # not carry a ``repo:`` field, which is only valid for parents.
    repo: str | None = None


@dataclass
class Package:
    manifest_path: Path
    repo_root: Path
    planning_wave: str
    tasks_dir: str
    milestones: list[str]
    manifest_tasks: list[ManifestTask]
    tasks: dict[str, Task]
    waves: list[list[str]]
    # LOC-25: set of allowed repo slugs loaded from
    # ``<repo_root>/.opensymphony/project-set.yaml``. ``None`` means the
    # inventory was not loaded (e.g. the file is missing); in that case
    # the validator cannot enforce inventory membership and the
    # converter reports that as a separate validation error so callers
    # know to onboard the project set.
    repo_inventory: set[str] | None = None
    # LOC-25: optional explicit override path the validator used to load
    # the inventory. When ``None``, the validator fell back to the
    # default location. Captured so dry-run/apply output can confirm
    # which inventory source actually gated the wave.
    repo_inventory_source: Path | None = None


class ValidationError(Exception):
    """Raised when the task package is invalid."""


class LinearError(Exception):
    """Raised when a Linear GraphQL operation fails."""


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate, preview, or publish a docs/tasks task package."
    )
    parser.add_argument("--manifest", default=None, help="Path to task-package.yaml.")
    parser.add_argument("--repo-root", default=None, help="Repository root.")
    subcommands = parser.add_subparsers(dest="command", required=True)

    validate_parser = subcommands.add_parser("validate", help="Validate the task package locally.")
    add_common_args(validate_parser)
    dry_run_parser = subcommands.add_parser("dry-run", help="Print conversion waves without Linear writes.")
    add_common_args(dry_run_parser)

    apply_parser = subcommands.add_parser("apply", help="Publish the task package to Linear.")
    add_common_args(apply_parser)
    apply_parser.add_argument("--project-slug", required=True, help="Linear project slugId.")
    apply_parser.add_argument("--team-key", help="Linear team key when a project has multiple teams.")
    apply_parser.add_argument(
        "--publish",
        default=None,
        help="Publish mapping path. Defaults to <tasksDir>/linear-publish.yaml.",
    )
    apply_parser.add_argument(
        "--no-project-overview",
        action="store_true",
        help="Skip updating the Linear project overview.",
    )

    args = parser.parse_args()
    repo_root = Path(args.repo_root or ".").resolve()
    manifest_path = resolve_path(repo_root, args.manifest or "docs/tasks/task-package.yaml")
    project_set_path = (
        Path(args.project_set).resolve() if getattr(args, "project_set", None) else None
    )

    try:
        package = load_package(repo_root, manifest_path, project_set_path=project_set_path)
        if args.command == "validate":
            print_validation_summary(package)
            return 0
        if args.command == "dry-run":
            print_dry_run(package)
            return 0
        if args.command == "apply":
            publish_path = resolve_publish_path(repo_root, package, args.publish)
            desired_repo_by_task = build_desired_repo_by_task(package.tasks)
            apply_to_linear(
                package=package,
                project_slug=args.project_slug,
                team_key=args.team_key,
                publish_path=publish_path,
                update_project_overview=not args.no_project_overview,
                desired_repo_by_task=desired_repo_by_task,
            )
            return 0
    except (ValidationError, LinearError) as error:
        print(str(error), file=sys.stderr)
        return 1

    raise AssertionError(f"unhandled command {args.command}")


def add_common_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--manifest", default=None, help="Path to task-package.yaml.")
    parser.add_argument("--repo-root", default=None, help="Repository root.")
    parser.add_argument(
        "--project-set",
        default=None,
        help=(
            "Path to project-set.yaml (defaults to "
            "<repo-root>/.opensymphony/project-set.yaml). When set, the converter "
            "validates every leaf task's `repo:` slug against this inventory "
            "before any Linear write (LOC-25)."
        ),
    )


def resolve_path(repo_root: Path, value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else repo_root / path


def resolve_publish_path(repo_root: Path, package: Package, value: str | None) -> Path:
    if value:
        return resolve_path(repo_root, value)
    return repo_root / package.tasks_dir / "linear-publish.yaml"


def load_package(
    repo_root: Path,
    manifest_path: Path,
    project_set_path: Path | None = None,
) -> Package:
    errors: list[str] = []
    manifest = load_yaml_file(manifest_path, errors, "manifest")
    if not isinstance(manifest, dict):
        raise ValidationError("task package manifest must be a YAML mapping")

    planning_wave = require_non_empty_string(manifest, "planningWave", errors)
    tasks_dir = require_non_empty_string(manifest, "tasksDir", errors)
    milestones = normalize_milestones(manifest.get("milestones"), errors)
    manifest_tasks = normalize_manifest_tasks(manifest.get("tasks"), errors)

    tasks: dict[str, Task] = {}
    for manifest_task in manifest_tasks:
        path = resolve_path(repo_root, manifest_task.file)
        task = load_task(path, manifest_task, milestones, errors)
        if task and task.id in tasks:
            errors.append(f"duplicate task id {task.id}")
        elif task:
            tasks[task.id] = task

    validate_manifest_references(manifest_tasks, tasks, milestones, errors)
    validate_task_graph(tasks, errors)

    repo_inventory, inventory_source = load_project_set_inventory(
        repo_root, project_set_path, errors
    )

    if not errors:
        validate_repo_routing(tasks, repo_inventory, errors)

    if errors:
        raise ValidationError(render_errors("Task package validation failed", errors))

    waves = dependency_waves(tasks)
    return Package(
        manifest_path=manifest_path,
        repo_root=repo_root,
        planning_wave=planning_wave,
        tasks_dir=tasks_dir,
        milestones=milestones,
        manifest_tasks=manifest_tasks,
        tasks=tasks,
        waves=waves,
        repo_inventory=repo_inventory,
        repo_inventory_source=inventory_source,
    )


def load_yaml_file(path: Path, errors: list[str], label: str) -> Any:
    if not path.is_file():
        errors.append(f"{label} file does not exist: {path}")
        return None
    try:
        return yaml.safe_load(path.read_text(encoding="utf-8"))
    except yaml.YAMLError as error:
        errors.append(f"{label} YAML parse failed in {path}: {error}")
    except OSError as error:
        errors.append(f"failed to read {label} file {path}: {error}")
    return None


def require_non_empty_string(data: dict[str, Any], key: str, errors: list[str]) -> str:
    value = data.get(key)
    if isinstance(value, str) and value.strip():
        return value.strip()
    errors.append(f"manifest field {key} must be a non-empty string")
    return ""


def normalize_milestones(value: Any, errors: list[str]) -> list[str]:
    if not isinstance(value, list) or not value:
        errors.append("manifest field milestones must be a non-empty list")
        return []

    milestones: list[str] = []
    for index, item in enumerate(value):
        name = normalize_milestone_name(item)
        if not name:
            errors.append(f"milestones[{index}] must be a non-empty string")
            continue
        milestones.append(name)

    duplicates = sorted(name for name, count in counts(milestones).items() if count > 1)
    for duplicate in duplicates:
        errors.append(f"duplicate milestone {duplicate}")
    return milestones


def normalize_milestone_name(value: Any) -> str | None:
    if isinstance(value, str):
        return value.strip() or None
    if isinstance(value, dict) and len(value) == 1:
        key, item = next(iter(value.items()))
        if isinstance(key, str) and isinstance(item, str):
            return f"{key}: {item}".strip()
    return None


def normalize_manifest_tasks(value: Any, errors: list[str]) -> list[ManifestTask]:
    if not isinstance(value, list) or not value:
        errors.append("manifest field tasks must be a non-empty list")
        return []

    tasks: list[ManifestTask] = []
    for index, item in enumerate(value):
        if not isinstance(item, dict):
            errors.append(f"tasks[{index}] must be a mapping")
            continue
        task_id = item.get("id")
        file_path = item.get("file")
        if not isinstance(task_id, str) or not task_id.strip():
            errors.append(f"tasks[{index}].id must be a non-empty string")
            continue
        if not isinstance(file_path, str) or not file_path.strip():
            errors.append(f"tasks[{index}].file must be a non-empty string")
            continue
        tasks.append(ManifestTask(id=task_id.strip(), file=file_path.strip()))

    for duplicate in sorted(name for name, count in counts([task.id for task in tasks]).items() if count > 1):
        errors.append(f"duplicate manifest task id {duplicate}")
    for duplicate in sorted(name for name, count in counts([task.file for task in tasks]).items() if count > 1):
        errors.append(f"duplicate manifest task file {duplicate}")
    return tasks


def load_task(
    path: Path,
    manifest_task: ManifestTask,
    milestones: list[str],
    errors: list[str],
) -> Task | None:
    if not path.is_file():
        errors.append(f"task {manifest_task.id} file does not exist: {manifest_task.file}")
        return None

    text = path.read_text(encoding="utf-8")
    match = re.match(r"\A---\n(.*?)\n---\n?", text, re.DOTALL)
    if not match:
        errors.append(f"task {manifest_task.id} is missing YAML frontmatter: {manifest_task.file}")
        return None

    try:
        frontmatter = yaml.safe_load(match.group(1))
    except yaml.YAMLError as error:
        errors.append(f"task {manifest_task.id} frontmatter YAML parse failed: {error}")
        return None
    if not isinstance(frontmatter, dict):
        errors.append(f"task {manifest_task.id} frontmatter must be a YAML mapping")
        return None

    for key in REQUIRED_FRONTMATTER:
        if key not in frontmatter:
            errors.append(f"task {manifest_task.id} is missing frontmatter field {key}")

    task_id = frontmatter.get("id")
    title = frontmatter.get("title")
    milestone = frontmatter.get("milestone")
    priority = frontmatter.get("priority")
    estimate = frontmatter.get("estimate")
    blocked_by = frontmatter.get("blockedBy")
    blocks = frontmatter.get("blocks")
    areas = normalize_area_slugs(frontmatter.get("areas", []), manifest_task.id, errors)
    parent = frontmatter.get("parent")
    repo = _parse_repo_slug(frontmatter.get("repo"), manifest_task.id, errors)

    if task_id != manifest_task.id:
        errors.append(
            f"task file {manifest_task.file} has id {task_id!r}, expected {manifest_task.id!r}"
        )
    if not isinstance(title, str) or not title.strip():
        errors.append(f"task {manifest_task.id} title must be a non-empty string")
    if not isinstance(milestone, str) or milestone not in milestones:
        errors.append(f"task {manifest_task.id} milestone must match a manifest milestone")
    if not isinstance(priority, int) or priority not in PRIORITY_NAMES:
        errors.append(f"task {manifest_task.id} priority must be an integer from 0 through 4")
    if not isinstance(estimate, int) or estimate < 0:
        errors.append(f"task {manifest_task.id} estimate must be a non-negative integer")
    if not is_string_list(blocked_by):
        errors.append(f"task {manifest_task.id} blockedBy must be a list of task IDs")
        blocked_by = []
    if not is_string_list(blocks):
        errors.append(f"task {manifest_task.id} blocks must be a list of task IDs")
        blocks = []
    if parent is not None and (not isinstance(parent, str) or not parent.strip()):
        errors.append(f"task {manifest_task.id} parent must be null or a task ID")
        parent = None

    body = text[match.end() :].strip()
    validate_sections(manifest_task.id, manifest_task.file, body, errors)

    return Task(
        id=manifest_task.id,
        file=manifest_task.file,
        path=path,
        title=title.strip() if isinstance(title, str) else manifest_task.id,
        milestone=milestone if isinstance(milestone, str) else "",
        priority=priority if isinstance(priority, int) else 3,
        estimate=estimate if isinstance(estimate, int) else 0,
        blocked_by=list(blocked_by) if is_string_list(blocked_by) else [],
        blocks=list(blocks) if is_string_list(blocks) else [],
        areas=areas,
        parent=parent.strip() if isinstance(parent, str) else None,
        body=body,
        repo=repo,
    )


def validate_sections(task_id: str, file_path: str, body: str, errors: list[str]) -> None:
    headings = set(re.findall(r"^##\s+(.+?)\s*$", body, re.MULTILINE))
    for section in REQUIRED_SECTIONS:
        if section not in headings:
            errors.append(f"task {task_id} is missing section ## {section} in {file_path}")


def validate_manifest_references(
    manifest_tasks: list[ManifestTask],
    tasks: dict[str, Task],
    milestones: list[str],
    errors: list[str],
) -> None:
    manifest_ids = {task.id for task in manifest_tasks}
    loaded_ids = set(tasks)
    for task_id in sorted(manifest_ids - loaded_ids):
        errors.append(f"manifest task {task_id} could not be loaded")

    for task in tasks.values():
        if task.milestone not in milestones:
            errors.append(f"task {task.id} milestone is not declared in the manifest")
        for dependency in task.blocked_by:
            if dependency not in manifest_ids:
                errors.append(f"task {task.id} blockedBy references unknown task {dependency}")
            if dependency == task.id:
                errors.append(f"task {task.id} cannot be blocked by itself")
        for blocked in task.blocks:
            if blocked not in manifest_ids:
                errors.append(f"task {task.id} blocks references unknown task {blocked}")
            if blocked == task.id:
                errors.append(f"task {task.id} cannot block itself")
        if task.parent:
            if task.parent not in manifest_ids:
                errors.append(f"task {task.id} parent references unknown task {task.parent}")
            if task.parent == task.id:
                errors.append(f"task {task.id} cannot be its own parent")
            if task.parent in task.blocked_by or task.parent in task.blocks:
                errors.append(f"task {task.id} must not add blocker metadata to its parent")


def validate_task_graph(tasks: dict[str, Task], errors: list[str]) -> None:
    parent_graph: dict[str, list[str]] = defaultdict(list)
    for task in tasks.values():
        if task.parent:
            parent_graph[task.parent].append(task.id)
    if has_cycle({task_id: children for task_id, children in parent_graph.items()}):
        errors.append("parent relationships contain a cycle")

    dependency_graph = {task.id: list(task.blocked_by) for task in tasks.values()}
    cycle = dependency_cycle(dependency_graph)
    if cycle:
        errors.append(f"blockedBy dependencies contain a cycle: {' -> '.join(cycle)}")

    creation_graph = {
        task.id: list(task.blocked_by) + ([task.parent] if task.parent else [])
        for task in tasks.values()
    }
    cycle = dependency_cycle(creation_graph)
    if cycle:
        errors.append(f"creation dependencies contain a cycle: {' -> '.join(cycle)}")


def dependency_waves(tasks: dict[str, Task]) -> list[list[str]]:
    remaining = set(tasks)
    created: set[str] = set()
    waves: list[list[str]] = []

    while remaining:
        wave = sorted(
            task_id
            for task_id in remaining
            if all(dep in created for dep in tasks[task_id].blocked_by)
            and (tasks[task_id].parent is None or tasks[task_id].parent in created)
        )
        if not wave:
            raise ValidationError("unable to compute dependency waves")
        waves.append(wave)
        created.update(wave)
        remaining.difference_update(wave)
    return waves


def is_string_list(value: Any) -> bool:
    return isinstance(value, list) and all(isinstance(item, str) and item.strip() for item in value)


def normalize_area_slugs(value: Any, task_id: str, errors: list[str]) -> list[str]:
    if value is None:
        return []
    if not is_string_list(value):
        errors.append(f"task {task_id} areas must be a list of strings")
        return []
    areas: list[str] = []
    for raw in value:
        stripped = raw.strip()
        # LOC-25 + LOC-22 + LOC-24: ``areas`` is reserved for the
        # ``area:`` namespace only. ``repo:`` (and any other reserved
        # non-area namespace) belongs to its own dedicated frontmatter
        # field (``repo``) and must never appear here, otherwise the
        # planning task would silently publish the wrong label.
        if stripped.lower().startswith("repo:"):
            errors.append(
                f"task {task_id} areas entry {stripped!r} uses the reserved "
                "non-area namespace 'repo:'; use the `repo:` frontmatter field instead"
            )
            continue
        area = area_slug(raw)
        if not area:
            errors.append(f"task {task_id} has an empty area value")
            continue
        areas.append(area)
    return sorted(set(areas))


def _parse_repo_slug(value: Any, task_id: str, errors: list[str]) -> str | None:
    """Parse the ``repo:`` frontmatter value into a normalised slug.

    Returns:
      * ``None`` when the frontmatter omits the field (or the value is
        YAML ``null``). This is the parent/review shape: the validator
        accepts a missing repo on parent tasks.
      * ``""`` when the frontmatter explicitly wrote ``repo:`` with no
        value (or only whitespace). The validator treats an empty
        repo on a leaf as a missing-leaf-repo error so the planner
        fixes the frontmatter before publish.
      * The trimmed slug string otherwise. The slug is preserved
        verbatim — the validator compares it character-for-character
        against the project-set inventory, so no lowercasing or
        slugification happens here.

    Non-string values are rejected with an error so a planning wave
    that accidentally writes ``repo: [foo, bar]`` cannot slip through.
    """
    if value is None:
        return None
    if not isinstance(value, str):
        errors.append(
            f"task {task_id} repo must be a string or null (got {type(value).__name__})"
        )
        return None
    return value.strip()


# LOC-25: default location for the global project-set inventory. Matches
# the location documented in `docs/configuration.md` (`## Global Project Set`)
# so the converter can validate ``repo:`` slugs without any extra wiring on
# the developer side. Tests can override it via ``--project-set``.
DEFAULT_PROJECT_SET_PATH = Path(".opensymphony/project-set.yaml")


def load_project_set_inventory(
    repo_root: Path,
    override_path: Path | None,
    errors: list[str],
) -> tuple[set[str] | None, Path | None]:
    """Load the project-set inventory of allowed repo slugs.

    Returns a tuple of ``(inventory, source_path)``:

    * ``inventory`` is the set of repo slugs declared under
      ``project_set.projects[].repos[].slug``. ``None`` means the
      inventory could not be loaded at all (the file was missing or
      unreadable); in that case the caller surfaces a separate
      validation error so operators know to onboard the project set.
    * ``source_path`` is the absolute path the loader actually read so
      dry-run output can prove which inventory gated the wave (and so
      tests can prove the explicit override path was used instead of
      the default).

    Fail-fast semantics:

    * The **default** inventory path (``<repo_root>/.opensymphony/project-set.yaml``)
      is mandatory: a missing file appends an error so a planning wave
      cannot silently skip the out-of-inventory check.
    * An **explicit** ``--project-set`` override may legitimately point
      at a missing file (the operator intentionally disabled the
      inventory for the test). The override is allowed to be missing,
      but a missing override still surfaces a warning so the operator
      knows the inventory gate was bypassed.
    """

    is_override = override_path is not None
    source = (
        override_path.resolve()
        if is_override
        else (repo_root / DEFAULT_PROJECT_SET_PATH).resolve()
    )
    if not source.is_file():
        if is_override:
            errors.append(
                f"project-set override {source} does not exist; the explicit "
                "override path is required to point at a real file so "
                "out-of-inventory checks remain effective"
            )
        else:
            errors.append(
                f"project-set file {source} is missing; the default inventory "
                "is required for LOC-25 repo validation — onboard the project set "
                "or pass --project-set to override the path"
            )
        return None, source
    try:
        data = yaml.safe_load(source.read_text(encoding="utf-8"))
    except (yaml.YAMLError, OSError) as error:
        errors.append(f"project-set file {source} is unreadable: {error}")
        return None, source
    if not isinstance(data, dict):
        errors.append(
            f"project-set file {source} must be a YAML mapping; got {type(data).__name__}"
        )
        return None, source

    slugs: set[str] = set()
    try:
        project_set = data.get("project_set") or {}
        projects = project_set.get("projects") or []
    except AttributeError:
        projects = []
    if not isinstance(projects, list):
        errors.append(
            f"project-set file {source} project_set.projects must be a list"
        )
        return None, source
    for project in projects:
        if not isinstance(project, dict):
            continue
        repos = project.get("repos") or []
        if not isinstance(repos, list):
            errors.append(
                f"project-set file {source} project {project.get('slug')!r} repos "
                "must be a list"
            )
            return None, source
        for repo in repos:
            if not isinstance(repo, dict):
                continue
            slug = repo.get("slug")
            if not isinstance(slug, str) or not slug.strip():
                errors.append(
                    f"project-set file {source} has a repo entry without a slug"
                )
                continue
            slugs.add(slug.strip())
    if not slugs:
        errors.append(
            f"project-set file {source} declared zero repos; planning repo routing "
            "cannot be validated"
        )
        return None, source
    return slugs, source


def validate_repo_routing(
    tasks: dict[str, Task],
    inventory: set[str] | None,
    errors: list[str],
) -> None:
    """Enforce the LOC-25 leaf-vs-parent repo routing contract.

    A task whose id appears in any other task's ``parent`` field is a
    parent/review node and MUST NOT carry a ``repo:`` value. Every other
    task is a leaf and MUST carry exactly one non-empty ``repo:`` slug
    that exists in the project-set inventory. Slugs are compared
    character-for-character; no lowercasing or slugification happens
    here, so an entry like ``OpenSymphony-Config`` is preserved verbatim.

    The validator distinguishes three error classes so the planning
    operator can fix the right field:

    * ``parent-with-repo`` — a parent/review task carries a slug.
    * ``missing-leaf-repo`` — a leaf task did not declare a slug.
    * ``out-of-inventory-repo`` — a leaf task declared a slug that the
      project set does not know about.
    """

    parent_ids: set[str] = {
        task.parent
        for task in tasks.values()
        if isinstance(task.parent, str) and task.parent.strip()
    }
    for task in sorted(tasks.values(), key=lambda t: t.id):
        is_parent = task.id in parent_ids
        declared = task.repo
        if is_parent:
            # LOC-25 review feedback: align Python and Rust on parent
            # ``repo`` semantics. The Rust manifest validator (see
            # ``graph_validate/manifest.rs``) treats any non-``None``
            # declared repo on a parent as an error, so the Python
            # converter must do the same — ``repo: ""`` (or any
            # whitespace-only value) on a parent is rejected here too.
            if declared is not None:
                errors.append(
                    f"task {task.id} is a parent/review task and must not carry "
                    f"`repo: {declared!r}`; the routing lives on the leaves"
                )
            continue
        # Leaf shape: a non-empty slug is required.
        if declared is None or not declared.strip():
            errors.append(
                f"task {task.id} is a leaf and must declare exactly one non-empty "
                "`repo: <slug>` frontmatter field that matches a project-set "
                "inventory key"
            )
            continue
        if inventory is not None and declared not in inventory:
            known = ", ".join(sorted(inventory)) if inventory else "<empty>"
            errors.append(
                f"task {task.id} declares `repo: {declared!r}` but the project-set "
                f"inventory does not contain that slug; known slugs: {known}"
            )


def build_desired_repo_by_task(
    tasks: dict[str, Task],
) -> dict[str, DesiredRepo]:
    """Project per-task ``DesiredRepo`` state from parsed ``Task.repo``.

    Parents (and any other task with no repo) get ``DesiredRepo.cleared()``
    so the additive merge strips any stale ``repo:*`` label rather than
    preserving it. Leaves with a non-empty slug get
    ``DesiredRepo.managed(slug)`` so the merge ensures exactly one
    ``repo:<slug>`` label survives.

    The map is keyed by task id and is consumed by ``apply_to_linear``
    via the ``desired_repo_by_task`` parameter.
    """

    parent_ids: set[str] = {
        task.parent
        for task in tasks.values()
        if isinstance(task.parent, str) and task.parent.strip()
    }
    desired: dict[str, DesiredRepo] = {}
    for task in tasks.values():
        if task.id in parent_ids:
            desired[task.id] = DesiredRepo.cleared()
            continue
        slug = (task.repo or "").strip()
        if slug:
            desired[task.id] = DesiredRepo.managed(slug)
        else:
            # Leaf without a slug should have been caught by
            # ``validate_repo_routing`` already; the publish path
            # treats this as a defensive preserve so a stale
            # ``repo:*`` label is not wiped when the validator was
            # bypassed.
            desired[task.id] = DesiredRepo.preserved()
    return desired


def area_slug(value: str) -> str:
    normalized = value.strip()
    if normalized.lower().startswith("area:"):
        normalized = normalized.split(":", 1)[1]
    normalized = re.sub(r"[^a-z0-9]+", "-", normalized.lower()).strip("-")
    return normalized


def area_label_name(area: str) -> str:
    return f"area:{area}"


def counts(values: list[str]) -> dict[str, int]:
    result: dict[str, int] = defaultdict(int)
    for value in values:
        result[value] += 1
    return result


def has_cycle(graph: dict[str, list[str]]) -> bool:
    return bool(dependency_cycle(graph))


def dependency_cycle(graph: dict[str, list[str]]) -> list[str]:
    visiting: set[str] = set()
    visited: set[str] = set()
    path: list[str] = []

    def visit(node: str) -> list[str]:
        if node in visiting:
            index = path.index(node)
            return path[index:] + [node]
        if node in visited:
            return []
        visiting.add(node)
        path.append(node)
        for neighbor in graph.get(node, []):
            cycle = visit(neighbor)
            if cycle:
                return cycle
        visiting.remove(node)
        visited.add(node)
        path.pop()
        return []

    for node in sorted(graph):
        cycle = visit(node)
        if cycle:
            return cycle
    return []


def render_errors(title: str, errors: list[str]) -> str:
    lines = [title + ":"]
    lines.extend(f"- {error}" for error in errors)
    return "\n".join(lines)


def print_validation_summary(package: Package) -> None:
    print(f"planningWave: {package.planning_wave}")
    print(f"milestones: {len(package.milestones)}")
    print(f"tasks: {len(package.tasks)}")
    print(f"waves: {len(package.waves)}")
    print_repo_routing_summary(package)
    print("validation: ok")


def print_repo_routing_summary(package: Package) -> None:
    """Surface the project-set inventory used to gate repo routing.

    The summary deliberately shows the absolute source path so operators
    (and tests) can confirm which inventory file was loaded — the
    explicit ``--project-set`` override path is the canonical answer, the
    default location only applies when no override was passed.
    """

    source = package.repo_inventory_source
    inventory = package.repo_inventory
    if source is None and inventory is None:
        print("repoInventory: <not loaded>")
        return
    if source is not None:
        print(f"repoInventory: {source}")
    if inventory is not None:
        print(f"repoInventorySlugs: {len(inventory)}")
    leaves = sum(1 for task in package.tasks.values() if not _is_parent(task, package.tasks))
    print(f"repoRouting: leaves={leaves} parents={len(package.tasks) - leaves}")


def _is_parent(task: Task, tasks: dict[str, Task]) -> bool:
    return any(
        isinstance(other.parent, str) and other.parent.strip() == task.id
        for other in tasks.values()
    )


def print_dry_run(package: Package) -> None:
    print_validation_summary(package)
    print()
    print("Milestones:")
    for milestone in package.milestones:
        count = sum(1 for task in package.tasks.values() if task.milestone == milestone)
        print(f"- {milestone} ({count} task(s))")
    print()
    print("Repo routing (LOC-25):")
    parent_ids = {
        task.parent
        for task in package.tasks.values()
        if isinstance(task.parent, str) and task.parent.strip()
    }
    for task in sorted(package.tasks.values(), key=lambda t: t.id):
        kind = "parent" if task.id in parent_ids else "leaf"
        slug = task.repo if task.repo else "-"
        print(f"- {kind:6s} {task.id} repo={slug}")
    print()
    print("Repo labels to publish (managed):")
    managed_slugs = sorted({
        task.repo
        for task in package.tasks.values()
        if task.repo and task.id not in parent_ids
    })
    for slug in managed_slugs:
        print(f"- repo:{slug}")
    print()
    print("Creation waves:")
    for index, wave in enumerate(package.waves, start=1):
        print(f"- Wave {index}: {', '.join(wave)}")


class LinearClient:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root
        self.helper = repo_root / ".agents/skills/linear/scripts/linear_graphql.py"
        self.queries = repo_root / ".agents/skills/linear/queries"
        if not self.helper.is_file():
            raise LinearError(f"Linear helper not found: {self.helper}")

    def call(self, query_name: str, variables: dict[str, Any], allow_errors: bool = False) -> dict[str, Any]:
        query_file = self.queries / query_name
        if not query_file.is_file():
            raise LinearError(f"Linear query file not found: {query_file}")
        with tempfile.NamedTemporaryFile("w", suffix=".json", encoding="utf-8", delete=False) as temp:
            json.dump(variables, temp)
            temp_path = temp.name
        try:
            result = subprocess.run(
                [
                    "python3",
                    str(self.helper),
                    "--query-file",
                    str(query_file),
                    "--variables-file",
                    temp_path,
                ],
                cwd=self.repo_root,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
        finally:
            Path(temp_path).unlink(missing_ok=True)

        data = json.loads(result.stdout) if result.stdout.strip().startswith("{") else None
        if result.returncode != 0 and not allow_errors:
            detail = result.stdout.strip() or result.stderr.strip()
            raise LinearError(f"Linear GraphQL call failed for {query_name}: {detail}")
        if data is None:
            raise LinearError(f"Linear GraphQL call returned non-JSON output for {query_name}")
        if data.get("errors") and not allow_errors:
            raise LinearError(f"Linear GraphQL errors for {query_name}: {json.dumps(data['errors'], indent=2)}")
        return data


def apply_to_linear(
    package: Package,
    project_slug: str,
    team_key: str | None,
    publish_path: Path,
    update_project_overview: bool,
    desired_repo_by_task: dict[str, DesiredRepo] | None = None,
) -> None:
    client = LinearClient(package.repo_root)
    state = load_project_state(client, project_slug)
    project = state["project"]
    team = select_team(project, team_key)
    publish = load_publish_file(publish_path)

    milestone_map = ensure_milestones(client, package, project)
    issue_map = ensure_issues(
        client,
        package,
        project,
        team,
        milestone_map,
        publish,
        desired_repo_by_task=desired_repo_by_task,
    )
    apply_blockers(client, package, issue_map)
    rewrite_issue_bodies(client, package, milestone_map, issue_map, project_slug)

    if update_project_overview:
        update_overview(client, package, issue_map, project)

    write_publish_file(publish_path, package, project_slug, milestone_map, issue_map)
    print(f"published tasks: {len(issue_map)}")
    print(f"publish mapping: {publish_path}")


def load_project_state(client: LinearClient, project_slug: str) -> dict[str, Any]:
    data = client.call("project_planning_state.graphql", {"slug": project_slug})
    nodes = data.get("data", {}).get("projects", {}).get("nodes", [])
    if not nodes:
        raise LinearError(f"Linear project not found for slug {project_slug}")
    project = nodes[0]
    _assert_project_state_complete(project, project_slug)
    return {"project": project}


def _assert_project_state_complete(project: dict[str, Any], project_slug: str) -> None:
    """Fail fast when the project issue or label pages are truncated."""

    issues = project.get("issues") or {}
    if (issues.get("pageInfo") or {}).get("hasNextPage"):
        raise LinearError(
            f"project {project_slug!r} issues page was truncated by pagination; "
            "cannot safely merge labels against a partial issue set"
        )
    for issue in issues.get("nodes") or []:
        if not isinstance(issue, dict):
            continue
        labels = issue.get("labels") or {}
        if (labels.get("pageInfo") or {}).get("hasNextPage"):
            identifier = issue.get("identifier") or issue.get("id") or "<unknown>"
            raise LinearError(
                f"issue {identifier!r} labels page was truncated by pagination; "
                "cannot safely merge against a partial label set"
            )


def select_team(project: dict[str, Any], team_key: str | None) -> dict[str, Any]:
    teams = project.get("teams", {}).get("nodes", [])
    if team_key:
        for team in teams:
            if team.get("key") == team_key:
                return team
        raise LinearError(f"project has no team with key {team_key}")
    if len(teams) == 1:
        return teams[0]
    keys = ", ".join(team.get("key", "") for team in teams)
    raise LinearError(f"project has multiple teams; pass --team-key. Available: {keys}")


def load_publish_file(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {}
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    return data if isinstance(data, dict) else {}


def ensure_milestones(
    client: LinearClient,
    package: Package,
    project: dict[str, Any],
) -> dict[str, dict[str, Any]]:
    existing = {
        milestone["name"]: milestone
        for milestone in project.get("projectMilestones", {}).get("nodes", [])
    }
    milestone_map: dict[str, dict[str, Any]] = {}
    for name in package.milestones:
        if name in existing:
            milestone_map[name] = existing[name]
            continue
        data = client.call(
            "project_milestone_create.graphql",
            {"input": {"projectId": project["id"], "name": name}},
        )
        milestone = data["data"]["projectMilestoneCreate"]["projectMilestone"]
        milestone_map[name] = milestone
        print(f"created milestone: {name}")
    return milestone_map


def ensure_issues(
    client: LinearClient,
    package: Package,
    project: dict[str, Any],
    team: dict[str, Any],
    milestone_map: dict[str, dict[str, Any]],
    publish: dict[str, Any],
    desired_repo_by_task: dict[str, DesiredRepo] | None = None,
) -> dict[str, dict[str, Any]]:
    existing_by_provenance = issues_by_provenance(project, package.planning_wave)
    publish_tasks = publish.get("tasks", {}) if isinstance(publish.get("tasks"), dict) else {}
    issue_map: dict[str, dict[str, Any]] = {}
    area_label_ids = ensure_area_labels(client, package, team)
    # LOC-25: lazily create any ``repo:<slug>`` labels referenced by leaf
    # tasks so the merge below can resolve their ids when ``_lookup_repo_label_id``
    # misses on the existing issue. The set is the union of every
    # managed-slug declared by leaf tasks in the package, no other path
    # touches this list.
    repo_label_ids = ensure_repo_labels(
        client,
        package,
        team,
        desired_repo_by_task or {},
    )
    desired_repo_by_task = desired_repo_by_task or {}

    for wave in package.waves:
        for task_id in wave:
            task = package.tasks[task_id]
            mapped = publish_tasks.get(task_id, {}) if isinstance(publish_tasks.get(task_id), dict) else {}
            existing = None
            needs_full_hydration = False
            if mapped.get("issueId"):
                # linear-publish.yaml only stores id/identifier/url - we need
                # to fetch the full issue with labels before updating.
                existing = {
                    "id": mapped["issueId"],
                    "identifier": mapped.get("issue"),
                    "url": mapped.get("url"),
                }
                needs_full_hydration = True
            elif task_id in existing_by_provenance:
                existing = existing_by_provenance[task_id]

            if existing and needs_full_hydration:
                existing = fetch_issue_with_labels(client, existing["id"], existing)

            body = issue_body(package, task, issue_map=None)
            input_data: dict[str, Any] = {
                "teamId": team["id"],
                "projectId": project["id"],
                "projectMilestoneId": milestone_map[task.milestone]["id"],
                "title": task.title,
                "description": body,
                "priority": task.priority,
                "estimate": task.estimate,
            }
            if task.parent:
                input_data["parentId"] = issue_map[task.parent]["id"]
            label_ids = merged_label_ids(
                existing=existing,
                task=task,
                area_label_ids=area_label_ids,
                desired_repo=desired_repo_by_task.get(task_id),
                repo_label_ids=repo_label_ids,
            )
            if label_ids is not None:
                input_data["labelIds"] = label_ids

            if existing:
                issue = update_issue(client, existing["id"], input_data)
                print(f"updated issue: {issue['identifier']} {task.title}")
            else:
                issue = create_issue(client, input_data, task.title)
                print(f"created issue: {issue['identifier']} {task.title}")
            issue_map[task_id] = issue
    return issue_map


def ensure_area_labels(client: LinearClient, package: Package, team: dict[str, Any]) -> dict[str, str]:
    areas = sorted(
        {
            area
            for task in package.tasks.values()
            for area in (task.areas or [])
        }
    )
    if not areas:
        return {}
    label_ids: dict[str, str] = {}
    for area in areas:
        name = area_label_name(area)
        existing = find_issue_label(client, name, team["id"])
        if existing:
            label_ids[area] = existing["id"]
            continue
        data = client.call(
            "issue_label_create.graphql",
            {"input": {"name": name, "teamId": team["id"]}},
            allow_errors=True,
        )
        if data.get("errors"):
            existing = find_issue_label(client, name, team["id"])
            if existing:
                label_ids[area] = existing["id"]
                continue
            raise LinearError(f"failed to create label {name}: {json.dumps(data['errors'], indent=2)}")
        label = data["data"]["issueLabelCreate"]["issueLabel"]
        label_ids[area] = label["id"]
        print(f"created area label: {name}")
    return label_ids


def repo_label_name(slug: str) -> str:
    """Render the canonical ``repo:<slug>`` label name.

    Slugs are preserved verbatim — no lowercasing or slugification — so
    the on-disk inventory matches the on-Linear label exactly.
    """

    return f"{REPO_PREFIX}{slug}"


def ensure_repo_labels(
    client: LinearClient,
    package: Package,
    team: dict[str, Any],
    desired_repo_by_task: dict[str, DesiredRepo],
) -> dict[str, str]:
    """Lazily ensure every managed ``repo:<slug>`` label exists.

    LOC-25 keeps the project set's repo slugs as the source of truth for
    the linear label set. The wave is only allowed to publish managed
    slugs — every other ``DesiredRepo.kind`` (``cleared`` / ``preserved``)
    is intentionally ignored here because clearing or preserving does
    not introduce a new label, only removes or keeps an existing one.

    Returns a slug -> label_id cache that ``merged_label_ids`` uses as a
    fallback when ``_lookup_repo_label_id`` cannot find an existing
    match on the issue.

    Lookup is exact-case first, then a defensive case-insensitive
    fallback. The exact-case primary path keeps the contract: a new
    ``repo:<slug>`` label is created with the inventory key verbatim, no
    lowercasing or slugification. The case-insensitive fallback only
    triggers when a legacy tool or manual intervention created a
    case-variant label on the team — in that case reusing the existing
    label prevents a duplicate ``repo:opensymphony`` (new) /
    ``repo:OpenSymphony`` (legacy) split that would later confuse
    ``_lookup_repo_label_id``.
    """

    slugs = sorted({
        desired.slug
        for desired in desired_repo_by_task.values()
        if desired.kind == "managed" and desired.slug
    })
    label_ids: dict[str, str] = {}
    for slug in slugs:
        name = repo_label_name(slug)
        existing = find_issue_label(client, name, team["id"])
        if existing:
            label_ids[slug] = existing["id"]
            continue
        legacy = _find_existing_repo_label_case_insensitive(client, name, team["id"])
        if legacy:
            label_ids[slug] = legacy["id"]
            print(
                f"reusing existing repo label {legacy['name']!r} for slug "
                f"{slug!r} (case-insensitive match; inventory slug preserved "
                f"on subsequent label emissions)"
            )
            continue
        data = client.call(
            "issue_label_create.graphql",
            {"input": {"name": name, "teamId": team["id"]}},
            allow_errors=True,
        )
        if data.get("errors"):
            existing = find_issue_label(client, name, team["id"])
            if existing:
                label_ids[slug] = existing["id"]
                continue
            legacy = _find_existing_repo_label_case_insensitive(client, name, team["id"])
            if legacy:
                label_ids[slug] = legacy["id"]
                continue
            raise LinearError(
                f"failed to create repo label {name}: "
                f"{json.dumps(data['errors'], indent=2)}"
            )
        label = data["data"]["issueLabelCreate"]["issueLabel"]
        label_ids[slug] = label["id"]
        print(f"created repo label: {name}")
    return label_ids


def _find_existing_repo_label_case_insensitive(
    client: LinearClient,
    name: str,
    team_id: str,
) -> dict[str, Any] | None:
    """Case-insensitive fallback lookup for an existing ``repo:<slug>`` label.

    ``find_issue_label`` uses Linear's ``name: { eq: $name }`` filter,
    which is case-sensitive. If a legacy tool or manual operator
    created a case-variant of the same label (e.g. ``repo:OpenSymphony``
    for an inventory slug ``opensymphony``), the exact-case lookup
    misses it and ``issueLabelCreate`` would otherwise produce a
    duplicate. This helper uses ``eqIgnoreCase`` so we can reuse the
    legacy label id without altering the on-disk inventory contract.
    """

    data = client.call(
        "issue_label_by_name_case_insensitive.graphql",
        {"name": name, "teamId": team_id, "first": 10},
        allow_errors=True,
    )
    if data.get("errors"):
        return None
    nodes = data.get("data", {}).get("issueLabels", {}).get("nodes", [])
    name_lower = name.lower()
    for label in nodes:
        existing_name = label.get("name")
        if (
            isinstance(existing_name, str)
            and existing_name.lower() == name_lower
            and existing_name != name
        ):
            return label
    return None


def find_issue_label(client: LinearClient, name: str, team_id: str) -> dict[str, Any] | None:
    """Look up an existing issue label by *exact* name on the given team.

    This is intentionally case-sensitive: it is used to dedupe *area*
    labels (e.g. ``area:planning``), where exact case is the contract —
    ``area:Planning`` and ``area:planning`` are different labels by spec
    (LOC-24). Repo labels, by contrast, may have legacy case-variants
    on Linear (``repo:OpenSymphony`` vs the inventory key
    ``repo:opensymphony``); for those, use
    ``_find_existing_repo_label_case_insensitive`` so a legacy label is
    reused instead of being shadowed by a fresh exact-case emission.
    """
    data = client.call(
        "issue_label_by_name.graphql",
        {"name": name, "teamId": team_id, "first": 10},
        allow_errors=True,
    )
    if data.get("errors"):
        return None
    nodes = data.get("data", {}).get("issueLabels", {}).get("nodes", [])
    for label in nodes:
        if label.get("name") == name:
            return label
    return None


def issues_by_provenance(project: dict[str, Any], planning_wave: str) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for issue in project.get("issues", {}).get("nodes", []):
        description = issue.get("description") or ""
        wave_match = re.search(r"<!--\s*task-planning-wave:\s*(.*?)\s*-->", description)
        id_match = re.search(r"<!--\s*task-source-id:\s*(.*?)\s*-->", description)
        if wave_match and id_match and wave_match.group(1) == planning_wave:
            result[id_match.group(1)] = issue
    return result


def merged_label_ids(
    *,
    existing: dict[str, Any] | None,
    task: Task,
    area_label_ids: dict[str, str],
    desired_repo: DesiredRepo | None,
    repo_label_ids: dict[str, str] | None = None,
) -> list[str] | None:
    """Compute the merged ``labelIds`` payload for a single task.

    Returns ``None`` when the task would not have any labels (so the caller
    can simply omit ``labelIds`` from the GraphQL input - matching the
    pre-LOC-22 behaviour for legacy tasks with no frontmatter ``areas``).

    ``repo_label_ids`` is the lazy label-id cache ``ensure_repo_labels``
    built before the wave started; it carries the ids for every managed
    ``repo:<slug>`` label the wave wants, regardless of whether the issue
    already carried the label or not. The merge keeps the existing-id
    path as a fast-path so legacy tasks that already carry the label do
    not pay a redundant lookup, then falls back to the cache for the
    newly-created label id.
    """

    existing_label_entries = _collect_existing_labels(existing)
    existing_ids_by_name = {
        label["name"]: label["id"] for label in existing_label_entries
    }

    desired_areas: list[str] | None = list(task.areas) if task.areas is not None else None
    # When ``desired_areas`` is an empty list, ``merge_label_ids`` will clear
    # all area labels; when it is ``None``, existing area labels are kept.

    area_ids_by_slug: dict[str, str] = {}
    for slug in task.areas or []:
        label_id = area_label_ids.get(slug)
        if label_id:
            area_ids_by_slug[slug] = label_id

    repo_id_by_slug: dict[str, str] = {}
    if desired_repo is not None and desired_repo.kind == "managed" and desired_repo.slug:
        # LOC-25: prefer the existing-issue id, then fall back to the
        # lazy cache. The cache wins for new leaves; the existing id wins
        # when re-publishing an issue that already carries the slug (so
        # Linear keeps the same label row instead of creating duplicates).
        label_id = _lookup_repo_label_id(slug=desired_repo.slug, existing=existing)
        if label_id is None and repo_label_ids:
            label_id = repo_label_ids.get(desired_repo.slug)
        if label_id:
            repo_id_by_slug[desired_repo.slug] = label_id

    merged = merge_label_ids(
        existing_ids_by_name,
        desired_areas=desired_areas,
        desired_repo=desired_repo,
        area_ids_by_slug=area_ids_by_slug,
        repo_id_by_slug=repo_id_by_slug,
    )
    if not merged:
        return None
    return merged


def _collect_existing_labels(issue: dict[str, Any] | None) -> list[dict[str, Any]]:
    """Extract the ``labels.nodes`` list from an issue, validating completeness.

    Returns an empty list when the issue has no labels, and raises
    :class:`LinearError` when the labels field is present but cannot be
    proven complete (paginated).
    """

    if issue is None:
        return []
    labels = issue.get("labels")
    if labels is None:
        return []
    if not isinstance(labels, dict):
        raise LinearError(
            f"issue {issue.get('identifier') or issue.get('id')!r} labels field "
            "is not a connection object"
        )
    nodes = labels.get("nodes") or []
    if not isinstance(nodes, list):
        raise LinearError(
            f"issue {issue.get('identifier') or issue.get('id')!r} labels.nodes "
            "is not a list"
        )
    page_info = labels.get("pageInfo") or {}
    if page_info.get("hasNextPage"):
        raise LinearError(
            f"issue {issue.get('identifier') or issue.get('id')!r} labels were "
            "truncated by pagination; cannot safely merge against partial set"
        )
    return [node for node in nodes if isinstance(node, dict)]


def _lookup_repo_label_id(
    *,
    slug: str,
    existing: dict[str, Any] | None,
) -> str | None:
    """Return the id of an existing ``repo:<slug>`` label, if any.

    LOC-22 does not create ``repo:*`` labels - it only reuses ones that
    already exist on the issue. LOC-25 owns creating new repo labels.
    """

    if existing is None:
        return None
    for label in _collect_existing_labels(existing):
        name = label.get("name")
        if isinstance(name, str) and name.lower() == f"{REPO_PREFIX}{slug.lower()}":
            return label.get("id")
    return None


def fetch_issue_with_labels(
    client: LinearClient,
    issue_id: str,
    base: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Fetch the full set of labels for an issue and merge them onto ``base``.

    The merge uses the per-issue ``issue_labels.graphql`` query, which is
    paginated so the caller (this function) is responsible for confirming
    completeness before returning.
    """

    base = dict(base or {})
    base.setdefault("id", issue_id)
    labels_by_name = fetch_labels_complete(client, issue_id)
    base["labels"] = {
        "nodes": [
            {"id": label_id, "name": name}
            for name, label_id in labels_by_name.items()
        ],
        "pageInfo": {"hasNextPage": False, "endCursor": None},
    }
    return base


def fetch_labels_complete(
    client: LinearClient,
    issue_id: str,
    page_size: int = 100,
) -> dict[str, str]:
    """Fetch every label on ``issue_id`` and prove completeness via pagination.

    Raises :class:`LinearError` when the labels field is truncated beyond the
    configurable page size, or when Linear reports an error mid-pagination.
    """

    labels_by_name: dict[str, str] = {}
    cursor: str | None = None
    while True:
        variables: dict[str, Any] = {"id": issue_id, "first": page_size}
        if cursor is not None:
            variables["after"] = cursor
        data = client.call("issue_labels.graphql", variables)
        if data.get("errors"):
            raise LinearError(
                f"failed to fetch labels for issue {issue_id}: "
                f"{json.dumps(data['errors'], indent=2)}"
            )
        issue_payload = data.get("data", {}).get("issue")
        if issue_payload is None:
            raise LinearError(f"issue {issue_id} not found while fetching labels")
        connection = issue_payload.get("labels") or {}
        nodes = connection.get("nodes") or []
        for node in nodes:
            if not isinstance(node, dict):
                continue
            name = node.get("name")
            label_id = node.get("id")
            if isinstance(name, str) and isinstance(label_id, str):
                labels_by_name[name] = label_id
        page_info = connection.get("pageInfo") or {}
        if not page_info.get("hasNextPage"):
            return labels_by_name
        cursor = page_info.get("endCursor")
        if not cursor:
            raise LinearError(
                f"issue {issue_id} labels pagination reported hasNextPage without "
                "endCursor; cannot safely fetch the rest"
            )


def create_issue(client: LinearClient, input_data: dict[str, Any], title: str) -> dict[str, Any]:
    data = client.call("issue_create.graphql", {"input": input_data}, allow_errors=True)
    if data.get("errors") and "estimate" in json.dumps(data["errors"]).lower():
        retry_input = dict(input_data)
        retry_input.pop("estimate", None)
        data = client.call("issue_create.graphql", {"input": retry_input})
    elif data.get("errors"):
        raise LinearError(f"failed to create issue {title}: {json.dumps(data['errors'], indent=2)}")
    return data["data"]["issueCreate"]["issue"]


def update_issue(client: LinearClient, issue_id: str, input_data: dict[str, Any]) -> dict[str, Any]:
    update_input = dict(input_data)
    update_input.pop("teamId", None)
    data = client.call(
        "issue_update.graphql",
        {"id": issue_id, "input": update_input},
        allow_errors=True,
    )
    if data.get("errors") and "estimate" in json.dumps(data["errors"]).lower():
        retry_input = dict(update_input)
        retry_input.pop("estimate", None)
        data = client.call("issue_update.graphql", {"id": issue_id, "input": retry_input})
    elif data.get("errors"):
        raise LinearError(f"failed to update issue {issue_id}: {json.dumps(data['errors'], indent=2)}")
    return data["data"]["issueUpdate"]["issue"]


def apply_blockers(client: LinearClient, package: Package, issue_map: dict[str, dict[str, Any]]) -> None:
    for task in package.tasks.values():
        for blocker_id in task.blocked_by:
            blocker = issue_map[blocker_id]
            blocked = issue_map[task.id]
            data = client.call(
                "issue_relation_create.graphql",
                {
                    "input": {
                        "issueId": blocker["id"],
                        "type": "blocks",
                        "relatedIssueId": blocked["id"],
                    }
                },
                allow_errors=True,
            )
            if data.get("errors"):
                message = json.dumps(data["errors"]).lower()
                if "duplicate" in message or "already" in message or "exists" in message:
                    continue
                raise LinearError(
                    f"failed to link {blocker['identifier']} blocks {blocked['identifier']}: "
                    f"{json.dumps(data['errors'], indent=2)}"
                )


def rewrite_issue_bodies(
    client: LinearClient,
    package: Package,
    milestone_map: dict[str, dict[str, Any]],
    issue_map: dict[str, dict[str, Any]],
    project_slug: str,
) -> None:
    for task in package.tasks.values():
        body = issue_body(package, task, issue_map=issue_map)
        update_issue(
            client,
            issue_map[task.id]["id"],
            {
                "projectMilestoneId": milestone_map[task.milestone]["id"],
                "description": body,
                "priority": task.priority,
                "estimate": task.estimate,
            },
        )
    print(f"rewrote issue bodies for project {project_slug}")


def issue_body(package: Package, task: Task, issue_map: dict[str, dict[str, Any]] | None) -> str:
    body = task.body
    if issue_map:
        body = replace_task_refs(body, issue_map)
        blocked_by = render_refs(task.blocked_by, issue_map)
        blocks = render_refs(task.blocks, issue_map)
    else:
        blocked_by = "Resolved during publish."
        blocks = "Resolved during publish."

    return "\n\n".join(
        [
            f"<!-- task-planning-wave: {package.planning_wave} -->",
            f"<!-- task-source-id: {task.id} -->",
            f"<!-- task-source-file: {task.file} -->",
            body,
            "## Linear Dependencies\n\n"
            f"- Blocked by: {blocked_by}\n"
            f"- Blocks: {blocks}",
            "## Linear Metadata\n\n"
            f"- Planning wave: {package.planning_wave}\n"
            f"- Milestone: {task.milestone}\n"
            f"- Areas: {', '.join(area_label_name(area) for area in task.areas) if task.areas else 'None'}\n"
            f"- Priority: {PRIORITY_NAMES.get(task.priority, task.priority)}\n"
            f"- Estimate: {task.estimate}",
            "## Definition of Done\n\n"
            "- All acceptance criteria above are satisfied.\n"
            "- Relevant tests pass, or manual verification evidence is attached.\n"
            "- A PR implementing this issue is merged to the target branch.",
        ]
    )


def replace_task_refs(body: str, issue_map: dict[str, dict[str, Any]]) -> str:
    def replace(match: re.Match[str]) -> str:
        task_id = match.group(0)
        issue = issue_map.get(task_id)
        if not issue:
            return task_id
        return f"[{issue['identifier']}]({issue['url']})"

    return re.sub(r"\b[A-Z][A-Z0-9]+-\d+\b", replace, body)


def render_refs(task_ids: list[str], issue_map: dict[str, dict[str, Any]]) -> str:
    if not task_ids:
        return "None"
    return ", ".join(f"[{issue_map[task_id]['identifier']}]({issue_map[task_id]['url']})" for task_id in task_ids)


def update_overview(client: LinearClient, package: Package, issue_map: dict[str, dict[str, Any]], project: dict[str, Any]) -> None:
    lines = [
        f"# {project['name']} Planning Wave",
        "",
        f"Planning wave: `{package.planning_wave}`",
        "",
        "## Milestones",
        "",
    ]
    for milestone in package.milestones:
        lines.append(f"### {milestone}")
        lines.append("")
        for task in package.tasks.values():
            if task.milestone == milestone:
                issue = issue_map[task.id]
                lines.append(f"- [{issue['identifier']}]({issue['url']}) - {task.title}")
        lines.append("")
    client.call("project_update_content.graphql", {"id": project["id"], "content": "\n".join(lines).strip() + "\n"})
    print("updated project overview")


def write_publish_file(
    path: Path,
    package: Package,
    project_slug: str,
    milestone_map: dict[str, dict[str, Any]],
    issue_map: dict[str, dict[str, Any]],
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "planningWave": package.planning_wave,
        "linearProject": project_slug,
        "publishedAt": datetime.now(timezone.utc).isoformat(),
        "milestones": {
            name: {
                "milestoneId": milestone["id"],
                "name": milestone["name"],
            }
            for name, milestone in milestone_map.items()
        },
        "tasks": {
            task_id: {
                "issue": issue["identifier"],
                "issueId": issue["id"],
                "url": issue["url"],
                "file": package.tasks[task_id].file,
            }
            for task_id, issue in issue_map.items()
        },
    }
    path.write_text(yaml.safe_dump(payload, sort_keys=False), encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
