#!/usr/bin/env python3
"""Namespace-aware additive label merge for the OpenSymphony task converter.

Linear's ``issue_update`` REPLACES the whole label set on every call. This
module gives the converter a deterministic way to compute the next label set
from:

* the existing label set on a Linear issue, and
* the *desired* managed namespaces (``area:*`` from task frontmatter,
  ``repo:*`` from a repo-aware package).

The merge is additive: any label whose name does not belong to a managed
namespace is preserved untouched, and a managed namespace is only rebuilt
when the task frontmatter (or its caller) asks for it.

Public surface
--------------

``merge_label_ids(existing_ids_by_name, *, desired_areas, desired_repo,
area_ids_by_slug, repo_id_by_slug)`` returns a list of label ids in a
deterministic order, suitable for assigning to ``labelIds``.

* ``existing_ids_by_name`` maps every currently-attached label *name* to
  its label *id*.
* ``desired_areas`` is either a list of area slugs (managed exactly to that
  set, prefixed with ``area:``) or ``None`` to mean "preserve whatever
  ``area:*`` labels are already on the issue".
* ``desired_repo`` is a :class:`DesiredRepo` instance:

  * ``DesiredRepo.managed(slug)`` - exactly one ``repo:<slug>`` label.
  * ``DesiredRepo.cleared()`` - no ``repo:*`` labels (parent/review task).
  * ``DesiredRepo.preserved()`` - preserve existing ``repo:*`` labels.
  * ``None`` - same as ``DesiredRepo.preserved()`` (default).

* ``area_ids_by_slug`` / ``repo_id_by_slug`` map slug -> id for labels the
  converter is about to introduce.

The reserved prefixes (``area:`` / ``repo:``) match case-insensitively.

This helper is pure and dependency-free so it can be unit-tested in isolation
and reused by LOC-25 (planning-seeds-the-repo-skill-and-crate) without
pulling in any Linear I/O.
"""

from __future__ import annotations

from dataclasses import dataclass


AREA_PREFIX = "area:"
REPO_PREFIX = "repo:"


@dataclass(frozen=True)
class DesiredRepo:
    """Desired state for the ``repo:*`` namespace.

    ``kind`` is one of:

    * ``"managed"`` - exactly the slug from ``slug`` is desired.
    * ``"cleared"`` - no ``repo:*`` labels are desired (parent/review task).
    * ``"preserved"`` - existing ``repo:*`` labels should be kept untouched.
    """

    kind: str
    slug: str | None = None

    @classmethod
    def managed(cls, slug: str) -> "DesiredRepo":
        if not slug:
            raise ValueError("DesiredRepo.managed requires a non-empty slug")
        return cls(kind="managed", slug=slug)

    @classmethod
    def cleared(cls) -> "DesiredRepo":
        return cls(kind="cleared")

    @classmethod
    def preserved(cls) -> "DesiredRepo":
        return cls(kind="preserved")


def _starts_with_prefix(name: str, prefix: str) -> bool:
    """Return True when ``name`` starts with ``prefix`` (case-insensitive)."""

    return name.lower().startswith(prefix.lower())


def has_label_namespace(name: str, prefix: str) -> bool:
    """Return True if ``name`` belongs to the ``prefix`` namespace."""

    return _starts_with_prefix(name, prefix)


def merge_label_ids(
    existing_ids_by_name: dict[str, str],
    *,
    desired_areas: list[str] | None,
    desired_repo: DesiredRepo | None = None,
    area_ids_by_slug: dict[str, str] | None = None,
    repo_id_by_slug: dict[str, str] | None = None,
) -> list[str]:
    """Compute the next label-id list under the merge rules.

    See module docstring for the merge contract.
    """

    if existing_ids_by_name is None:
        raise ValueError("existing_ids_by_name is required")
    area_lookup = dict(area_ids_by_slug or {})
    repo_lookup = dict(repo_id_by_slug or {})

    area_ids: dict[str, str] = {}
    repo_ids: dict[str, str] = {}
    unmanaged_ids: list[str] = []

    for name, label_id in existing_ids_by_name.items():
        if _starts_with_prefix(name, AREA_PREFIX):
            area_ids[name.lower()] = label_id
        elif _starts_with_prefix(name, REPO_PREFIX):
            repo_ids[name.lower()] = label_id
        else:
            unmanaged_ids.append(label_id)

    # Unmanaged labels come first in their original order to keep merges
    # stable across runs; managed namespaces are sorted for determinism.
    result: list[str] = list(unmanaged_ids)

    if desired_areas is not None:
        for slug in desired_areas:
            label_id = area_lookup.get(slug)
            if not label_id:
                raise ValueError(
                    f"area slug {slug!r} has no id mapping; pass area_ids_by_slug"
                )
            result.append(label_id)
    else:
        for name in sorted(area_ids):
            result.append(area_ids[name])

    if desired_repo is not None:
        if desired_repo.kind == "managed":
            assert desired_repo.slug, "managed DesiredRepo requires a slug"
            label_id = repo_lookup.get(desired_repo.slug)
            if not label_id:
                raise ValueError(
                    f"repo slug {desired_repo.slug!r} has no id mapping; "
                    "pass repo_id_by_slug"
                )
            result.append(label_id)
        elif desired_repo.kind == "cleared":
            pass
        elif desired_repo.kind == "preserved":
            for name in sorted(repo_ids):
                result.append(repo_ids[name])
        else:
            raise ValueError(f"unknown DesiredRepo kind: {desired_repo.kind!r}")
    else:
        for name in sorted(repo_ids):
            result.append(repo_ids[name])

    return result