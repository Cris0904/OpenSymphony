#!/usr/bin/env bash
#
# Synthetic E2E for the LOC-30 repo-aware publish path.
#
# Builds a deterministic task package with one parent/review node and
# two leaf nodes pointing at different project-set repo slugs
# (``opensymphony`` and ``OpenSymphony-Config``). The script exercises
# the full validate -> dry-run -> apply chain against a disposable
# Linear project, then runs a harness check against the resulting
# Linear labels so the publish path is proven end-to-end.
#
# Modes:
#
#   --dry-run   (default)  Validate + dry-run + harness check on the
#                          dry-run output. No Linear writes.
#   --live                  Run ``apply`` against the project given by
#                          ``--project-slug`` and run the harness check
#                          against the live Linear state.
#
# Required env (live mode only):
#
#   LINEAR_API_KEY          Personal API key for the disposable project.
#   --project-slug <slug>   Linear project slugId (project must already
#                          exist; the script treats it as disposable).
#
# Exit codes:
#
#   0  all checks passed
#   1  validate / dry-run / apply failed
#   2  harness check failed (label contract not satisfied)

set -euo pipefail

# Verify PyYAML is available before the harness invokes the converter
# subprocesses. The script embeds ``python3 -c "import yaml, ..."`` calls
# that surface a confusing ModuleNotFoundError when PyYAML is missing.
if ! python3 -c 'import yaml' 2>/dev/null; then
    echo "PyYAML is required but not installed; install with 'pip install PyYAML'." >&2
    exit 1
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
CONVERTER="${REPO_ROOT}/.agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py"
LINEAR_HELPER="${REPO_ROOT}/.agents/skills/linear/scripts/linear_graphql.py"
QUERY_DIR="${REPO_ROOT}/.agents/skills/linear/queries"
ISSUE_BY_KEY_QUERY="${QUERY_DIR}/issue_by_key.graphql"
ISSUE_LABELS_QUERY="${QUERY_DIR}/issue_labels.graphql"

MODE="dry-run"
PROJECT_SLUG=""
TEAM_KEY=""
KEEP_TMP="0"

usage() {
    cat <<'USAGE'
Usage: scripts/multirepo_live_linear_e2e.sh [options]

Options:
  --dry-run          Validate + dry-run + harness check (default).
  --live             Run ``apply`` against the disposable Linear project
                     and run the harness check on live labels.
  --project-slug SLUG Linear project slugId (required for --live).
  --team-key KEY     Linear team key for multi-team projects (--live).
  --keep-tmp         Keep the temporary work directory after the run.
  -h, --help         Show this help.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)
            MODE="dry-run"
            shift
            ;;
        --live)
            MODE="live"
            shift
            ;;
        --project-slug)
            PROJECT_SLUG="${2:-}"
            shift 2
            ;;
        --team-key)
            TEAM_KEY="${2:-}"
            shift 2
            ;;
        --keep-tmp)
            KEEP_TMP="1"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage >&2
            exit 64
            ;;
    esac
done

if [[ "${MODE}" == "live" ]]; then
    if [[ -z "${LINEAR_API_KEY:-}" ]]; then
        echo "LINEAR_API_KEY is required for --live mode" >&2
        exit 64
    fi
    if [[ -z "${PROJECT_SLUG}" ]]; then
        echo "--project-slug is required for --live mode" >&2
        exit 64
    fi
fi

if [[ ! -f "${CONVERTER}" ]]; then
    echo "Converter not found: ${CONVERTER}" >&2
    exit 1
fi

if [[ ! -f "${LINEAR_HELPER}" ]]; then
    echo "Linear helper not found: ${LINEAR_HELPER}" >&2
    exit 1
fi

TMP_ROOT="$(mktemp -d -t loc30-multirepo-e2e.XXXXXX)"
trap_cleanup() {
    if [[ "${KEEP_TMP}" == "1" ]]; then
        echo "keeping temporary directory: ${TMP_ROOT}"
    else
        rm -rf "${TMP_ROOT}"
    fi
}
trap trap_cleanup EXIT

PROJECT_SET_PATH="${TMP_ROOT}/project-set.yaml"
TASKS_DIR="${TMP_ROOT}/docs/tasks"
TASK_PACKAGE="${TMP_ROOT}/task-package.yaml"
PUBLISH_FILE="${TMP_ROOT}/linear-publish.yaml"
mkdir -p "${TASKS_DIR}"

cat > "${PROJECT_SET_PATH}" <<'YAML'
schema_version: 1

project_set:
  slug: loc30-multirepo-fixture
  name: LOC-30 Multi-Repo Fixture
  projects:
    - slug: opensymphony
      name: OpenSymphony
      repos:
        - slug: opensymphony
          url: git@github.com:example/opensymphony.git
          default_branch: main
        - slug: OpenSymphony-Config
          url: git@github.com:example/opensymphony-config.git
          default_branch: main
YAML

write_task() {
    local task_id="$1"
    local title="$2"
    local parent="$3"
    local repo="$4"
    local file="${TASKS_DIR}/${task_id}.md"

    local parent_yaml="null"
    if [[ -n "${parent}" ]]; then
        parent_yaml="\"${parent}\""
    fi

    local repo_yaml="null"
    if [[ -n "${repo}" ]]; then
        repo_yaml="\"${repo}\""
    fi

    cat > "${file}" <<EOF
---
id: ${task_id}
title: "${title}"
milestone: "M-LOC30-E2E"
priority: 3
estimate: 1
blockedBy: []
blocks: []
parent: ${parent_yaml}
areas:
  - linear
repo: ${repo_yaml}
---

## Summary

${title} synthetic task for LOC-30 multi-repo E2E.

## Scope

### In scope

- Exercise repo-aware publish path.

### Out of scope

- Anything else.

## Deliverables

- One Linear issue with the right labels.

## Acceptance Criteria

- [ ] Linear labels satisfy the harness contract.

## Test Plan

- Run this script.

## Context

- LOC-30 / OSYM-627.

## Definition of Ready

- [ ] N/A
EOF
}

write_task "TASK-LOC30-PARENT" "LOC-30 E2E Parent" "" ""
write_task "TASK-LOC30-LEAF-A" "LOC-30 E2E Leaf A (opensymphony)" "TASK-LOC30-PARENT" "opensymphony"
write_task "TASK-LOC30-LEAF-B" "LOC-30 E2E Leaf B (OpenSymphony-Config)" "TASK-LOC30-PARENT" "OpenSymphony-Config"

cat > "${TASK_PACKAGE}" <<YAML
planningWave: loc30-multirepo-e2e
tasksDir: ${TASKS_DIR}
milestones:
  - "M-LOC30-E2E"
tasks:
  - id: TASK-LOC30-PARENT
    file: ${TASKS_DIR}/TASK-LOC30-PARENT.md
  - id: TASK-LOC30-LEAF-A
    file: ${TASKS_DIR}/TASK-LOC30-LEAF-A.md
  - id: TASK-LOC30-LEAF-B
    file: ${TASKS_DIR}/TASK-LOC30-LEAF-B.md
YAML

echo "==> validate"
python3 "${CONVERTER}" validate \
    --manifest "${TASK_PACKAGE}" \
    --repo-root "${REPO_ROOT}" \
    --project-set "${PROJECT_SET_PATH}"

echo
echo "==> dry-run"
DRY_RUN_OUT="${TMP_ROOT}/dry-run.out"
python3 "${CONVERTER}" dry-run \
    --manifest "${TASK_PACKAGE}" \
    --repo-root "${REPO_ROOT}" \
    --project-set "${PROJECT_SET_PATH}" \
    | tee "${DRY_RUN_OUT}"

echo
echo "==> harness check (dry-run output)"
if ! grep -q "TASK-LOC30-PARENT repo=-" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing 'TASK-LOC30-PARENT repo=-'" >&2
    exit 2
fi
if ! grep -q "TASK-LOC30-LEAF-A repo=opensymphony" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing 'TASK-LOC30-LEAF-A repo=opensymphony'" >&2
    exit 2
fi
if ! grep -q "TASK-LOC30-LEAF-B repo=OpenSymphony-Config" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing 'TASK-LOC30-LEAF-B repo=OpenSymphony-Config'" >&2
    exit 2
fi
if ! grep -q "Repo labels to publish (managed):" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing 'Repo labels to publish (managed):'" >&2
    exit 2
fi
if ! grep -qE "^- repo:opensymphony\$" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing managed slug 'repo:opensymphony'" >&2
    exit 2
fi
if ! grep -qE "^- repo:OpenSymphony-Config\$" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing managed slug 'repo:OpenSymphony-Config'" >&2
    exit 2
fi

if [[ "${MODE}" == "dry-run" ]]; then
    echo
    echo "dry-run harness check: OK"
    exit 0
fi

echo
echo "==> apply (live Linear)"
apply_args=(
    apply
    --manifest "${TASK_PACKAGE}"
    --repo-root "${REPO_ROOT}"
    --project-set "${PROJECT_SET_PATH}"
    --project-slug "${PROJECT_SLUG}"
    --publish "${PUBLISH_FILE}"
)
if [[ -n "${TEAM_KEY}" ]]; then
    apply_args+=(--team-key "${TEAM_KEY}")
fi
python3 "${CONVERTER}" "${apply_args[@]}"

if [[ ! -f "${PUBLISH_FILE}" ]]; then
    echo "harness: apply did not write ${PUBLISH_FILE}" >&2
    exit 1
fi

echo
echo "==> harness check (live Linear labels)"
check_labels() {
    local task_id="$1"
    local expect_repo_count="$2"
    local expect_repo_names="$3"  # space-separated; "" means none
    local issue_key
    issue_key="$(python3 -c "
import sys, yaml
with open('${PUBLISH_FILE}', encoding='utf-8') as f:
    data = yaml.safe_load(f) or {}
tasks = data.get('tasks', {}) if isinstance(data, dict) else {}
print(tasks.get('${task_id}', {}).get('issue', '') or '')
")"
    if [[ -z "${issue_key}" ]]; then
        echo "harness: ${task_id} missing issue key in ${PUBLISH_FILE}" >&2
        return 1
    fi

    local label_payload
    label_payload="$(python3 -c "
import json, subprocess, sys
vars = {'key': '${issue_key}'}
result = subprocess.run(
    ['python3', '${LINEAR_HELPER}',
     '--query-file', '${ISSUE_BY_KEY_QUERY}',
     '--variables', json.dumps(vars)],
    cwd='${REPO_ROOT}',
    capture_output=True, text=True, check=True,
)
data = json.loads(result.stdout)
issue = data['data']['issue']
issue_id = issue['id']
vars = {'id': issue_id, 'first': 100}
result = subprocess.run(
    ['python3', '${LINEAR_HELPER}',
     '--query-file', '${ISSUE_LABELS_QUERY}',
     '--variables', json.dumps(vars)],
    cwd='${REPO_ROOT}',
    capture_output=True, text=True, check=True,
)
data = json.loads(result.stdout)
print(issue_id)
for node in data['data']['issue']['labels']['nodes']:
    print(node['name'])
")"

    local issue_id
    issue_id="$(echo "${label_payload}" | head -n 1)"
    local labels
    labels="$(echo "${label_payload}" | tail -n +2)"

    echo "  ${task_id} (${issue_key}, ${issue_id}) labels:"
    while IFS= read -r label; do
        [[ -z "${label}" ]] && continue
        echo "    - ${label}"
    done <<< "${labels}"

    local repo_count=0
    local mismatched=0
    while IFS= read -r label; do
        [[ -z "${label}" ]] && continue
        local label_lower
        label_lower="$(printf '%s' "${label}" | tr '[:upper:]' '[:lower:]')"
        if [[ "${label_lower}" == repo:* ]]; then
            repo_count=$((repo_count + 1))
            local want=0
            for name in ${expect_repo_names}; do
                if [[ "${label}" == "${name}" ]]; then
                    want=1
                    break
                fi
            done
            if [[ "${want}" -eq 0 ]]; then
                mismatched=$((mismatched + 1))
            fi
        fi
    done <<< "${labels}"

    if [[ "${repo_count}" -ne "${expect_repo_count}" ]]; then
        echo "harness: ${task_id} expected ${expect_repo_count} repo:* labels, found ${repo_count}" >&2
        return 1
    fi
    if [[ "${mismatched}" -ne 0 ]]; then
        echo "harness: ${task_id} had ${mismatched} unexpected repo:* label(s)" >&2
        return 1
    fi
    return 0
}

check_labels "TASK-LOC30-PARENT" 0 "" || exit 2
check_labels "TASK-LOC30-LEAF-A" 1 "repo:opensymphony" || exit 2
check_labels "TASK-LOC30-LEAF-B" 1 "repo:OpenSymphony-Config" || exit 2

echo
echo "==> re-apply (idempotency)"
apply_args=(
    apply
    --manifest "${TASK_PACKAGE}"
    --repo-root "${REPO_ROOT}"
    --project-set "${PROJECT_SET_PATH}"
    --project-slug "${PROJECT_SLUG}"
    --publish "${PUBLISH_FILE}"
)
if [[ -n "${TEAM_KEY}" ]]; then
    apply_args+=(--team-key "${TEAM_KEY}")
fi
python3 "${CONVERTER}" "${apply_args[@]}"

check_labels "TASK-LOC30-PARENT" 0 "" || exit 2
check_labels "TASK-LOC30-LEAF-A" 1 "repo:opensymphony" || exit 2
check_labels "TASK-LOC30-LEAF-B" 1 "repo:OpenSymphony-Config" || exit 2

echo
echo "live harness check: OK"
exit 0
