#!/usr/bin/env bash
#
# Live Multi-Repo E2E Verification (LOC-31).
#
# Deterministic E2E harness for the full multi-repo user journey:
#
#   plan -> task package -> Linear labels -> repo resolver
#        -> dispatch gate -> clone hook -> OpenHands.
#
# The script always exercises the strict planning-contract guard (the
# Rust manifest validator + Python `convert-tasks-to-linear validate`
# stage from LOC-29), then the converter publish path (LOC-30), and
# finally the live Linear + OpenHands dispatch stages.
#
# Strict mode is the default. Set ``OSYM_E2E_ALLOW_SYNTHETIC_PLAN=1``
# to bypass the planning-contract guard (escape hatch for the older
# LOC-30 harness which used synthetic plans; the LOC-31 ticket
# requires the strict path).
#
# Environment toggles:
#
#   OSYM_E2E_LIVE_LINEAR=1
#     Run the live Linear publish + label-check stage against the
#     disposable Linear project given by ``TEST_LINEAR_PROJECT_SLUG``.
#     Requires ``LINEAR_API_KEY``. The stage is skipped when unset.
#
#   TEST_LINEAR_PROJECT_SLUG
#     Disposable Linear project slugId used by the live Linear stage.
#     The script treats the project as disposable (it creates and
#     archives per-run child issues through the existing converter
#     publish path; cleanup is the operator's responsibility).
#
#   OPENSYMPHONY_LIVE_OPENHANDS=1
#     Run the live OpenHands dispatch stage against the disposable
#     Linear project. Requires ``LLM_MODEL`` + ``LLM_API_KEY`` and a
#     pre-built ``opensymphony`` binary at
#     ``${REPO_ROOT}/target/debug/opensymphony``. The stage is
#     skipped when unset.
#
#   OSYM_E2E_ALLOW_SYNTHETIC_PLAN=1
#     Escape hatch to skip the planning-contract guard. The strict
#     mode is what the LOC-31 ticket requires; this flag exists only
#     so downstream harness iterations can iterate on the publish
#     path without re-proving the contract on every run.
#
#   OSYM_E2E_KEEP_TMP=1
#     Preserve the temp artifact root after the script exits. By
#     default the temp root is removed on exit; the live Linear /
#     OpenHands stages force it on so evidence survives.
#
#   OSYM_E2E_LOG_DIR
#     Override the log directory. Defaults to
#     ``${REPO_ROOT}/target/multirepo_live_linear_e2e``.
#
# Exit codes:
#
#   0  all enabled stages passed.
#   1  validate / dry-run / apply / OpenHands dispatch failed.
#   2  harness check failed (label contract not satisfied).
#   64 invalid invocation / missing required env.

set -euo pipefail

SCRIPT_NAME="multirepo_live_linear_e2e"

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
PLANNER_GUARD="${REPO_ROOT}/scripts/multirepo_planner_contract_check.sh"

LOG_DIR="${OSYM_E2E_LOG_DIR:-${REPO_ROOT}/target/${SCRIPT_NAME}}"
mkdir -p "${LOG_DIR}"

OSYM_E2E_LIVE_LINEAR="${OSYM_E2E_LIVE_LINEAR:-0}"
OSYM_E2E_ALLOW_SYNTHETIC_PLAN="${OSYM_E2E_ALLOW_SYNTHETIC_PLAN:-0}"
OSYM_E2E_KEEP_TMP="${OSYM_E2E_KEEP_TMP:-0}"
OPENSYMPHONY_LIVE_OPENHANDS="${OPENSYMPHONY_LIVE_OPENHANDS:-0}"

if [[ "${OSYM_E2E_LIVE_LINEAR}" == "1" ]]; then
    if [[ -z "${LINEAR_API_KEY:-}" ]]; then
        echo "LINEAR_API_KEY is required when OSYM_E2E_LIVE_LINEAR=1" >&2
        exit 64
    fi
    if [[ -z "${TEST_LINEAR_PROJECT_SLUG:-}" ]]; then
        echo "TEST_LINEAR_PROJECT_SLUG is required when OSYM_E2E_LIVE_LINEAR=1" >&2
        exit 64
    fi
    # Live Linear mode always retains evidence.
    OSYM_E2E_KEEP_TMP="1"
fi

if [[ "${OPENSYMPHONY_LIVE_OPENHANDS}" == "1" ]]; then
    if [[ -z "${LLM_MODEL:-}" ]] || [[ -z "${LLM_API_KEY:-}" ]]; then
        echo "LLM_MODEL and LLM_API_KEY are required when OPENSYMPHONY_LIVE_OPENHANDS=1" >&2
        exit 64
    fi
    if [[ "${OSYM_E2E_LIVE_LINEAR}" != "1" ]]; then
        echo "OPENSYMPHONY_LIVE_OPENHANDS=1 requires OSYM_E2E_LIVE_LINEAR=1" >&2
        exit 64
    fi
    # Live OpenHands mode always retains evidence.
    OSYM_E2E_KEEP_TMP="1"

    # The orchestrator supervises its own OpenHands agent-server on the
    # default loopback port (127.0.0.1:8000). When the operator's install
    # has left a long-running server bound to that port (e.g. from
    # ``opensymphony install`` followed by ``tools/openhands-server/run-local.sh``)
    # the supervisor refuses to launch a second instance and the run exits
    # before any dispatch happens. The harness owns its environment for
    # the duration of the stage, so stop the pre-existing server first so
    # the supervisor can bind the port. Set
    # ``OSYM_E2E_PRESERVE_EXISTING_OPENHANDS=1`` to skip the cleanup
    # (e.g. when running against a shared remote OpenHands server).
    if [[ "${OSYM_E2E_PRESERVE_EXISTING_OPENHANDS:-0}" != "1" ]]; then
        OH_PORT="${OPENSYMPHONY_OPENHANDS_PORT:-8000}"
        OH_PIDS="$(lsof -nP -iTCP:"${OH_PORT}" -sTCP:LISTEN -t 2>/dev/null || true)"
        if [[ -n "${OH_PIDS}" ]]; then
            echo "==> preflight: stopping existing OpenHands server(s) on port ${OH_PORT}: ${OH_PIDS}"
            # Filter to only OpenHands agent-server PIDs to avoid killing
            # unrelated processes that happen to bind 127.0.0.1:8000. The
            # check matches the canonical executable basename
            # (``openhands.agent_server``) plus the package path
            # (``openhands/agent_server``) so we only ever touch the
            # orchestrator's supervised server, never an unrelated
            # process whose argv happens to contain the substring
            # ``openhands_server``.
            FILTERED=""
            for pid in ${OH_PIDS}; do
                cmd="$(ps -p "${pid}" -o command= 2>/dev/null || true)"
                base="$(basename "${cmd}" 2>/dev/null || true)"
                if [[ "${base}" == "openhands.agent_server" ]] \
                    || [[ "${cmd}" == */openhands/agent_server* ]] \
                    || [[ "${cmd}" == *openhands.agent_server.* ]]; then
                    FILTERED="${FILTERED} ${pid}"
                fi
            done
            if [[ -n "${FILTERED}" ]]; then
                echo "    killing OpenHands server pids:${FILTERED}"
                kill ${FILTERED} 2>/dev/null || true
                # Wait briefly for graceful exit.
                for _ in 1 2 3 4 5 6 7 8 9 10; do
                    if ! kill -0 ${FILTERED} 2>/dev/null; then
                        break
                    fi
                    sleep 0.2
                done
                # Force kill any survivors.
                kill -9 ${FILTERED} 2>/dev/null || true
            else
                echo "    WARNING: port ${OH_PORT} is bound by non-OpenHands processes; refusing to kill them." >&2
                echo "    Set OSYM_E2E_PRESERVE_EXISTING_OPENHANDS=1 to skip this check, or free port ${OH_PORT} manually." >&2
                exit 64
            fi
        fi
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

TMP_ROOT="$(mktemp -d -t loc31-multirepo-e2e.XXXXXX)"
trap_cleanup() {
    if [[ "${OSYM_E2E_KEEP_TMP}" == "1" ]]; then
        echo
        echo "keeping temporary directory: ${TMP_ROOT}"
        echo "log directory: ${LOG_DIR}"
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

# Disposable multi-repo project-set with two bare-repo-friendly
# ``file://`` URLs that the OpenHands dispatch stage will seed with
# marker files. The slugs (``repo-a`` / ``repo-b``) match the LOC-29
# fixtures and the marker files documented in the LOC-31 ticket body.
cat > "${PROJECT_SET_PATH}" <<'YAML'
schema_version: 1

project_set:
  slug: loc31-multirepo-fixture
  name: LOC-31 Multi-Repo Fixture
  projects:
    - slug: multi-repo-fixture
      name: LOC-31 Multi-Repo Fixture
      repos:
        - slug: repo-a
          url: file:///tmp/loc31-repo-a
          default_branch: main
        - slug: repo-b
          url: file:///tmp/loc31-repo-b
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
milestone: "M-LOC31-E2E"
priority: 3
estimate: 1
blockedBy: []
blocks: []
parent: ${parent_yaml}
areas:
  - e2e
repo: ${repo_yaml}
---

## Summary

${title} synthetic task for LOC-31 multi-repo E2E.

## Scope

### In scope

- Exercise repo-aware publish path end-to-end.

### Out of scope

- Anything else.

## Deliverables

- One Linear issue with the right labels.

## Acceptance Criteria

- [ ] Linear labels satisfy the harness contract.

## Test Plan

- Run this script.

## Context

- LOC-31 / OSYM-628.

## Definition of Ready

- [ ] Synthetic task; not gated on real prerequisites.
EOF
}

write_task "TASK-LOC31-PARENT" "LOC-31 E2E Parent" "" ""
write_task "TASK-LOC31-LEAF-A" "LOC-31 E2E Leaf A (repo-a)" "TASK-LOC31-PARENT" "repo-a"
write_task "TASK-LOC31-LEAF-B" "LOC-31 E2E Leaf B (repo-b)" "TASK-LOC31-PARENT" "repo-b"

cat > "${TASK_PACKAGE}" <<YAML
planningWave: loc31-multirepo-e2e
tasksDir: ${TASKS_DIR}
milestones:
  - "M-LOC31-E2E"
tasks:
  - id: TASK-LOC31-PARENT
    file: ${TASKS_DIR}/TASK-LOC31-PARENT.md
  - id: TASK-LOC31-LEAF-A
    file: ${TASKS_DIR}/TASK-LOC31-LEAF-A.md
  - id: TASK-LOC31-LEAF-B
    file: ${TASKS_DIR}/TASK-LOC31-LEAF-B.md
YAML

# Stage 1: planning-contract guard.
# Strict by default; the strict mode is what the LOC-31 ticket
# requires. ``OSYM_E2E_ALLOW_SYNTHETIC_PLAN=1`` is the documented
# escape hatch that lets downstream harness iterations skip the
# guard while iterating on the publish path.
echo "==> planning-contract guard (synthetic=${OSYM_E2E_ALLOW_SYNTHETIC_PLAN} [0=strict, 1=allow-synthetic])"
PLANNER_LOG="${LOG_DIR}/planning-contract.log"
: > "${PLANNER_LOG}"
if ! OSYM_E2E_ALLOW_SYNTHETIC_PLAN="${OSYM_E2E_ALLOW_SYNTHETIC_PLAN}" \
    "${PLANNER_GUARD}" >"${PLANNER_LOG}" 2>&1; then
    echo "harness: planning-contract guard failed; see ${PLANNER_LOG}" >&2
    exit 1
fi
echo "    OK: planning-contract guard passed"

echo
echo "==> validate (LOC-30 converter)"
python3 "${CONVERTER}" validate \
    --manifest "${TASK_PACKAGE}" \
    --repo-root "${REPO_ROOT}" \
    --project-set "${PROJECT_SET_PATH}" \
    | tee "${LOG_DIR}/validate.log"

echo
echo "==> dry-run (LOC-30 converter)"
DRY_RUN_OUT="${TMP_ROOT}/dry-run.out"
python3 "${CONVERTER}" dry-run \
    --manifest "${TASK_PACKAGE}" \
    --repo-root "${REPO_ROOT}" \
    --project-set "${PROJECT_SET_PATH}" \
    | tee "${DRY_RUN_OUT}" "${LOG_DIR}/dry-run.log"

echo
echo "==> harness check (dry-run output)"
if ! grep -q "TASK-LOC31-PARENT repo=-" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing 'TASK-LOC31-PARENT repo=-'" >&2
    exit 2
fi
if ! grep -q "TASK-LOC31-LEAF-A repo=repo-a" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing 'TASK-LOC31-LEAF-A repo=repo-a'" >&2
    exit 2
fi
if ! grep -q "TASK-LOC31-LEAF-B repo=repo-b" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing 'TASK-LOC31-LEAF-B repo=repo-b'" >&2
    exit 2
fi
if ! grep -q "Repo labels to publish (managed):" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing 'Repo labels to publish (managed):'" >&2
    exit 2
fi
if ! grep -qE "^- repo:repo-a\$" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing managed slug 'repo:repo-a'" >&2
    exit 2
fi
if ! grep -qE "^- repo:repo-b\$" "${DRY_RUN_OUT}"; then
    echo "harness: dry-run output missing managed slug 'repo:repo-b'" >&2
    exit 2
fi
echo "    OK: dry-run harness check passed"

if [[ "${OSYM_E2E_LIVE_LINEAR}" != "1" ]]; then
    echo
    echo "dry-run harness check: OK (live Linear stage skipped; set OSYM_E2E_LIVE_LINEAR=1 to run)"
    exit 0
fi

echo
echo "==> apply (live Linear publish)"
APPLY_LOG="${LOG_DIR}/apply.log"
python3 "${CONVERTER}" apply \
    --manifest "${TASK_PACKAGE}" \
    --repo-root "${REPO_ROOT}" \
    --project-set "${PROJECT_SET_PATH}" \
    --project-slug "${TEST_LINEAR_PROJECT_SLUG}" \
    --publish "${PUBLISH_FILE}" \
    | tee "${APPLY_LOG}"

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

    {
        echo "  ${task_id} (${issue_key}, ${issue_id}) labels:"
        while IFS= read -r label; do
            [[ -z "${label}" ]] && continue
            echo "    - ${label}"
        done <<< "${labels}"
    } | tee -a "${LOG_DIR}/live-labels.log"

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

: > "${LOG_DIR}/live-labels.log"
check_labels "TASK-LOC31-PARENT" 0 "" || exit 2
check_labels "TASK-LOC31-LEAF-A" 1 "repo:repo-a" || exit 2
check_labels "TASK-LOC31-LEAF-B" 1 "repo:repo-b" || exit 2

echo
echo "==> re-apply (idempotency)"
python3 "${CONVERTER}" apply \
    --manifest "${TASK_PACKAGE}" \
    --repo-root "${REPO_ROOT}" \
    --project-set "${PROJECT_SET_PATH}" \
    --project-slug "${TEST_LINEAR_PROJECT_SLUG}" \
    --publish "${PUBLISH_FILE}" \
    | tee -a "${APPLY_LOG}"

check_labels "TASK-LOC31-PARENT" 0 "" || exit 2
check_labels "TASK-LOC31-LEAF-A" 1 "repo:repo-a" || exit 2
check_labels "TASK-LOC31-LEAF-B" 1 "repo:repo-b" || exit 2

if [[ "${OPENSYMPHONY_LIVE_OPENHANDS}" != "1" ]]; then
    echo
    echo "live Linear harness check: OK (live OpenHands stage skipped; set OPENSYMPHONY_LIVE_OPENHANDS=1 to run)"
    echo "evidence: ${TMP_ROOT}"
    exit 0
fi

# ----------------------------------------------------------------------
# Live OpenHands dispatch stage.
#
# The stage builds the dogfood ``opensymphony`` binary, seeds two
# disposable bare repos with marker files, writes a temp repo copy
# that points its project-set at those bare remotes, launches the
# orchestrator against the disposable Linear project, waits for it
# to dispatch only the repo leaves, and captures hook side-effect
# evidence for both workspaces.
#
# This stage is opt-in because it (a) needs the live OpenHands
# credentials in the environment, and (b) takes a meaningful amount
# of wall-clock time (build + dispatch + clone).
# ----------------------------------------------------------------------

OH_STAGE_DIR="${TMP_ROOT}/openhands"
mkdir -p "${OH_STAGE_DIR}/workspaces" "${OH_STAGE_DIR}/repos" "${OH_STAGE_DIR}/run-logs"

# Pick a free port for the supervised OpenHands server. The orchestrator
# defaults to 8000, which collides with the host's long-running server
# and causes the supervisor to refuse to launch. Reserve 8001 (or
# OSYM_E2E_OPENHANDS_DISPATCH_PORT) for the per-test supervised server
# so both can coexist.
OH_DISPATCH_PORT="${OSYM_E2E_OPENHANDS_DISPATCH_PORT:-8001}"
# WORKSPACE_ROOT is consumed by WORKFLOW.md's
# ``workspace.root: ${WORKSPACE_ROOT}`` so workspaces land under the
# per-run artifact root (the default /symphony_workspaces path is
# not writable on every host). Set this BEFORE writing WORKFLOW.md
# so the heredoc interpolation picks up the correct path.
export WORKSPACE_ROOT="${OH_STAGE_DIR}/workspaces"

BINARY="${REPO_ROOT}/target/debug/opensymphony"
echo
echo "==> live OpenHands dispatch stage (workspace root: ${OH_STAGE_DIR}/workspaces, openhands port: ${OH_DISPATCH_PORT})"

if [[ ! -x "${BINARY}" ]]; then
    echo "building opensymphony binary at ${BINARY}..." | tee "${LOG_DIR}/build.log"
    if ! (cd "${REPO_ROOT}" && cargo build --bin opensymphony) >"${LOG_DIR}/build.log" 2>&1; then
        echo "harness: cargo build failed; see ${LOG_DIR}/build.log" >&2
        exit 1
    fi
fi

REPO_A_DIR="${OH_STAGE_DIR}/repos/repo-a.git"
REPO_B_DIR="${OH_STAGE_DIR}/repos/repo-b.git"

seed_bare_repo() {
    local bare_dir="$1"
    local marker_name="$2"
    local work_dir="${bare_dir}-work"
    rm -rf "${bare_dir}" "${work_dir}"
    git init -q --bare "${bare_dir}"
    git init -q -b main "${work_dir}"
    git -c user.email=harness@example.com -c user.name=harness \
        commit -q --allow-empty -m "init"
    printf 'LOC-31 E2E marker for %s\n' "${marker_name}" > "${work_dir}/${marker_name}"
    git -C "${work_dir}" -c user.email=harness@example.com -c user.name=harness \
        add "${marker_name}"
    git -C "${work_dir}" -c user.email=harness@example.com -c user.name=harness \
        commit -q -m "add marker"
    git -C "${work_dir}" remote add origin "${bare_dir}"
    git -C "${work_dir}" push -q -u origin main
    rm -rf "${work_dir}"
}

seed_bare_repo "${REPO_A_DIR}" "E2E_REPO_A_MARKER.txt"
seed_bare_repo "${REPO_B_DIR}" "E2E_REPO_B_MARKER.txt"

# Temp repo copy whose project-set points at the seeded bare remotes.
TEMP_REPO="${OH_STAGE_DIR}/repo"
mkdir -p "${TEMP_REPO}"
# Exclude WORKFLOW.md because the project-set.yaml in the temp copy
# becomes the source of truth; we write a clean stub below so the
# orchestrator has a workflow file to load. Keep memory.yaml so the
# memory auto-capture loader doesn't trip, but skip the binary
# memory.duckdb file (it would race with the orchestrator's open).
tar -cf - --exclude=target --exclude=.git --exclude=WORKFLOW.md \
    --exclude=.opensymphony/openhands \
    --exclude=.opensymphony/memory/memory.duckdb \
    --exclude=.opensymphony/memory/sessions \
    --exclude=.opensymphony/memory/indices \
    --exclude=.opensymphony/generated \
    . | (cd "${TEMP_REPO}" && tar -xf -)
# Replace the legacy WORKFLOW.md with a minimal stub. When
# project-set mode is active, WORKFLOW.md must not define any
# tracker/polling/agent fields - those live in
# `.opensymphony/project-set.yaml`. An empty front-matter is the
# cleanest stub that satisfies the loader.
cat > "${TEMP_REPO}/WORKFLOW.md" <<WFL
---
# Project-set owns tracker / polling / agent; this stub only carries
# the workspace root + hooks + openhands settings that the workflow
# model still requires (project-set does not own these fields).
workspace:
  root: ${WORKSPACE_ROOT}
hooks:
  after_create: "opensymphony workspace clone"
  timeout_ms: 0
agent:
  max_turns: "1"
  stall_timeout_ms: 0
openhands:
  transport:
    # The orchestrator's local-server supervisor refuses to launch
    # a second OpenHands server on a port that is already bound and
    # ready. When the operator already has a long-running server at
    # the default ``127.0.0.1:8000`` (e.g. from the install step),
    # point the orchestrator at an unused port so the supervisor
    # launches a fresh, isolated server for this run. The
    # orchestrator and agent both talk to the same port, so the
    # marker-file + repo-key checks succeed end-to-end without
    # touching the host's long-running server.
    base_url: http://127.0.0.1:${OH_DISPATCH_PORT}
  local_server:
    # Force the supervisor to launch (rather than only probe) so the
    # run is self-contained. The supervisor refuses to launch on a
    # port that is already bound and ready, which is why the
    # transport ``base_url`` above points at a free port.
    enabled: true
---

# LOC-31 disposable multi-repo workflow stub

Per-run E2E workflow for LOC-31. All tracker / polling / agent
settings live in `.opensymphony/project-set.yaml`.
WFL
mkdir -p "${TEMP_REPO}/.opensymphony"
cat > "${TEMP_REPO}/.opensymphony/project-set.yaml" <<YAML
schema_version: 1

project_set:
  slug: loc31-multirepo-fixture
  name: LOC-31 Multi-Repo Fixture

  linear:
    endpoint: https://api.linear.app/graphql
    project_slug: ${TEST_LINEAR_PROJECT_SLUG}
    api_key_env: LINEAR_API_KEY
    # ``Backlog`` is included so the orchestrator picks up the disposable
    # test issues the converter publishes (Linear's default initial state).
    # The repo-resolver still skips parent issues at the dispatch gate.
    active_states: [Backlog, Todo, In Progress, "Human Review", Merging, Rework]
    terminal_states: [Done, Closed, Cancelled, Canceled, Duplicate]

  polling:
    interval_ms: 5000

  agent:
    max_concurrent_agents: 4

  projects:
    - slug: multi-repo-fixture
      name: LOC-31 Multi-Repo Fixture
      repos:
        - slug: repo-a
          url: ${REPO_A_DIR}
          default_branch: main
        - slug: repo-b
          url: ${REPO_B_DIR}
          default_branch: main
YAML

# Write a minimal config so ``opensymphony run`` picks up the
# project-set; the local-dev example does the same.
cat > "${TEMP_REPO}/config.yaml" <<'YAML'
control_plane:
  bind: 127.0.0.1:2468

openhands:
  tool_dir: ~/.opensymphony/openhands-server

memory:
  auto_capture: true
  auto_archive: false
YAML

# Launch the orchestrator against the disposable Linear project and
# capture stdout/stderr to per-stage logs. The orchestrator is
# expected to poll the project, dispatch the repo leaves, and clone
# the corresponding bare remotes into per-issue workspaces.
export LINEAR_API_KEY
export LLM_MODEL
export LLM_API_KEY
export LLM_BASE_URL="${LLM_BASE_URL:-https://openrouter.ai/api/v1}"
export OPENSYMPHONY_OPENHANDS_MODEL="${LLM_MODEL}"
export OPENSYMPHONY_OPENHANDS_API_KEY="${LLM_API_KEY}"
export OPENAI_API_KEY="${LLM_API_KEY}"
export OPENSYMPHONY_LIVE_OPENHANDS=1

RUN_LOG="${OH_STAGE_DIR}/run-logs/opensymphony-run.log"
RUN_PIDFILE="${OH_STAGE_DIR}/run-logs/opensymphony-run.pid"
echo "    launching: ${BINARY} run --config ${TEMP_REPO}/config.yaml"
(
    cd "${TEMP_REPO}"
    exec "${BINARY}" run --config "${TEMP_REPO}/config.yaml"
) >"${RUN_LOG}" 2>&1 &
RUN_PID=$!
echo "${RUN_PID}" > "${RUN_PIDFILE}"

# Poll the workspace root for the marker files / repo-key evidence.
# The OpenHands stage runs in the background; we give it a bounded
# window so this script can produce a deterministic pass/fail
# signal. The default is 5 minutes, overridable for CI.
OH_DISPATCH_TIMEOUT="${OSYM_E2E_OPENHANDS_DISPATCH_TIMEOUT:-300}"
OH_DISPATCH_DEADLINE=$((SECONDS + OH_DISPATCH_TIMEOUT))

verify_marker() {
    local issue_key="$1"
    local marker_name="$2"
    local expect_repo_key="$3"
    local ws="${OH_STAGE_DIR}/workspaces/${issue_key}"

    # Wait briefly for the workspace to be created.
    while (( SECONDS < OH_DISPATCH_DEADLINE )); do
        if [[ -d "${ws}" ]] && [[ -f "${ws}/${marker_name}" ]]; then
            echo "    OK: ${ws} contains ${marker_name}"
            # Hook evidence: ``opensymphony workspace clone`` records
            # ``workspace clone: ok key=<repo-key> url=<url> ...`` on
            # stderr; the workspace manager captures that stderr and
            # serializes it into ``run.json`` under ``hooks[*].stderr``.
            # We probe the JSON for ``key=<expect_repo_key>`` rather
            # than reaching into ``.opensymphony/logs/`` (which is just
            # the workspace-manager's placeholder dir; hook stdout/stderr
            # are not streamed there).
            local run_json="${ws}/.opensymphony/run.json"
            if [[ -f "${run_json}" ]] \
                && grep -q "\"key=${expect_repo_key}\"" "${run_json}"; then
                echo "    OK: hook evidence records key=${expect_repo_key}"
                return 0
            fi
        fi
        sleep 2
    done
    echo "    FAIL: ${ws} missing ${marker_name} or hook evidence" >&2
    return 1
}

verify_parent_not_cloned() {
    local issue_key="$1"
    local ws="${OH_STAGE_DIR}/workspaces/${issue_key}"
    if [[ ! -d "${ws}" ]]; then
        echo "    OK: parent workspace ${ws} does not exist (expected)"
        return 0
    fi
    echo "    FAIL: parent workspace ${ws} was created (should not be)" >&2
    return 1
}

PARENT_KEY="$(python3 -c "
import yaml
with open('${PUBLISH_FILE}', encoding='utf-8') as f:
    data = yaml.safe_load(f) or {}
tasks = data.get('tasks', {}) if isinstance(data, dict) else {}
print(tasks.get('TASK-LOC31-PARENT', {}).get('issue', '') or '')
")"
LEAF_A_KEY="$(python3 -c "
import yaml
with open('${PUBLISH_FILE}', encoding='utf-8') as f:
    data = yaml.safe_load(f) or {}
tasks = data.get('tasks', {}) if isinstance(data, dict) else {}
print(tasks.get('TASK-LOC31-LEAF-A', {}).get('issue', '') or '')
")"
LEAF_B_KEY="$(python3 -c "
import yaml
with open('${PUBLISH_FILE}', encoding='utf-8') as f:
    data = yaml.safe_load(f) or {}
tasks = data.get('tasks', {}) if isinstance(data, dict) else {}
print(tasks.get('TASK-LOC31-LEAF-B', {}).get('issue', '') or '')
")"

echo
echo "==> verifying OpenHands dispatch evidence"
OH_DISPATCH_FAILED=0
verify_parent_not_cloned "${PARENT_KEY}" || OH_DISPATCH_FAILED=1
verify_marker "${LEAF_A_KEY}" "E2E_REPO_A_MARKER.txt" "repo-a" || OH_DISPATCH_FAILED=1
verify_marker "${LEAF_B_KEY}" "E2E_REPO_B_MARKER.txt" "repo-b" || OH_DISPATCH_FAILED=1

# Always shut the orchestrator down once verification finishes so the
# temp artifact root is clean. Bounded graceful-exit window followed by
# a SIGKILL fallback so the script cannot hang on a stuck orchestrator
# (mirrors the preflight cleanup at lines ~146-157).
if kill -0 "${RUN_PID}" 2>/dev/null; then
    kill "${RUN_PID}" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        if ! kill -0 "${RUN_PID}" 2>/dev/null; then
            break
        fi
        sleep 0.5
    done
    if kill -0 "${RUN_PID}" 2>/dev/null; then
        echo "    orchestrator still alive after SIGTERM; sending SIGKILL"
        kill -9 "${RUN_PID}" 2>/dev/null || true
    fi
    wait "${RUN_PID}" 2>/dev/null || true
fi

if [[ "${OH_DISPATCH_FAILED}" == "1" ]]; then
    echo "harness: live OpenHands dispatch stage failed; see ${OH_STAGE_DIR}/run-logs/" >&2
    exit 1
fi

echo
echo "live multi-repo e2e harness: OK"
echo "evidence root: ${TMP_ROOT}"
echo "log directory: ${LOG_DIR}"
exit 0

