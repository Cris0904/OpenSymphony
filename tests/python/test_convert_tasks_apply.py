#!/usr/bin/env python3
"""Fake-client tests for ``ensure_issues`` label merge behaviour.

These tests stub :class:`LinearClient` so they exercise the real
``ensure_issues`` / ``merged_label_ids`` code path without contacting
Linear. They cover the LOC-22 acceptance criteria around additive label
updates and pagination safety.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
sys.path.insert(0, str(REPO_ROOT / ".agents/skills/convert-tasks-to-linear/scripts"))

import convert_tasks_to_linear as ctl  # noqa: E402
from label_merge import DesiredRepo  # noqa: E402


class FakeClient:
    """Stand-in for :class:`convert_tasks_to_linear.LinearClient`.

    Records every GraphQL call so tests can assert which mutations fired and
    which payloads were sent. Pagination is simulated by replaying the
    configured page responses across ``issue_labels.graphql`` calls.

    Tests construct a *real* :class:`ctl.LinearClient` (so the ``ensure_issues``
    code path is exercised) and replace its ``call`` method with this
    instance's bound method.
    """

    def __init__(self, real_client: ctl.LinearClient) -> None:
        self.real_client = real_client
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.label_pages: dict[str, list[list[dict[str, str]]]] = {}
        self.label_names_to_ids: dict[str, str] = {}
        self.fail_on: set[str] = set()
        self.real_client.call = self.call  # type: ignore[method-assign]

    def call(
        self,
        query_name: str,
        variables: dict[str, Any],
        allow_errors: bool = False,
    ) -> dict[str, Any]:
        self.calls.append((query_name, variables))
        if query_name in self.fail_on:
            return {"errors": [{"message": f"forced failure for {query_name}"}]}

        if query_name == "issue_labels.graphql":
            return self._paginated_labels(variables)

        if query_name == "project_planning_state.graphql":
            return self._project_state()

        if query_name == "issue_label_by_name.graphql":
            name = variables["name"]
            label_id = self.label_names_to_ids.get(name)
            if label_id:
                return {
                    "data": {
                        "issueLabels": {
                            "nodes": [{"id": label_id, "name": name}]
                        }
                    }
                }
            return {"data": {"issueLabels": {"nodes": []}}}

        if query_name == "issue_label_create.graphql":
            name = variables["input"]["name"]
            label_id = f"label-{name.replace(':', '-')}"
            self.label_names_to_ids[name] = label_id
            return {
                "data": {
                    "issueLabelCreate": {
                        "issueLabel": {"id": label_id, "name": name}
                    }
                }
            }

        if query_name == "issue_create.graphql":
            title = variables["input"].get("title", "")
            issue_id = f"new-{title[:8]}"
            return {
                "data": {
                    "issueCreate": {
                        "issue": {
                            "id": issue_id,
                            "identifier": f"NEW-{title[:6].upper()}",
                            "title": title,
                            "url": f"https://linear.app/test/issue/{issue_id}",
                            "description": variables["input"].get("description", ""),
                            "state": {"id": "s", "name": "Todo"},
                            "project": {"id": "p", "name": "P", "slugId": "p"},
                            "parent": None,
                        }
                    }
                }
            }

        if query_name == "issue_update.graphql":
            issue_id = variables["id"]
            return {
                "data": {
                    "issueUpdate": {
                        "issue": {
                            "id": issue_id,
                            "identifier": f"EX-{issue_id[-6:].upper()}",
                            "title": "",
                            "description": "",
                            "url": f"https://linear.app/test/issue/{issue_id}",
                            "state": {"id": "s", "name": "Todo"},
                            "project": {"id": "p", "name": "P", "slugId": "p"},
                            "parent": None,
                        }
                    }
                }
            }

        if query_name == "project_update_content.graphql":
            return {"data": {"projectUpdate": {"project": {"id": "p"}}}}
        if query_name == "project_milestone_create.graphql":
            return {
                "data": {
                    "projectMilestoneCreate": {
                        "projectMilestone": {"id": "ms-new", "name": "NEW"}
                    }
                }
            }
        if query_name == "issue_relation_create.graphql":
            return {"data": {"issueRelationCreate": {"success": True}}}

        # All other queries succeed with minimal valid payloads.
        return {"data": {}}

    # --- helpers ---------------------------------------------------------

    def _paginated_labels(self, variables: dict[str, Any]) -> dict[str, Any]:
        issue_id = variables["id"]
        cursor = variables.get("after")
        pages = self.label_pages.get(issue_id)
        if pages is None:
            return {
                "data": {
                    "issue": {
                        "id": issue_id,
                        "labels": {
                            "nodes": [],
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                        },
                    }
                }
            }
        index = 0 if cursor is None else int(cursor)
        if index >= len(pages):
            nodes: list[dict[str, str]] = []
            has_next = False
        else:
            nodes = pages[index]
            has_next = index + 1 < len(pages)
        end_cursor = str(index + 1) if has_next else None
        return {
            "data": {
                "issue": {
                    "id": issue_id,
                    "labels": {
                        "nodes": nodes,
                        "pageInfo": {"hasNextPage": has_next, "endCursor": end_cursor},
                    },
                }
            }
        }

    def set_project_state(self, project: dict[str, Any]) -> None:
        self._project = project

    def _project_state(self) -> dict[str, Any]:
        return {"data": {"projects": {"nodes": [self._project]}}}

    def last_update(self) -> dict[str, Any] | None:
        for name, variables in reversed(self.calls):
            if name == "issue_update.graphql":
                return variables
        return None

    def last_create(self) -> dict[str, Any] | None:
        for name, variables in reversed(self.calls):
            if name == "issue_create.graphql":
                return variables
        return None


def _make_task(
    *,
    task_id: str,
    title: str,
    areas: list[str] | None,
    milestone: str = "M1: Test",
    parent: str | None = None,
) -> ctl.Task:
    return ctl.Task(
        id=task_id,
        file=f"docs/tasks/{task_id}.md",
        path=Path(f"docs/tasks/{task_id}.md"),
        title=title,
        milestone=milestone,
        priority=3,
        estimate=1,
        blocked_by=[],
        blocks=[],
        areas=areas,
        parent=parent,
        body="body",
    )


def _make_package(
    *,
    tasks: dict[str, ctl.Task],
    milestones: list[str] | None = None,
) -> ctl.Package:
    manifest_tasks = [
        ctl.ManifestTask(id=tid, file=task.file) for tid, task in tasks.items()
    ]
    waves = ctl.dependency_waves(tasks)
    return ctl.Package(
        manifest_path=Path("manifest.yaml"),
        repo_root=Path("."),
        planning_wave="test-wave",
        tasks_dir="docs/tasks",
        milestones=milestones or ["M1: Test"],
        manifest_tasks=manifest_tasks,
        tasks=tasks,
        waves=waves,
    )


def _make_team() -> dict[str, Any]:
    return {"id": "team-id", "key": "TEST", "name": "Test Team"}


def _make_project(
    *,
    issues: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    return {
        "id": "project-id",
        "name": "Test Project",
        "slugId": "test-project",
        "url": "https://linear.app/test/project",
        "content": "",
        "teams": {"nodes": [_make_team()]},
        "projectMilestones": {"nodes": [{"id": "ms-id", "name": "M1: Test"}]},
        "issues": {
            "nodes": issues or [],
            "pageInfo": {"hasNextPage": False, "endCursor": None},
        },
    }


def _make_existing_issue(
    *,
    issue_id: str,
    identifier: str,
    labels: list[dict[str, str]],
    title: str = "Existing",
    task_id: str = "T1",
) -> dict[str, Any]:
    return {
        "id": issue_id,
        "identifier": identifier,
        "title": title,
        "url": f"https://linear.app/test/issue/{identifier}",
        "description": (
            "<!-- task-planning-wave: test-wave -->\n"
            f"<!-- task-source-id: {task_id} -->\n"
            f"<!-- task-source-file: docs/tasks/{task_id}.md -->\nbody"
        ),
        "priority": 3,
        "estimate": 1,
        "parent": None,
        "projectMilestone": {"id": "ms-id", "name": "M1: Test"},
        "state": {"id": "state-id", "name": "Todo", "type": "unstarted"},
        "labels": {
            "nodes": labels,
            "pageInfo": {"hasNextPage": False, "endCursor": None},
        },
    }


class EnsureIssuesLabelTests(unittest.TestCase):
    """End-to-end label merge coverage against the real ``ensure_issues``."""

    def _make_fake_client(
        self, project: dict[str, Any] | None = None
    ) -> FakeClient:
        """Build a real ``LinearClient`` whose ``call`` is the fake.

        Pre-registers every ``area:*`` and ``repo:*`` label already on
        the project's issues so ``ensure_area_labels`` / ``find_issue_label``
        reuse the existing id instead of creating a duplicate.
        """

        real_client = ctl.LinearClient(REPO_ROOT)
        fake = FakeClient(real_client)
        if project is not None:
            fake.set_project_state(project)
            for issue in project.get("issues", {}).get("nodes", []):
                for label in (issue.get("labels") or {}).get("nodes", []):
                    name = label.get("name", "")
                    label_id = label.get("id", "")
                    if (
                        isinstance(name, str)
                        and isinstance(label_id, str)
                        and (
                            name.lower().startswith("area:")
                            or name.lower().startswith("repo:")
                        )
                    ):
                        fake.label_names_to_ids[name] = label_id
        return fake

    def _ensure_issues(
        self,
        *,
        package: ctl.Package,
        project: dict[str, Any],
        publish: dict[str, Any] | None = None,
        desired_repo_by_task: dict[str, DesiredRepo] | None = None,
    ) -> FakeClient:
        fake = self._make_fake_client(project)
        with tempfile.TemporaryDirectory() as tmp:
            package.repo_root = Path(tmp)
            ctl.ensure_issues(
                client=fake.real_client,
                package=package,
                project=project,
                team=_make_team(),
                milestone_map={"M1: Test": {"id": "ms-id", "name": "M1: Test"}},
                publish=publish or {},
                desired_repo_by_task=desired_repo_by_task,
            )
        return fake

    # ---- Provenance-discovered issue -------------------------------------

    def test_provenance_issue_preserves_handset_repo_label(self) -> None:
        """A legacy/bootstrap task with a hand-set ``repo:`` keeps it."""

        task = _make_task(task_id="T1", title="Repo-aware leaf", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        existing = _make_existing_issue(
            issue_id="issue-1",
            identifier="TEST-1",
            labels=[{"id": "label-repo", "name": "repo:opensymphony"}],
        )
        project = _make_project(issues=[existing])

        fake = self._ensure_issues(package=package, project=project)

        self.assertIsNotNone(fake.last_update())
        sent_label_ids = fake.last_update()["input"]["labelIds"]
        self.assertIn("label-repo", sent_label_ids)
        # No new ``repo:*`` label was created because LOC-22 doesn't manage it.
        self.assertEqual(
            sum(1 for lid in sent_label_ids if lid == "label-repo"),
            1,
        )

    def test_areas_managed_exactly_to_frontmatter(self) -> None:
        """A task with ``areas`` only gets those exact labels (plus preserved)."""

        task = _make_task(task_id="T1", title="Managed areas", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        existing = _make_existing_issue(
            issue_id="issue-2",
            identifier="TEST-3",
            labels=[
                {"id": "label-area-legacy", "name": "area:legacy"},
                {"id": "label-area-planning", "name": "area:planning"},
                {"id": "label-custom", "name": "custom:keep"},
            ],
        )
        project = _make_project(issues=[existing])

        fake = self._ensure_issues(package=package, project=project)

        sent_label_ids = fake.last_update()["input"]["labelIds"]
        # area:legacy must be dropped because frontmatter overrides it.
        self.assertNotIn("label-area-legacy", sent_label_ids)
        self.assertIn("label-area-planning", sent_label_ids)
        self.assertIn("label-custom", sent_label_ids)

    def test_areas_absent_preserves_existing_area_labels(self) -> None:
        """A legacy task without ``areas`` keeps existing area labels."""

        task = _make_task(task_id="T1", title="Legacy", areas=None)
        package = _make_package(tasks={"T1": task})
        existing = _make_existing_issue(
            issue_id="issue-3",
            identifier="TEST-4",
            labels=[
                {"id": "label-area", "name": "area:legacy"},
                {"id": "label-repo", "name": "repo:opensymphony"},
            ],
        )
        project = _make_project(issues=[existing])

        fake = self._ensure_issues(package=package, project=project)

        # ``areas`` is absent so existing ``area:legacy`` and ``repo:*`` are
        # preserved as-is. The merge rebuilds the full label set so the
        # ``labelIds`` payload contains exactly those preserved ids.
        sent_label_ids = fake.last_update()["input"]["labelIds"]
        self.assertIn("label-area", sent_label_ids)
        self.assertIn("label-repo", sent_label_ids)

    # ---- linear-publish.yaml mapped issues -------------------------------

    def test_publish_yaml_mapped_issue_hydrates_labels(self) -> None:
        """Mapped-from-publish issues get label hydration before update."""

        task = _make_task(task_id="T1", title="Mapped", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        project = _make_project(issues=[])
        publish = {
            "tasks": {
                "T1": {
                    "issue": "TEST-9",
                    "issueId": "mapped-issue-id",
                    "url": "https://linear.app/test/issue/TEST-9",
                }
            }
        }

        fake = self._make_fake_client(project)
        fake.label_pages["mapped-issue-id"] = [
            [
                {"id": "label-custom", "name": "custom:tag"},
                {"id": "label-area", "name": "area:planning"},
            ]
        ]
        # Pre-register the existing ``area:planning`` label so the merge
        # preserves its id (no duplicate label created).
        fake.label_names_to_ids["area:planning"] = "label-area"

        with tempfile.TemporaryDirectory() as tmp:
            package.repo_root = Path(tmp)
            ctl.ensure_issues(
                client=fake.real_client,
                package=package,
                project=project,
                team=_make_team(),
                milestone_map={"M1: Test": {"id": "ms-id", "name": "M1: Test"}},
                publish=publish,
            )

        self.assertIn(
            ("issue_labels.graphql", {"id": "mapped-issue-id", "first": 100}),
            fake.calls,
        )
        sent_label_ids = fake.last_update()["input"]["labelIds"]
        self.assertIn("label-custom", sent_label_ids)
        self.assertIn("label-area", sent_label_ids)

    def test_publish_yaml_mapped_issue_paginates_labels(self) -> None:
        """Paginated mapped-issue label fetches accumulate every page."""

        task = _make_task(task_id="T1", title="Mapped", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        project = _make_project(issues=[])
        publish = {
            "tasks": {
                "T1": {
                    "issue": "TEST-9",
                    "issueId": "mapped-issue-id",
                    "url": "https://linear.app/test/issue/TEST-9",
                }
            }
        }

        fake = self._make_fake_client(project)
        fake.label_pages["mapped-issue-id"] = [
            [
                {"id": "label-page1-a", "name": "page1-a"},
                {"id": "label-page1-b", "name": "page1-b"},
            ],
            [
                {"id": "label-page2-a", "name": "page2-a"},
                {"id": "label-page2-b", "name": "page2-b"},
            ],
        ]

        with tempfile.TemporaryDirectory() as tmp:
            package.repo_root = Path(tmp)
            ctl.ensure_issues(
                client=fake.real_client,
                package=package,
                project=project,
                team=_make_team(),
                milestone_map={"M1: Test": {"id": "ms-id", "name": "M1: Test"}},
                publish=publish,
            )

        sent_label_ids = fake.last_update()["input"]["labelIds"]
        self.assertIn("label-page1-a", sent_label_ids)
        self.assertIn("label-page1-b", sent_label_ids)
        self.assertIn("label-page2-a", sent_label_ids)
        self.assertIn("label-page2-b", sent_label_ids)
        # Ensure both pages were requested (cursor progression).
        paginated_calls = [
            v for n, v in fake.calls if n == "issue_labels.graphql"
        ]
        self.assertEqual(len(paginated_calls), 2)
        self.assertNotIn("after", paginated_calls[0])
        self.assertEqual(paginated_calls[1].get("after"), "1")

    def test_truncated_project_labels_fail_before_update(self) -> None:
        """A truncated label page on the project response aborts the run."""

        task = _make_task(task_id="T1", title="Truncated", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        truncated = _make_existing_issue(
            issue_id="issue-trunc",
            identifier="TEST-2",
            labels=[{"id": "label-x", "name": "area:planning"}],
        )
        truncated["labels"]["pageInfo"]["hasNextPage"] = True
        project = _make_project(issues=[truncated])

        fake = self._make_fake_client(project)

        with tempfile.TemporaryDirectory() as tmp:
            package.repo_root = Path(tmp)
            with self.assertRaises(ctl.LinearError) as ctx:
                ctl.load_project_state(fake.real_client, "test-project")
        self.assertIn("labels page was truncated", str(ctx.exception))

    # ---- Repo-aware desired state -----------------------------------------

    def test_repo_aware_leaf_replaces_stale_repo(self) -> None:
        """``DesiredRepo.managed`` replaces an existing repo label."""

        task = _make_task(task_id="T1", title="Leaf", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        existing = _make_existing_issue(
            issue_id="issue-4",
            identifier="TEST-5",
            labels=[
                {"id": "label-stale-repo", "name": "repo:old"},
                {"id": "label-area", "name": "area:planning"},
            ],
        )
        project = _make_project(issues=[existing])
        # Pre-register ``repo:old`` so ``_lookup_repo_label_id`` finds it
        # and we exercise the managed-slug path without creating new labels.
        fake = self._make_fake_client(project)
        fake.label_names_to_ids["repo:old"] = "label-stale-repo"

        with tempfile.TemporaryDirectory() as tmp:
            package.repo_root = Path(tmp)
            ctl.ensure_issues(
                client=fake.real_client,
                package=package,
                project=project,
                team=_make_team(),
                milestone_map={"M1: Test": {"id": "ms-id", "name": "M1: Test"}},
                publish={},
                desired_repo_by_task={"T1": DesiredRepo.managed("old")},
            )

        sent_label_ids = fake.last_update()["input"]["labelIds"]
        self.assertIn("label-stale-repo", sent_label_ids)
        self.assertIn("label-area", sent_label_ids)

    def test_repo_aware_parent_clears_stale_repo(self) -> None:
        """``DesiredRepo.cleared()`` drops all existing repo labels."""

        task = _make_task(task_id="T1", title="Parent", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        existing = _make_existing_issue(
            issue_id="issue-5",
            identifier="TEST-6",
            labels=[
                {"id": "label-stale-repo", "name": "repo:old"},
                {"id": "label-area", "name": "area:planning"},
            ],
        )
        project = _make_project(issues=[existing])

        fake = self._ensure_issues(
            package=package,
            project=project,
            desired_repo_by_task={"T1": DesiredRepo.cleared()},
        )

        sent_label_ids = fake.last_update()["input"]["labelIds"]
        self.assertNotIn("label-stale-repo", sent_label_ids)
        self.assertIn("label-area", sent_label_ids)

    def test_existing_areas_only_update_preserves_unmanaged(self) -> None:
        """No regression: ordinary area publish keeps unmanaged labels."""

        task = _make_task(task_id="T1", title="Standard", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        existing = _make_existing_issue(
            issue_id="issue-6",
            identifier="TEST-7",
            labels=[
                {"id": "label-area-existing", "name": "area:planning"},
                {"id": "label-unmanaged", "name": "ops:triage"},
            ],
        )
        project = _make_project(issues=[existing])

        fake = self._ensure_issues(package=package, project=project)

        sent_label_ids = fake.last_update()["input"]["labelIds"]
        self.assertIn("label-area-existing", sent_label_ids)
        self.assertIn("label-unmanaged", sent_label_ids)

    def test_repo_managed_missing_label_is_created_lazily(self) -> None:
        """LOC-25: managed ``repo:<slug>`` labels are created on demand.

        ``ensure_repo_labels`` lazily creates the ``repo:<slug>`` label
        before any issue gets updated, so a leaf whose slug does not
        already exist on the team still ends up with exactly one
        ``repo:<slug>`` label after publish. The merge helper no longer
        raises a ``ValueError`` for that case because the cache now
        carries the newly created id.
        """

        task = _make_task(task_id="T1", title="Missing repo", areas=["planning"])
        package = _make_package(tasks={"T1": task})
        existing = _make_existing_issue(
            issue_id="issue-7",
            identifier="TEST-8",
            labels=[{"id": "label-area", "name": "area:planning"}],
        )
        project = _make_project(issues=[existing])

        fake = self._make_fake_client(project)

        with tempfile.TemporaryDirectory() as tmp:
            package.repo_root = Path(tmp)
            ctl.ensure_issues(
                client=fake.real_client,
                package=package,
                project=project,
                team=_make_team(),
                milestone_map={"M1: Test": {"id": "ms-id", "name": "M1: Test"}},
                publish={},
                desired_repo_by_task={
                    "T1": DesiredRepo.managed("does-not-exist")
                },
            )

        # The lazy creator fired before any issue update.
        create_names = [
            variables["input"]["name"]
            for name, variables in fake.calls
            if name == "issue_label_create.graphql"
        ]
        self.assertIn("repo:does-not-exist", create_names)

        # The final ``issue_update`` payload carried the newly-created
        # repo label id (the label id is mocked as ``label-repo:does-not-exist``).
        sent_label_ids = fake.last_update()["input"]["labelIds"]
        self.assertIn("label-repo-does-not-exist", sent_label_ids)
        self.assertIn("label-area", sent_label_ids)


if __name__ == "__main__":
    unittest.main()
