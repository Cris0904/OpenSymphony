# Configuration

This document covers target-repo bootstrap, generated files, and the runtime
configuration that `opensymphony run` expects.

## Bootstrap

Use `opensymphony init` from the target repository root:

```bash
cd /path/to/target-repo
opensymphony init
```

`opensymphony init` is the primary setup path for existing repositories. It:

- fetches the current starter files from the template repo's raw GitHub URLs
- copies missing files into the target repo
- leaves an existing `AGENTS.md` untouched and writes starter guidance to
  `AGENTS-example.md` during first-time setup
- prompts before overwriting other conflicting files
- writes the static `opensymphony workspace clone` hook into `WORKFLOW.md`'s
  `after_create` (no hardcoded URL); the runtime injects the resolved
  `RepoRef` via env vars at clone time
- registers the onboarded repo into the project-set inventory under
  `<cwd>/.opensymphony/project-set.yaml` (slug → `{ url, default_branch }`),
  using the `git remote` URL when one is confidently detected
- writes the Linear project slug/key into
  `project_set.linear.project_slug` (not `WORKFLOW.md`); `api_key_env` is
  env-backed and stays out of the serialized file
- strips project-set-owned global fields (`tracker.*`, `polling.interval_ms`,
  `agent.max_concurrent_agents`) from the generated `WORKFLOW.md` so it is
  already valid under strict project-set mode
- creates or updates `.gitignore` so local OpenSymphony runtime state stays
  untracked, while keeping `.opensymphony/project-set.yaml` versioned
- can optionally scaffold OpenHands AI PR review
- can configure the GitHub Actions variables, label, and optional review secret
  automatically when `gh` is installed and can access the repository
- prompts whether to commit and push the generated OpenSymphony files so shared
  skills and, when selected, AI PR Review setup are present in the remote
  repository before story work starts

For repositories that are already initialized, `opensymphony update` is the
maintenance path for template-owned skills:

```bash
cd /path/to/target-repo
opensymphony update
```

The command first checks whether the installed CLI is older than the newest
published `opensymphony` release and only runs `cargo install opensymphony`
when it actually needs to. If the current directory already looks like an
OpenSymphony target repo because it has both `WORKFLOW.md` and `config.yaml`,
the command then refreshes changed or new files under `.agents/skills/`.

### Migrating a legacy single-repo config (LOC-20)

Repositories bootstrapped before the strict project-set runtime boundary carry
project-set-owned global fields (`tracker.*`, `polling.interval_ms`,
`agent.max_concurrent_agents`) directly in `WORKFLOW.md` and do not have a
`.opensymphony/project-set.yaml`. `opensymphony update` detects that legacy
shape and, on every invocation inside such a target repo, performs an atomic
migration:

- generates or upserts `.opensymphony/project-set.yaml` with all required
  `project_set.*` fields (slug, Linear scope, polling, total concurrency, and
  one repo inventory entry);
- rewrites `WORKFLOW.md` to strip the migrated global fields, preserving the
  repo-local workspace, hooks, agent settings, OpenHands settings, and the
  byte-identical prompt body;
- preserves `config.yaml` and the `.gitignore` runtime policy, keeping
  `.opensymphony/project-set.yaml` versioned.

The migration is atomic: any unsafe auth (`tracker.api_key` that is neither
omitted nor an env-var reference), missing or ambiguous git remote, or
existing project-set conflict aborts the migration before any file is written.
Idempotency is guaranteed — re-running `opensymphony update` produces no
duplicate repo inventory entries and no churn in already-migrated files.

Two flags control the migration step inside `opensymphony update`:

- `--migrate-only` runs the migration step alone and exits. It skips the
  OpenSymphony self-update, the skill refresh, and the project memory init
  steps. Use this to recover from a partially applied migration without
  re-running the rest of the update flow.
- `--skip-migration` opts the target repo out of the migration step entirely.
  The rest of `update` (self-update check, skill refresh, memory init) still
  runs. Passing `--migrate-only` and `--skip-migration` together is a hard
  error.

The template repository is still the upstream source of those starter assets,
but it is an implementation detail of `opensymphony init`, not a required
manual setup step:

- [kumanday/OpenSymphony-template](https://github.com/kumanday/OpenSymphony-template)
- [Raw template base](https://raw.githubusercontent.com/kumanday/OpenSymphony-template/refs/heads/main/WORKFLOW.md)

## Files Added By `init`

Core bootstrap payload:

- `WORKFLOW.md` (with the static `opensymphony workspace clone` `after_create`
  hook and without project-set-owned global fields)
- `AGENTS.md`
- `AGENTS-example.md` when `AGENTS.md` already existed before first-time setup
- `config.yaml`
- `.opensymphony/project-set.yaml` created when missing, or upserted with the
  onboarded repo's inventory entry
- `.gitignore` created or updated to ignore OpenSymphony runtime state while
  keeping `.opensymphony/project-set.yaml` versioned
- `.agents/skills/` copied recursively, including skill-local `references/`, `scripts/`, and similar helper files
- `.agents/skills/linear/references/`
- `.github/CODEOWNERS`
- `.github/pull_request_template.md`

## Refreshing Template Skills

`opensymphony update` only refreshes template-managed files under
`.agents/skills/`.

It does not:

- rerun the interactive `init` prompts
- modify `WORKFLOW.md`
- merge or rewrite `AGENTS.md`
- create `AGENTS-example.md` after `config.yaml` exists
- copy `.github/*` bootstrap files
- delete repo-local extra skills that are not in the template tree

Optional AI PR review scaffolding:

- `.github/workflows/ai-pr-review.yml`
- `.agents/skills/custom-codereview-guide.md`

## Labels

If you enable AI PR review and `gh` is available with repository access,
`opensymphony init` can create the `review-this` label for you. If automation is
skipped, create it once per repository:

```bash
gh label create "review-this" --description "Trigger AI PR review" --color "d73a4a" --force
```

## Review The Generated Workflow

After `init`, review `WORKFLOW.md` and `config.yaml`.

If you accept the final commit/push prompt, `init` stages only the files it
created or updated, commits them as `chore: bootstrap OpenSymphony`, and pushes
`HEAD` to the detected git remote. If the repository already has staged changes
or no single remote can be detected, `init` leaves git alone and prints a
reminder to commit and push manually.

Important fields:

| Field | Description | Env Var | Example |
|-------|-------------|---------|---------|
| `tracker.project_slug` | Linear `Project.slugId` from the project URL | - | `my-project-5250e49b61f4` |
| `workspace.root` | Where to store per-issue workspaces | - | `~/.opensymphony/workspaces` |
| `openhands.conversation.agent.llm.model` | LLM model to use | `LLM_MODEL` | `openai/accounts/fireworks/models/glm-5p1` |

For Linear trackers, `tracker.project_slug` should store the project's
`slugId`, not a `team/project` path.

## Environment Variables

OpenSymphony uses standard OpenHands environment variable names.

Fireworks example via the OpenAI-compatible provider adapter:

```bash
export LLM_MODEL="openai/accounts/fireworks/models/glm-5p1"
export LLM_API_KEY="fw-..."
export LLM_BASE_URL="https://api.fireworks.ai/inference/v1"
```

The workflow supports `${VAR}` syntax for environment variable substitution in
the front matter:

```yaml
openhands:
  conversation:
    agent:
      llm:
        model: ${LLM_MODEL}
```

## Timeout Environment Variables

A handful of CLI helpers shell out to long-running commands. Each one is
bounded by a process-local environment variable so air-gapped CI, slow
proxies, and unresponsive remotes cannot hang the CLI indefinitely.

| Variable | Default | Scope | Notes |
| --- | --- | --- | --- |
| `OPENHANDS_GIT_REMOTE_SHOW_TIMEOUT_MS` | `5000` | `opensymphony init` / `update` default-branch detection (LOC-27) | Bounded timeout for the `git remote show <remote>` probe used by `detect_git_default_branch`. When the timeout elapses the helper kills the child, emits a `tracing::warn!`, and falls through to a `default_branch`-less entry in the project-set inventory. Accepts any positive integer in milliseconds. `0`, negative numbers, and unparseable strings fall back to the default. |
| `OPENSYMPHONY_TEMPLATE_FETCH_TIMEOUT_MS` | `30000` | `opensymphony init` template asset fetch | Bounded timeout for the per-asset fetch used to materialize the OpenSymphony template. Accepts any positive integer in milliseconds. |

The default-branch timeout is intentionally short (5s) because
`git remote show <remote>` makes a synchronous network round-trip to the
remote server, and the only useful response the CLI needs is the
`HEAD branch:` line. If your remote is slow but reachable, raise the
timeout for a single run:

```bash
OPENHANDS_GIT_REMOTE_SHOW_TIMEOUT_MS=15000 opensymphony init
```

## Conversation Condensation

Optional conversation condensation is enabled by default per workflow to reduce
long-history context pressure before the agent-server hits the model window:

```yaml
openhands:
  conversation:
    agent:
      condenser:
        max_size: 240
        keep_first: 2
```

OpenSymphony forwards an OpenHands `LLMSummarizingCondenser` that reuses the
conversation agent's LLM settings. The condenser is enabled by default with
`max_size: 240` and `keep_first: 2`. To disable it, set `enabled: false`.

## Runtime Config

`opensymphony init` also copies a starter `config.yaml` next to the target
repository `WORKFLOW.md`.

Minimal local-supervised example:

```yaml
control_plane:
  bind: 127.0.0.1:2468

openhands:
  tool_dir: ~/.opensymphony/openhands-server

memory:
  auto_capture: true
  auto_archive: false
```

The bind address is the single local HTTP surface for both the gateway API used
by the web/desktop clients (`/api/v1/capabilities`,
`/api/v1/dashboard/snapshot`, and related `/api/v1/*` routes) and the
control-plane compatibility routes used by the TUI (`/healthz`,
`/api/v1/snapshot`, and `/api/v1/control/events`).

Provision that app-managed directory with:

```bash
opensymphony install openhands
```

For managed local OpenHands, OpenSymphony derives a repository-scoped
conversation store from `openhands.tool_dir` and the target repo path:

```text
<tool_dir>/workspace/conversations/repos/<repo-key>/
  active/
  archived/
```

`opensymphony run` first moves known terminal issue conversations from existing
workspace manifests into `archived/`, then prepares `active/` from current
Linear candidate issue manifests before launching the managed server with
`OH_CONVERSATIONS_PATH` pointing at `active/`. The terminal-workspace sweep is a
temporary compatibility shim for older flat stores. This keeps completed or
manually archived issue history out of normal server startup while preserving it
for `opensymphony debug`.

When your workflow points at an external OpenHands agent-server with
`openhands.transport.session_api_key_env`, `config.yaml` can omit
`openhands.tool_dir`.

Use [examples/target-repo/config.yaml](../examples/target-repo/config.yaml) as
the starting template if you want to inspect the checked-in example.

[examples/configs/local-dev.yaml](../examples/configs/local-dev.yaml) is a
developer-facing doctor fixture for this repository. It is not the runtime
config that `opensymphony run` looks for in a target repo.

## Global Project Set (`.opensymphony/project-set.yaml`)

When the orchestrator is booting from outside a single target repo — that is,
when a single `opensymphony run` instance serves multiple repositories — it
looks up the **global** project-set config in addition to per-repo
`config.yaml` + `WORKFLOW.md`.

Phase-1 ships one file at a fixed path:

```text
<config_root>/.opensymphony/project-set.yaml
```

`config_root` is the directory the runtime `config.yaml` lives in (typically
`--config`'s parent). `.opensymphony/project-set.yaml` is **versioned** —
it sits next to other config files you commit and is allow-listed by the
generated `.gitignore` (only runtime artifacts under `.opensymphony/` are
ignored).

### When the file is absent

The file is optional. If `.opensymphony/project-set.yaml` does not exist,
`opensymphony run` falls back to today's legacy single-repo flow unchanged
and `opensymphony doctor` reports a `[SKIP] project-set` check.

### Schema (Phase 1)

```yaml
schema_version: 1

project_set:
  slug: opensymphony-updates
  name: OpenSymphony Updates

  linear:
    endpoint: https://api.linear.app/graphql
    project_slug: opensymphony-bootstrap-e7b957855cb7
    api_key_env: LINEAR_API_KEY
    active_states: [Todo, In Progress, Human Review, Merging, Rework]
    terminal_states: [Done, Closed, Cancelled, Canceled, Duplicate]

  polling:
    interval_ms: 5000

  agent:
    max_concurrent_agents: 4

  projects:
    - slug: opensymphony
      name: OpenSymphony
      repos:
        - slug: opensymphony
          url: git@github.com:kumanday/OpenSymphony.git
          default_branch: main
          path: ../OpenSymphony
```

Required fields:

- `schema_version` — must equal `1` for Phase 1; rejected otherwise.
- `project_set.slug` — globally-unique project-set identifier (used in logs
  and inventory keys).
- `project_set.linear.project_slug` — the single Linear project this
  orchestrator polls in Phase 1.
- `project_set.linear.active_states` / `terminal_states` — the Linear
  state set that drives dispatch.
- `project_set.projects[].repos[].slug` and `url` — the inventory facts.
  Duplicate repo slugs across the whole project set are rejected.

### Repo slugs vs `RepoRef.key`

`project_set.projects[].repos[].slug` is a **team-local repo identifier** —
it is what the rest of the orchestrator uses as `RepoRef.key` to look the
repo up in the project-set inventory. The orchestrator only ever sees this
slug; it does not need to parse `url` for routing.

The slug is intentionally NOT required to be a GitHub `org/repo` path. Bare
names like `opensymphony` are fine and are what the resolver stores in
`RepoRef.key`. `url` and `default_branch` carry the clone-source facts;
`path` (optional) carries local boot metadata only and is intentionally not
part of `RepoRef`.

### Repo slugs and the `repo:<slug>` Linear label

The reserved `repo:<slug>` Linear label namespace is keyed off this same
project-set repo slug. `repo:<slug>` MUST map to a `project_set.projects[].repos[].slug`
value (i.e. `RepoRef.key`) **exactly**, character-for-character. The
`create-implementation-plan` and `convert-tasks-to-linear` skills, the
`repo_for_issue` resolver, and the inventory validation in the converter all
assume exact match:

- Repo slugs are NOT lowercased, slugified, or otherwise coerced. Bare names
  like `opensymphony` are stored and compared verbatim.
- Area slugs, by contrast, ARE lowercased and slugified (see the
  `area_slug` helper in `convert_tasks_to_linear.py`); that normalization
  does NOT apply to `repo:` labels.
- Treat `repo:` as a reserved Linear label namespace that maps to the
  inventory slug exactly. See `docs/memory.md` for the related Memory-doc
  boundary and `AGENTS.md` for the canonical reserved-namespace note.

### Memory `project` vs `projectSet` and `execution_repo_key` vs `execution_repo`

The worker memory handoff exposes two distinct project scopes and two
distinct repo scopes. They are NOT required to be the same value:

- `OPENSYMPHONY_MEMORY_PROJECT` / MCP `project` is the **tracker project
  slug** (`tracker.project_slug`). It identifies the Linear project the
  current run is filed under.
- `OPENSYMPHONY_MEMORY_PROJECT_SET` / MCP `projectSet` is the
  **project-set slug** (`project_set.slug` / `project_set.config.slug`).
  It identifies the project-set inventory that owns the repo topology.
  When the project-set slug is unknown, this env var is intentionally
  unset instead of silently mirroring the tracker project.
- `OPENSYMPHONY_MEMORY_EXECUTION_REPO` / MCP `repo` is the run-level
  **target repo path** (often `config.repo_root` or a child workspace
  path). It is kept only as a warning-backed transitional fallback for the
  duration of the LOC-26 migration.
- `OPENSYMPHONY_MEMORY_EXECUTION_REPO_KEY` / MCP `executionRepoKey` is
  the **issue's resolved repo key** (`NormalizedIssue.execution_repo_ref.key`).
  It is the preferred value for repo-scoped Memory context and is matched
  against `RepositoryFacet.key` on the Memory side.

New callers should:

- Send `project` and `projectSet` independently when both are known. Do
  NOT collapse them into a single field.
- Send `executionRepoKey` from the issue's resolved `RepoRef.key` when
  available, and rely on `repo` only as a path-based fallback.

See [memory.md](memory.md) for the related repo-facet and `--repo` vs
`--paths` boundary, and the
[LOC-26](https://linear.app/localgputokenscrazy/issue/LOC-26/memory-repository-facet-and-repo-scoped-context)
slice for the original task.

### `LINEAR_API_KEY` fallback

`project_set.linear.api_key_env` is the env-var name that holds the Linear
API token. The default is `LINEAR_API_KEY`; if the field is omitted the
resolver still consults `LINEAR_API_KEY` directly. A missing or empty
`LINEAR_API_KEY` (or the configured override) fails the doctor
`[FAIL] project-set` check with `MissingEnvironmentVariable`.

### What it owns vs `config.yaml` / `WORKFLOW.md`

| Field | Lives in |
|-------|----------|
| `project_set.slug`, `project_set.projects[].repos[]` (inventory) | `.opensymphony/project-set.yaml` |
| `project_set.linear.*`, `project_set.polling.*`, `project_set.agent.max_concurrent_agents` | `.opensymphony/project-set.yaml` |
| `control_plane.bind`, `openhands.tool_dir`, `memory.*` | per-repo `config.yaml` |
| Per-repo workflow, prompt template, agent/model contract | per-repo `WORKFLOW.md` |

See [examples/project-set.yaml](../examples/project-set.yaml) for a
ready-to-edit template.

### Strict project-set boundary (LOC-18)

When `.opensymphony/project-set.yaml` is present, the runtime treats the
project-set as the **only** source of truth for the moved global fields.
`WORKFLOW.md` is allowed to omit them in project-set mode; if it does not,
the runtime reports them as **stale moved config** and the orchestrator
hard-fails.

Fields that move from `WORKFLOW.md` into `.opensymphony/project-set.yaml`
when project-set mode is active:

| WORKFLOW.md field (moved) | Project-set destination |
|---------------------------|--------------------------|
| `tracker.kind` | `project_set.linear` (kind implied: `linear`) |
| `tracker.endpoint` | `project_set.linear.endpoint` |
| `tracker.project_slug` | `project_set.linear.project_slug` |
| `tracker.api_key` | `project_set.linear.api_key_env` |
| `tracker.active_states` | `project_set.linear.active_states` |
| `tracker.terminal_states` | `project_set.linear.terminal_states` |
| `polling.interval_ms` | `project_set.polling.interval_ms` |
| `agent.max_concurrent_agents` | `project_set.agent.max_concurrent_agents` |

`tracker.api_key` is intentionally a **destination-only** field in the
diagnostic — the project-set stores the env-var name, not the secret
itself. The secret stays in the process environment.

In project-set mode, `opensymphony run` and `opensymphony doctor` enforce
the boundary as follows:

* `opensymphony run` hard-fails with a diagnostic that names the stale
  field(s) and the project-set destination(s). The stale values are
  **never** silently used as a fallback.
* `opensymphony doctor` reports a failing `[FAIL] project-set-boundary`
  check with the same diagnostic, and continues unrelated checks where
  practical so it remains useful as a migration guide.
* `opensymphony doctor` reports the active mode via `[PASS] mode: active
  mode: project-set` (or `legacy-single-repo` when the project-set file
  is absent).
* The legacy `linear.enabled: false` placeholder relaxation applies
  **only** in legacy single-repo mode. In project-set mode the real
  `project_set.linear.api_key_env` is always consulted; the Linear check
  cannot be silenced by `config.yaml` alone.

#### Evidence (captured from this repo)

The boundary was exercised end-to-end against this repo with a
pre-migration-shaped `WORKFLOW.md` (still defining every moved field)
and a project-set file copied from `examples/project-set.yaml`.

Stale-fields `opensymphony run` hard-fail (exit 1):

```text
$ opensymphony run --config /tmp/loc18_evidence/doctor.yaml
project-set mode is active but /tmp/loc18_evidence/repo/WORKFLOW.md still defines
project-set-owned fields: stale project-set-owned fields in WORKFLOW.md:
tracker.kind -> project_set.linear (kind implied: linear),
tracker.endpoint -> project_set.linear.endpoint,
tracker.project_slug -> project_set.linear.project_slug,
tracker.api_key -> project_set.linear.api_key_env,
tracker.active_states -> project_set.linear.active_states,
tracker.terminal_states -> project_set.linear.terminal_states,
polling.interval_ms -> project_set.polling.interval_ms,
agent.max_concurrent_agents -> project_set.agent.max_concurrent_agents;
move them to `.opensymphony/project-set.yaml` and remove them from WORKFLOW.md
```

Stale-fields `opensymphony doctor` (boundary check fails; unrelated checks
still report):

```text
[PASS] mode: active mode: project-set; project-set at .../project-set.yaml
       (slug `opensymphony-updates`, inventory of 1 repo(s)) owns global
       tracker/polling/total-concurrency; repo WORKFLOW.md owns repo-local fields
[PASS] workflow: resolved .../WORKFLOW.md -> workspace ...
[SKIP] workflow-prompt: skipped because project-set boundary is failing;
       fix stale moved fields in WORKFLOW.md
[WARN] workspace-root: ...
[PASS] bind-scope: OpenHands loopback target http://127.0.0.1:8000 ...
[SKIP] linear: skipped because project-set boundary is failing
[PASS] project-set: resolved .../project-set.yaml -> slug `opensymphony-updates`,
       linear project `opensymphony-bootstrap-e7b957855cb7`, 1 repos in inventory
[FAIL] project-set-boundary: WORKFLOW.md still defines project-set-owned fields
       in project-set mode: tracker.kind -> project_set.linear (kind implied:
       linear), tracker.endpoint -> project_set.linear.endpoint, ...,
       agent.max_concurrent_agents -> project_set.agent.max_concurrent_agents;
       remove them from WORKFLOW.md and set the matching values under
       `linear`/`polling`/`agent` in `.opensymphony/project-set.yaml`
       (migration owner: LOC-20)
```

Migrated `WORKFLOW.md` (omits every moved field) under the same project-set:

```text
[PASS] mode: active mode: project-set; ...
[PASS] workflow: resolved .../repo_migrated/WORKFLOW.md -> workspace ...
[PASS] workflow-prompt: rendered 28 characters from .../repo_migrated/WORKFLOW.md
[PASS] linear: project-set Linear auth ready: project
       opensymphony-bootstrap-e7b957855cb7, api_key_env LINEAR_API_KEY
       resolved (5 active / 5 terminal)
[PASS] project-set: resolved .../project-set.yaml -> ...
[PASS] project-set-boundary: no stale moved fields in
       .../project-set.yaml; project-set owns tracker/polling/total-concurrency
       and WORKFLOW.md owns repo-local fields
```

### Migrating an existing single-repo

`opensymphony run` never modifies user files. Migrating an existing repo
into project-set mode is owned by a separate migration command
([LOC-20](https://linear.app/localgputokenscrazy/issue/LOC-20/existing-repo-project-set-migration)),
which is responsible for:

1. Writing `.opensymphony/project-set.yaml` with the moved `tracker.*`,
   `polling.interval_ms`, and `agent.max_concurrent_agents` values from
   the existing `WORKFLOW.md`.
2. Removing those fields from `WORKFLOW.md` and leaving only the
   repo-local surface (workspace, hooks, per-repo agent settings,
   OpenHands/model contract, prompt template).

Until the migration runs, the existing single-repo flow remains
functional as the pre-migration legacy mode. The two shapes do **not**
intermix at runtime: the presence of `.opensymphony/project-set.yaml`
flips the mode to `project-set`, and `WORKFLOW.md` must be in the
migrated shape or the orchestrator will refuse to boot.

#### Evidence (LOC-18 PR #7)

The three runtime shapes were exercised against a freshly built binary
(`cargo build` -> `target/debug/opensymphony`) using three temporary
project trees (cleaned up after capture).

**1. Project-set mode with a migrated `WORKFLOW.md`** (omits every
project-set-owned field). `opensymphony doctor` reaches the strict
composer, reports the active mode, and the boundary check passes:

```
[PASS] mode: active mode: project-set; project-set at ./.opensymphony/project-set.yaml
        (slug `opensymphony-updates`, inventory of 1 repo(s)) owns global
        tracker/polling/total-concurrency; repo WORKFLOW.md owns repo-local fields
[PASS] workflow: resolved ./target-repo/WORKFLOW.md -> workspace ...,
        OpenHands http://127.0.0.1:8000, project opensymphony-bootstrap-e7b957855cb7,
        tracker auth resolved
[PASS] linear: project-set Linear auth ready: project
        opensymphony-bootstrap-e7b957855cb7, api_key_env LINEAR_API_KEY resolved
        (5 active / 5 terminal)
[PASS] project-set: resolved ./.opensymphony/project-set.yaml -> slug
        `opensymphony-updates`, linear project `opensymphony-bootstrap-e7b957855cb7`,
        1 repos in inventory
[PASS] project-set-boundary: no stale moved fields in
        ./.opensymphony/project-set.yaml; project-set owns
        tracker/polling/total-concurrency and WORKFLOW.md owns repo-local fields
```

`opensymphony run` reaches the strict composer without the stale-field
diagnostic; downstream failures are unrelated to LOC-18
(e.g. `memory.yaml` missing — fixed with `opensymphony memory init`).

**2. Project-set mode with a stale `WORKFLOW.md`** (still defines all
eight moved fields). `opensymphony run` hard-fails (exit code `1`) with
the per-field diagnostic listing every stale field and its project-set
destination:

```
$ opensymphony run --config ./doctor.yaml
project-set mode is active but .../WORKFLOW.md still defines project-set-owned
fields: stale project-set-owned fields in WORKFLOW.md:
  tracker.kind -> project_set.linear (kind implied: linear),
  tracker.endpoint -> project_set.linear.endpoint,
  tracker.project_slug -> project_set.linear.project_slug,
  tracker.api_key -> project_set.linear.api_key_env,
  tracker.active_states -> project_set.linear.active_states,
  tracker.terminal_states -> project_set.linear.terminal_states,
  polling.interval_ms -> project_set.polling.interval_ms,
  agent.max_concurrent_agents -> project_set.agent.max_concurrent_agents;
  move them to `.opensymphony/project-set.yaml` and remove them from WORKFLOW.md
$ echo $?
1
```

`opensymphony doctor` in the same stale project surfaces the full
picture — the unrelated `workspace-root`, `local-safety`,
`openhands-transport`, and `linear` checks all run against the
legacy-resolved workflow and the `[FAIL] project-set-boundary` check
carries the migration diagnostic:

```
[PASS] mode: active mode: project-set; ...
[PASS] workflow: resolved ... -> ... , tracker auth resolved
[SKIP] workflow-prompt: skipped because project-set boundary is failing;
        fix stale moved fields in WORKFLOW.md
[WARN] workspace-root: workspace root ... is usable but looks shared
[WARN] local-safety: trusted-machine mode only; ...
[PASS] bind-scope: OpenHands loopback target http://127.0.0.1:8000 ...
[PASS] linear: project-set Linear auth ready: project
        opensymphony-bootstrap-e7b957855cb7, api_key_env LINEAR_API_KEY resolved
        (2 active / 1 terminal)
[PASS] project-set: resolved ./.opensymphony/project-set.yaml -> ...
[FAIL] project-set-boundary: WORKFLOW.md still defines project-set-owned
        fields in project-set mode: tracker.kind -> project_set.linear
        (kind implied: linear), tracker.endpoint -> project_set.linear.endpoint,
        tracker.project_slug -> project_set.linear.project_slug,
        tracker.api_key -> project_set.linear.api_key_env,
        tracker.active_states -> project_set.linear.active_states,
        tracker.terminal_states -> project_set.linear.terminal_states,
        polling.interval_ms -> project_set.polling.interval_ms,
        agent.max_concurrent_agents -> project_set.agent.max_concurrent_agents;
        remove them from WORKFLOW.md and set the matching values under
        `linear`/`polling`/`agent` in `.opensymphony/project-set.yaml`
        (migration owner: LOC-20)
```

Note the `[PASS] linear` line — the `linear` check now runs against
the project-set's `api_key_env` even while the boundary is failing,
because the legacy-resolved workflow still carries the same tracker
fields the project-set owns.

**3. Legacy single-repo mode** (no `.opensymphony/project-set.yaml`).
The pre-migration `config.yaml` + `WORKFLOW.md` flow is preserved
exactly:

```
[PASS] mode: active mode: legacy-single-repo; pre-migration single-repo
        flow in use; create `.opensymphony/project-set.yaml` to opt into
        strict project-set mode
[PASS] workflow: resolved ./target-repo/WORKFLOW.md -> workspace ...,
        tracker auth resolved
[PASS] workflow-prompt: rendered 41 characters from ./target-repo/WORKFLOW.md
[SKIP] linear: Linear checks skipped because `linear.enabled` is false;
        workflow tracker project opensymphony-bootstrap-e7b957855cb7 still resolved
[SKIP] project-set: no ./.opensymphony/project-set.yaml present;
        legacy single-repo flow in use
[SKIP] project-set-boundary: project-set mode inactive; legacy single-repo
        flow has no boundary to enforce
```

`opensymphony run` reaches the legacy resolver without the stale-field
diagnostic, confirming the legacy path is unaffected.

These three transcripts are the operational definition of the LOC-18
runtime boundary. The integration tests in
`crates/opensymphony-cli/tests/{run,doctor}.rs` cover the same three
shapes with deterministic assertions so future regressions show up in
CI.

## Planning Workspace

The planning workspace is a dense, editable, review-oriented UI for the
hosted-client mode. It renders from the local planning workspace state and is
intended to feel like a task-creation tool with Linear as the publishing
target.

### Intentional MVP limitations

- The fixture planning session is intentionally reused across project switches
  in the local app shell. The workspace is not yet keyed per project, so
  switching projects keeps the same conversation, artifacts, and hierarchy
  until the gateway provides real planning sessions or a per-project session
  loader is implemented. This is documented behavior, not a bug.

## Memory Configuration

Project memory stores runtime state under `.opensymphony/memory` and can be
captured automatically by `opensymphony run`. Runtime automation is controlled
by `config.yaml`:

```yaml
memory:
  auto_capture: true
  auto_archive: false
```

`auto_capture` defaults to `true`. It captures terminal issue transitions
observed by the run loop. `auto_archive` defaults to `false`; when enabled, it
archives only after successful capture with no blocking warnings.
When archive succeeds and the repo uses the managed local OpenHands server,
OpenSymphony also moves the issue's persisted conversation from the repo-scoped
`active/` store to `archived/`.

Initialize the shared memory policy and learned ontology file with:

```bash
opensymphony memory init
```

This creates `.opensymphony/memory/memory.yaml` and updates `.gitignore` so only
that shared config is tracked. Capsules, markdown indexes, DuckDB, source
snapshots, and runtime logs remain local:

```text
.opensymphony/memory/
  memory.yaml
  issues/
  indexes/
  memory.duckdb
```

`memory.yaml` contains policy plus learned structure. `memory init` seeds stable
areas from existing top-level `docs/*.md` files when present; otherwise it
starts with an empty `areas` map and capture evolves it from Linear and PR
narrative evidence:

```yaml
memory_root: .opensymphony/memory
visibility: private
index_path: .opensymphony/memory/memory.duckdb
confidence_threshold: 75
markdown_indexes: true
docs:
  public_root: docs
  default_visibility: public
  deny_private_links: true
areas:
  openhands-runtime:
    title: OpenHands Runtime
    docs_target: docs/openhands-runtime.md
    visibility: public
    status: stable
    confidence: 85
    aliases:
      - OpenHands Runtime
    source_refs:
      docs:
        - docs/openhands-runtime.md
      linear_labels:
        - runtime
```

Private memory should stay out of source control. Commit
`.opensymphony/memory/memory.yaml` and generated public docs when appropriate;
do not commit issue capsules, markdown indexes, DuckDB, source snapshots, or
runtime state.

## OpenHands PR Review

If you opt into OpenHands PR review during `init`, the CLI will try to
configure the GitHub Actions variables, label, and optional review secret for
you when:

- `gh` is installed
- `gh` can access the target repository
- you approve the automation prompt

If any of those are missing, `init` falls back to a short checklist plus the
manual `gh` commands. The full verification and branch-protection guidance
lives in the OpenSymphony docs at
[ai-pr-review-human-setup.md](ai-pr-review-human-setup.md); `init` does not
copy that guide into the target repository.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-252 contributed: PR #10: Implement foundation workflow and scheduler contracts
- COE-253 contributed: PR #19: COE-253: OpenHands Runtime Adapter (merge `911b0b4`)
- COE-254 contributed: PR #6: COE-254: bootstrap tracker, workspace, and orchestration core
- COE-255 contributed: PR #4: COE-255: add control plane and FrankenTUI slice
- COE-256 contributed: PR #1: COE-257: tighten hosted deployment guidance
- COE-258 contributed: PR #83: Add memory init and mapped docs sync

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-252: Foundation and Contracts
- COE-253: OpenHands Runtime Adapter
- COE-254: Tracker, Workspaces, and Orchestration
- COE-255: Observability and FrankenTUI
- COE-256: Validation and Local Operations
- COE-258: Bootstrap workspace and crate boundaries
- COE-259: Workflow loader and typed config
- COE-260: Domain model and orchestrator state machine
- COE-261: Local agent-server supervisor
- COE-262: REST client and conversation contract
- COE-263: Workspace manager and lifecycle hooks
- COE-264: Linear read adapter and issue normalization
- COE-265: WebSocket event stream, reconciliation, and recovery
- COE-266: Issue session runner
- COE-267: Linear MCP write surface
- COE-268: Orchestrator scheduler, retries, and reconciliation
- COE-269: Control-plane API and snapshot store
- COE-270: Repository harness and generated context artifacts
- COE-271: FrankenTUI operator client
- COE-272: Fake OpenHands server and protocol contract suite
- COE-273: Live local end-to-end suite
- COE-274: CLI packaging, doctor, and local operations docs
- COE-277: Implement hierarchy-aware task selection
- COE-278: Doctor live probe resolves repo-local OpenHands launcher paths reliably
- COE-280: Support workflow-owned OpenHands auth, provider, and launcher overrides at runtime
- COE-281: Support path-bearing OpenHands base URLs and MCP config at runtime
- COE-282: Support workflow-owned OpenHands conversation reuse policy at runtime
- COE-284: Add orchestrator run command to CLI and make it installable
- COE-286: Abort active CLI worker tasks on graceful orchestrator shutdown
- COE-287: Add opensymphony debug command for conversational session debugging
- COE-288: Add context condenser support to prevent LLM context window overflow
- COE-293: OpenHands agent has no filesystem tools - only FinishTool and ThinkTool
- COE-294: Detect LLM config changes and rehydrate conversations with updated env vars
- COE-382: Add supply-chain and security audits to CI
- COE-383: Decompose oversized session and TUI modules into focused submodules
- COE-384: Expand error-path tests for Linear client and workspace hooks
- COE-385: Resolve runtime tracking TODO in OpenHands session runner
- COE-386: Wire cargo-llvm-cov coverage reporting and regression floor into CI
- COE-387: Audit tracing spans and diagnostics for secret leakage
- COE-394: Frontend Workspace And Shared Schemas
- COE-395: Planning Artifact Schema And Session Service
- COE-397: Gateway API Client, Transport Adapters, And Reducers
- COE-398: Tauri Shell And Security Capabilities
- COE-399: Linear Read Coverage And Task Graph Cache
- COE-400: OpenHands Event Normalization And Runtime Mirror
- COE-401: Web App Entry And Deployment Modes
- COE-402: App Shell, Dashboard, Task Graph, And Run Views
- COE-403: Terminal And Log Renderer Prototype
- COE-404: Desktop Connection Profiles And Daemon Management
- COE-405: Linear Milestone, Issue, And Sub-Issue Mutations
- COE-406: Repository, Linear, And Research Analysis
- COE-409: Desktop Settings, Keychain, And Native Actions
- COE-410: Desktop Local Stream Optimization
- COE-411: Task Graph Editor And Runtime Overlay UI
- COE-412: Runtime Timeline And Terminal/Log Association
- COE-413: Implementation Plan Generator Stage
- COE-414: Diff, Validation, Approval, And Run Action Views
- COE-415: Milestone, Issue, And Sub-Issue Compiler
- COE-416: Dependency Graph And Plan Checks
- COE-417: Planning Workspace UI
- COE-434: Long-running harness liveness and scheduler/runtime ownership contract
- COE-435: Long-running run observability fixtures and client-facing diagnostics
- COE-449: Desktop alpha recovery: replace stubs with functional app
- COE-452: DuckDB Prebuilt Developer Build Mode
- COE-453: Non-Interactive Init For Automation

## Source refs

- COE-252
- COE-253
- COE-254
- COE-255
- COE-256
- COE-258
- COE-259
- COE-260
- COE-261
- COE-262
- COE-263
- COE-264
- COE-265
- COE-266
- COE-267
- COE-268
- COE-269
- COE-270
- COE-271
- COE-272
- COE-273
- COE-274
- COE-277
- COE-278
- COE-280
- COE-281
- COE-282
- COE-284
- COE-286
- COE-287
- COE-288
- COE-293
- COE-294
- COE-382
- COE-383
- COE-384
- COE-385
- COE-386
- COE-387
- COE-394
- COE-395
- COE-397
- COE-398
- COE-399
- COE-400
- COE-401
- COE-402
- COE-403
- COE-404
- COE-405
- COE-406
- COE-409
- COE-410
- COE-411
- COE-412
- COE-413
- COE-414
- COE-415
- COE-416
- COE-417
- COE-434
- COE-435
- COE-449
- COE-452
- COE-453

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
