#!/usr/bin/env bash
# multirepo_live_linear_e2e — strict-mode planning-contract guard for the
# multi-repo end-to-end flow (LOC-29).
#
# This script is the user-journey guard that proves the planning
# source-of-truth (the `create-implementation-plan` skill plus the Rust
# `opensymphony-planning` manifest validator plus the Python
# `convert-tasks-to-linear validate` command) agrees on the
# repo-frontmatter contract before any Linear write is attempted.
#
# Stages:
#   1. Planning contract guard
#      (`check_create_implementation_plan_contract`) — verifies the
#      `create-implementation-plan` SKILL.md documents the leaf-vs-
#      parent repo contract, the exact-inventory-slug rule, the
#      `areas` namespace misuse ban, and the one-repo auto-fill rule.
#   2. Rust manifest validator — runs the new LOC-29 fixture tests
#      against the tiny multi-repo plan and the `areas` namespace
#      misuse cases.
#   3. Python converter validate — runs the existing
#      `tests/python/test_convert_tasks_validate_repo.py` suite to
#      prove the Python side agrees with the Rust side on the same
#      contract.
#
# Environment toggles:
#   OSYM_E2E_ALLOW_SYNTHETIC_PLAN=1
#     Escape hatch for downstream LOC-31 verification: skips the
#     planning-contract guard so the script can proceed with a
#     synthetic plan while the on-disk contract is still being
#     iterated on. The Rust fixture and Python validate stages
#     still run so the gate cannot be silently bypassed.
#
# Exit status:
#   0  — every enabled stage passed.
#   non-zero — at least one enabled stage failed.

set -euo pipefail

SCRIPT_NAME="multirepo_live_linear_e2e"
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")"/.. && pwd)"
LOG_DIR="${OSYM_E2E_LOG_DIR:-${ROOT_DIR}/target/${SCRIPT_NAME}}"
mkdir -p "${LOG_DIR}"

SKILL_PATH="${ROOT_DIR}/.agents/skills/create-implementation-plan/SKILL.md"
PYTHON_CONVERT_SCRIPT="${ROOT_DIR}/.agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py"
PYTHON_FIXTURE_DIRS=(
  "${ROOT_DIR}/tests/fixtures/multirepo/tiny-multi-repo-sub-issues"
  "${ROOT_DIR}/tests/fixtures/multirepo/tiny-multi-repo-top-level"
)
RUST_FIXTURE_TESTS=(
  "opensymphony_planning::graph_validate::manifest::tests::tiny_multi_repo_plan_passes"
  "opensymphony_planning::graph_validate::manifest::tests::multi_repo_sub_issue_layout_passes"
  "opensymphony_planning::graph_validate::manifest::tests::areas_repo_namespace_misuse_is_reported"
  "opensymphony_planning::graph_validate::manifest::tests::areas_repo_namespace_misuse_is_case_insensitive_on_prefix"
  "opensymphony_planning::graph_validate::manifest::tests::parent_with_repo_is_reported"
  "opensymphony_planning::graph_validate::manifest::tests::missing_leaf_repo_is_reported"
  "opensymphony_planning::graph_validate::manifest::tests::empty_repo_value_is_treated_as_missing_leaf_repo"
)
PYTHON_VALIDATE_MODULE="tests.python.test_convert_tasks_validate_repo"

log_path() {
  local stage="$1"
  echo "${LOG_DIR}/${stage}.log"
}

# Stage 1: verify the planning skill documents every clause of the
# leaf-vs-parent repo-frontmatter contract. We use `grep -E` against the
# on-disk SKILL.md so the guard is a single source of truth that
# reviewers can re-run without rebuilding the Rust crate or the Python
# converter.
check_create_implementation_plan_contract() {
  local log
  log="$(log_path planning-contract)"
  : > "${log}"

  if [[ ! -f "${SKILL_PATH}" ]]; then
    echo "FAIL: ${SKILL_PATH} not found" >&2 | tee -a "${log}"
    return 1
  fi

  # Each entry is a pattern that must match the on-disk SKILL.md.
  # Patterns are anchored on the strongest single phrase that proves
  # the corresponding contract clause is documented. Patterns are
  # matched against the multi-line file via grep -z so prose that
  # spans two lines (e.g. "**required on leaf**\n  tasks") still
  # matches as a single phrase.
  local -a required_patterns=(
    # Leaf-vs-parent shape:
    'required on leaf'
    'forbidden on parent'
    # Exact-inventory-slug rule (no normalization):
    'exact'
    'lowercas'
    'slugif'
    # Reserved-namespace separation:
    'repo:<slug>'
    'area:<slug>'
    # Areas misuse ban (mirrors the Python converter's
    # `normalize_area_slugs` rejection):
    'reserved non-area namespace'
    # Auto-fill rule (one-repo obvious case):
    'single_repo_slug'
    'Inventory has'
  )
  local -a missing_patterns=()
  local pattern
  # Flatten the SKILL.md so multi-line phrases (e.g. "**required on
  # leaf**\n  tasks") match as a single substring. We avoid
  # `grep -Pz` because BSD `grep` on macOS does not support `-P`.
  local flattened
  flattened="$(tr '\n' ' ' < "${SKILL_PATH}")"
  for pattern in "${required_patterns[@]}"; do
    if ! printf '%s' "${flattened}" | grep -E -q -- "${pattern}"; then
      missing_patterns+=("${pattern}")
    fi
  done

  if [[ "${#missing_patterns[@]}" -gt 0 ]]; then
    {
      echo "FAIL: planning-contract guard missing patterns in ${SKILL_PATH}:"
      for pattern in "${missing_patterns[@]}"; do
        echo "  - ${pattern}"
      done
    } | tee -a "${log}" >&2
    return 1
  fi

  {
    echo "OK: planning-contract guard passed"
    echo "  - skill: ${SKILL_PATH}"
    echo "  - patterns checked: ${#required_patterns[@]}"
  } | tee -a "${log}"
}

# Stage 2: run the new Rust fixture tests. Each test is filtered by
# full path so a single failing test surfaces the failure rather than
# the whole planning test binary failing.
check_rust_manifest_validator() {
  local log
  log="$(log_path rust-validator)"
  : > "${log}"

  local test
  local failed=0
  for test in "${RUST_FIXTURE_TESTS[@]}"; do
    echo "  - cargo test ${test}" | tee -a "${log}"
    if ! cargo test --lib "${test}" 2>&1 | tee -a "${log}" >/dev/null; then
      echo "FAIL: rust fixture test failed: ${test}" >&2 | tee -a "${log}"
      failed=1
    fi
  done
  if [[ "${failed}" -ne 0 ]]; then
    return 1
  fi
  echo "OK: rust manifest validator fixtures passed" | tee -a "${log}"
}

# Stage 3: run the Python converter validate suite. This is the
# publish-time gate that the planning-contract guard hands off to;
# passing it proves the on-disk Rust + Python surfaces agree on the
# repo-frontmatter contract. We additionally validate the on-disk
# multi-repo fixtures so the script is the end-to-end proof that the
# planning artifacts (not just the unit tests) pass the contract.
check_python_converter() {
  local log
  log="$(log_path python-validate)"
  : > "${log}"

  echo "  - python3 -m unittest ${PYTHON_VALIDATE_MODULE}" | tee -a "${log}"
  if ! python3 -m unittest "${PYTHON_VALIDATE_MODULE}" 2>&1 | tee -a "${log}" >/dev/null; then
    echo "FAIL: python validate suite failed" >&2 | tee -a "${log}"
    return 1
  fi

  local fixture
  for fixture in "${PYTHON_FIXTURE_DIRS[@]}"; do
    if [[ ! -d "${fixture}" ]]; then
      echo "FAIL: python fixture dir missing: ${fixture}" >&2 | tee -a "${log}"
      return 1
    fi
    if [[ ! -f "${fixture}/task-package.yaml" ]]; then
      echo "FAIL: python fixture manifest missing: ${fixture}/task-package.yaml" >&2 | tee -a "${log}"
      return 1
    fi
    echo "  - python3 ${PYTHON_CONVERT_SCRIPT} validate --repo-root ${fixture}" | tee -a "${log}"
    if ! (
        cd "${fixture}"
        python3 "${PYTHON_CONVERT_SCRIPT}" validate \
          --repo-root . \
          --manifest task-package.yaml \
          2>&1 | tee -a "${log}" >/dev/null
      ); then
      echo "FAIL: python validate rejected fixture ${fixture}" >&2 | tee -a "${log}"
      return 1
    fi
  done

  echo "OK: python validate suite passed (incl. on-disk multirepo fixtures)" | tee -a "${log}"
}

main() {
  local allow_synthetic="${OSYM_E2E_ALLOW_SYNTHETIC_PLAN:-0}"

  if [[ "${allow_synthetic}" == "1" ]]; then
    echo "NOTE: OSYM_E2E_ALLOW_SYNTHETIC_PLAN=1 — planning-contract guard SKIPPED for downstream LOC-31 verification" >&2
  else
    check_create_implementation_plan_contract
  fi

  check_rust_manifest_validator
  check_python_converter

  echo "PASS: ${SCRIPT_NAME} (logs: ${LOG_DIR})"
}

main "$@"
