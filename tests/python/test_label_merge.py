#!/usr/bin/env python3
"""Unit tests for the namespace-aware additive label merge helper."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path


# Make the converter scripts importable.
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
sys.path.insert(0, str(REPO_ROOT / ".agents/skills/convert-tasks-to-linear/scripts"))

from label_merge import (  # noqa: E402
    AREA_PREFIX,
    REPO_PREFIX,
    DesiredRepo,
    has_label_namespace,
    merge_label_ids,
)


class HasLabelNamespaceTests(unittest.TestCase):
    def test_area_prefix_matches_case_insensitively(self) -> None:
        self.assertTrue(has_label_namespace("area:planning", AREA_PREFIX))
        self.assertTrue(has_label_namespace("Area:Planning", AREA_PREFIX))
        self.assertTrue(has_label_namespace("AREA:planning", AREA_PREFIX))

    def test_area_prefix_rejects_other_namespaces(self) -> None:
        self.assertFalse(has_label_namespace("repo:opensymphony", AREA_PREFIX))
        self.assertFalse(has_label_namespace("bug", AREA_PREFIX))
        self.assertFalse(has_label_namespace("areaic", AREA_PREFIX))

    def test_repo_prefix_matches_case_insensitively(self) -> None:
        self.assertTrue(has_label_namespace("repo:opensymphony", REPO_PREFIX))
        self.assertTrue(has_label_namespace("Repo:OpenSymphony", REPO_PREFIX))
        self.assertFalse(has_label_namespace("repobug", REPO_PREFIX))


class MergeLabelIdsTests(unittest.TestCase):
    """Coverage for the LOC-22 acceptance criteria over the pure helper."""

    # ---- Legacy hand-set repo preservation -----------------------------------

    def test_legacy_handset_repo_label_is_preserved(self) -> None:
        existing = {"repo:handcrafted": "label-handcrafted"}
        result = merge_label_ids(
            existing,
            desired_areas=None,
            desired_repo=None,
        )
        self.assertEqual(result, ["label-handcrafted"])

    def test_legacy_handset_repo_label_is_preserved_with_areas_managed(self) -> None:
        existing = {
            "repo:handcrafted": "label-handcrafted",
            "area:planning": "label-planning",
        }
        result = merge_label_ids(
            existing,
            desired_areas=["planning"],
            desired_repo=None,
            area_ids_by_slug={"planning": "label-planning"},
        )
        self.assertEqual(result, ["label-planning", "label-handcrafted"])

    # ---- Repo-aware leaf desired state --------------------------------------

    def test_repo_aware_leaf_replaces_existing_repo(self) -> None:
        existing = {
            "repo:oldrepo": "label-old",
            "area:planning": "label-area",
        }
        result = merge_label_ids(
            existing,
            desired_areas=["planning"],
            desired_repo=DesiredRepo.managed("newrepo"),
            area_ids_by_slug={"planning": "label-area"},
            repo_id_by_slug={"newrepo": "label-new"},
        )
        # Old repo:oldrepo is gone, area:planning retained (id match), new repo
        # added in deterministic order at the end.
        self.assertEqual(result, ["label-area", "label-new"])

    def test_repo_aware_leaf_with_unmanaged_preserved(self) -> None:
        existing = {
            "repo:oldrepo": "label-old",
            "area:planning": "label-area",
            "priority:high": "label-priority",
        }
        result = merge_label_ids(
            existing,
            desired_areas=["planning"],
            desired_repo=DesiredRepo.managed("newrepo"),
            area_ids_by_slug={"planning": "label-area"},
            repo_id_by_slug={"newrepo": "label-new"},
        )
        self.assertEqual(
            result,
            ["label-priority", "label-area", "label-new"],
        )

    # ---- Repo-aware parent/review cleanup -----------------------------------

    def test_repo_aware_parent_clears_existing_repo(self) -> None:
        existing = {
            "repo:stale": "label-stale",
            "area:planning": "label-area",
        }
        result = merge_label_ids(
            existing,
            desired_areas=["planning"],
            desired_repo=DesiredRepo.cleared(),
            area_ids_by_slug={"planning": "label-area"},
        )
        self.assertEqual(result, ["label-area"])

    def test_repo_aware_parent_with_unmanaged_preserved(self) -> None:
        existing = {
            "repo:stale": "label-stale",
            "area:planning": "label-area",
            "ops:needs-triage": "label-ops",
        }
        result = merge_label_ids(
            existing,
            desired_areas=["planning"],
            desired_repo=DesiredRepo.cleared(),
            area_ids_by_slug={"planning": "label-area"},
        )
        self.assertEqual(result, ["label-ops", "label-area"])

    # ---- Unmanaged label preservation ---------------------------------------

    def test_unmanaged_labels_are_preserved(self) -> None:
        existing = {
            "area:planning": "label-area",
            "custom:thing": "label-custom",
            "priority:high": "label-priority",
        }
        result = merge_label_ids(
            existing,
            desired_areas=["planning"],
            desired_repo=None,
            area_ids_by_slug={"planning": "label-area"},
        )
        self.assertEqual(
            result,
            ["label-custom", "label-priority", "label-area"],
        )

    # ---- areas present vs absent ---------------------------------------------

    def test_areas_present_replaces_existing_area(self) -> None:
        existing = {
            "area:planning": "label-planning",
            "area:legacy": "label-legacy",
        }
        result = merge_label_ids(
            existing,
            desired_areas=["planning"],
            desired_repo=None,
            area_ids_by_slug={"planning": "label-planning"},
        )
        # Only area:planning kept; area:legacy dropped.
        self.assertEqual(result, ["label-planning"])

    def test_areas_absent_preserves_existing_areas(self) -> None:
        existing = {
            "area:planning": "label-planning",
            "area:legacy": "label-legacy",
        }
        result = merge_label_ids(
            existing,
            desired_areas=None,
            desired_repo=None,
        )
        # Both area labels preserved in sorted order.
        self.assertEqual(result, ["label-legacy", "label-planning"])

    def test_areas_present_empty_list_clears_all_areas(self) -> None:
        existing = {"area:planning": "label-planning"}
        result = merge_label_ids(
            existing,
            desired_areas=[],
            desired_repo=None,
            area_ids_by_slug={},
        )
        self.assertEqual(result, [])

    # ---- Case-insensitive reserved-prefix matching --------------------------

    def test_case_insensitive_area_match(self) -> None:
        existing = {"AREA:Planning": "label-1", "Area:Legacy": "label-2"}
        result = merge_label_ids(
            existing,
            desired_areas=["planning"],
            desired_repo=None,
            area_ids_by_slug={"planning": "label-new"},
        )
        # Both AREA:* are recognised as managed; AREA:Legacy dropped because
        # areas is present and only area:planning is requested.
        self.assertEqual(result, ["label-new"])

    def test_case_insensitive_repo_match(self) -> None:
        existing = {"REPO:Stale": "label-stale"}
        result = merge_label_ids(
            existing,
            desired_areas=None,
            desired_repo=DesiredRepo.cleared(),
        )
        self.assertEqual(result, [])

    # ---- Repo-preserved mode (legacy bootstrap) ------------------------------

    def test_explicit_preserved_repo_keeps_existing_repo(self) -> None:
        existing = {"repo:handcrafted": "label-handcrafted"}
        result = merge_label_ids(
            existing,
            desired_areas=None,
            desired_repo=DesiredRepo.preserved(),
        )
        self.assertEqual(result, ["label-handcrafted"])

    # ---- Deterministic ordering ---------------------------------------------

    def test_result_order_is_deterministic(self) -> None:
        existing = {
            "area:b": "id-b",
            "area:a": "id-a",
            "repo:z": "id-z",
            "unmanaged:2": "id-u2",
            "unmanaged:1": "id-u1",
        }
        result = merge_label_ids(
            existing,
            desired_areas=None,
            desired_repo=None,
        )
        # Unmanaged first (insertion order), then sorted area, then sorted repo.
        self.assertEqual(result, ["id-u2", "id-u1", "id-a", "id-b", "id-z"])

    # ---- Error paths --------------------------------------------------------

    def test_missing_area_mapping_raises(self) -> None:
        with self.assertRaises(ValueError):
            merge_label_ids(
                {},
                desired_areas=["planning"],
                desired_repo=None,
                area_ids_by_slug={},
            )

    def test_missing_repo_mapping_raises(self) -> None:
        with self.assertRaises(ValueError):
            merge_label_ids(
                {},
                desired_areas=None,
                desired_repo=DesiredRepo.managed("nope"),
                repo_id_by_slug={},
            )

    def test_managed_repo_requires_slug(self) -> None:
        with self.assertRaises(ValueError):
            DesiredRepo.managed("")

    def test_none_existing_raises(self) -> None:
        with self.assertRaises(ValueError):
            merge_label_ids(
                None,  # type: ignore[arg-type]
                desired_areas=None,
                desired_repo=None,
            )


if __name__ == "__main__":
    unittest.main()