use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use tempfile::TempDir;
use tokio::{net::TcpListener, process::Command};

#[tokio::test]
async fn update_skips_reinstall_when_current_matches_latest_and_refreshes_skills() {
    let server = UpdateServer::start(env!("CARGO_PKG_VERSION")).await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");

    fs::write(repo.path().join("WORKFLOW.md"), "# workflow\n").expect("workflow should write");
    fs::write(
        repo.path().join("config.yaml"),
        "openhands:\n  tool_dir: ~/.opensymphony\n",
    )
    .expect("config should write");
    fs::create_dir_all(repo.path().join(".agents/skills/linear"))
        .expect("linear skill dir should exist");
    fs::write(
        repo.path().join(".agents/skills/linear/SKILL.md"),
        "# stale linear\n",
    )
    .expect("stale linear skill should write");
    fs::create_dir_all(repo.path().join(".agents/skills/commit"))
        .expect("commit skill dir should exist");
    fs::write(
        repo.path().join(".agents/skills/commit/SKILL.md"),
        "# commit\n",
    )
    .expect("commit skill should write");
    fs::create_dir_all(repo.path().join(".agents/skills/local-only"))
        .expect("local-only dir should exist");
    fs::write(
        repo.path().join(".agents/skills/local-only/SKILL.md"),
        "# keep me\n",
    )
    .expect("local skill should write");

    let output = run_update(repo.path(), &cargo_log, &server, &[]).await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert_eq!(
        cargo_invocation_count(&cargo_log),
        0,
        "cargo should not run when the installed version is current",
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".agents/skills/linear/SKILL.md"))
            .expect("linear skill should exist"),
        "# linear\n",
    );
    assert!(
        repo.path().join(".agents/skills/push/SKILL.md").is_file(),
        "new template-managed skills should be created",
    );
    assert!(
        !repo
            .path()
            .join(".agents/skills/opensymphony-memory/SKILL.md")
            .exists(),
        "memory skill should only be refreshed when the template repo provides it",
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".agents/skills/local-only/SKILL.md"))
            .expect("local-only skill should survive"),
        "# keep me\n",
    );
    let memory_config = fs::read_to_string(repo.path().join(".opensymphony/memory/memory.yaml"))
        .expect("update should initialize memory config in target repos");
    assert!(
        memory_config.contains("memory_root: .opensymphony/memory"),
        "memory config should contain the default memory root: {memory_config}",
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".gitignore")).expect(".gitignore should exist"),
        memory_gitignore_policy("")
    );
    assert!(
        !repo.path().join("AGENTS.md").exists(),
        "update should not create other bootstrap assets",
    );
    assert!(
        !repo.path().join(".github/CODEOWNERS").exists(),
        "update should not copy .github bootstrap files",
    );
    assert!(
        stdout.contains("skipping `cargo install opensymphony`"),
        "stdout should explain the skipped reinstall: {stdout}",
    );
    assert!(
        stdout.contains("Detected an OpenSymphony target repo"),
        "stdout should explain why skills were refreshed: {stdout}",
    );
    assert!(
        stdout.contains("Updated:") && stdout.contains("- .agents/skills/linear/SKILL.md"),
        "stdout should list updated skill files: {stdout}",
    );
    assert!(
        stdout.contains("Created:")
            && stdout.contains("- .agents/skills/push/SKILL.md")
            && !stdout.contains("- .agents/skills/opensymphony-memory/SKILL.md"),
        "stdout should list created skill files: {stdout}",
    );
    assert!(
        stdout.contains("Memory init summary:")
            && stdout.contains("- .opensymphony/memory/memory.yaml")
            && stdout.contains("- .gitignore"),
        "stdout should list memory initialization files: {stdout}",
    );
}

#[tokio::test]
async fn update_installs_when_latest_is_newer_and_skips_skill_refresh_outside_target_repo() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");

    let output = run_update(repo.path(), &cargo_log, &server, &[]).await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert_eq!(
        cargo_invocation_count(&cargo_log),
        1,
        "cargo install should run when a newer published version exists",
    );
    let cargo_log = fs::read_to_string(&cargo_log).expect("cargo log should exist");
    assert!(
        cargo_log.contains("ARGS=install opensymphony"),
        "cargo install should use the requested command: {cargo_log}",
    );
    assert!(
        stdout.contains("Skipped template skill refresh because this directory is missing `WORKFLOW.md` and `config.yaml`."),
        "stdout should explain why the skill refresh was skipped: {stdout}",
    );
}

async fn run_update(
    repo_root: &Path,
    cargo_log: &Path,
    server: &UpdateServer,
    extra_args: &[&str],
) -> std::process::Output {
    let fake_bin_dir = repo_root.join(".test-bin");
    fs::create_dir_all(&fake_bin_dir).expect("fake bin dir should exist");
    write_fake_cargo(fake_bin_dir.join("cargo"), cargo_log);

    Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("update")
        .args(extra_args)
        .current_dir(repo_root)
        .env("PATH", path_only(fake_bin_dir.as_path()))
        .env("OPENSYMPHONY_TEMPLATE_BASE_URL", server.base_url())
        .env(
            "OPENSYMPHONY_UPDATE_CRATE_METADATA_URL",
            server.crate_metadata_url(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .expect("update command should run")
}

struct UpdateServer {
    base_url: String,
    crate_metadata_url: String,
    task: tokio::task::JoinHandle<()>,
}

impl UpdateServer {
    async fn start(latest_version: &str) -> Self {
        let state = Arc::new(ServerState {
            latest_version: latest_version.to_string(),
            assets: template_assets(),
        });
        let app = Router::new()
            .fallback(get(update_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("update server should bind");
        let address = listener
            .local_addr()
            .expect("update server should have an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("update server should run");
        });

        Self {
            base_url: format!("http://{address}/"),
            crate_metadata_url: format!("http://{address}/__crate.json"),
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn crate_metadata_url(&self) -> &str {
        &self.crate_metadata_url
    }
}

impl Drop for UpdateServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct ServerState {
    latest_version: String,
    assets: BTreeMap<String, String>,
}

async fn update_handler(
    State(state): State<Arc<ServerState>>,
    uri: Uri,
    _request: Request,
) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "__crate.json" {
        return (
            StatusCode::OK,
            serde_json::json!({
                "crate": {
                    "max_version": state.latest_version,
                }
            })
            .to_string(),
        )
            .into_response();
    }

    if path == "__tree.json" {
        let tree = state
            .assets
            .keys()
            .map(|path| serde_json::json!({ "path": path, "type": "blob" }))
            .collect::<Vec<_>>();
        return (
            StatusCode::OK,
            serde_json::json!({ "tree": tree }).to_string(),
        )
            .into_response();
    }

    match state.assets.get(path) {
        Some(content) => (StatusCode::OK, content.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, format!("missing asset {path}")).into_response(),
    }
}

fn template_assets() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            ".agents/skills/commit/SKILL.md".to_string(),
            "# commit\n".to_string(),
        ),
        (
            ".agents/skills/linear/SKILL.md".to_string(),
            "# linear\n".to_string(),
        ),
        (
            ".agents/skills/push/SKILL.md".to_string(),
            "# push\n".to_string(),
        ),
        (
            ".agents/skills/linear/queries/viewer.graphql".to_string(),
            "query Viewer { viewer { id } }\n".to_string(),
        ),
    ])
}

fn cargo_invocation_count(log_path: &Path) -> usize {
    match fs::read_to_string(log_path) {
        Ok(contents) => contents
            .lines()
            .filter(|line| line.starts_with("ARGS="))
            .count(),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => 0,
        Err(source) => panic!("cargo log should be readable: {source}"),
    }
}

fn memory_gitignore_policy(prefix: &str) -> String {
    format!(
        "{prefix}.opensymphony*\n!.opensymphony/\n.opensymphony/*\n!.opensymphony/memory/\n.opensymphony/memory/*\n!.opensymphony/memory/memory.yaml\n!.opensymphony/project-set.yaml\n"
    )
}

fn path_only(path: &Path) -> OsString {
    // Prepend the fake bin dir so the test harness can shadow `cargo`, but
    // preserve the system PATH so `git` (used by migration) and `gh` still
    // resolve when the test runs outside of CI.
    let mut entries = vec![path.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&existing).collect::<Vec<_>>());
    }
    std::env::join_paths(entries).expect("path should join")
}

fn write_fake_cargo(path: PathBuf, log_path: &Path) {
    write_executable(
        path,
        &format!(
            "#!/bin/sh\nset -eu\nprintf 'PWD=%s\\n' \"$PWD\" >> \"{}\"\nprintf 'ARGS=%s\\n' \"$*\" >> \"{}\"\n",
            log_path.display(),
            log_path.display(),
        ),
    );
}

fn write_executable(path: PathBuf, contents: &str) {
    fs::write(&path, contents).expect("executable should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&path)
            .expect("executable metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("executable should be executable");
    }
}

// ---------------------------------------------------------------------------
// Integration tests for the existing-repo project-set migration (LOC-20).
//
// These tests exercise the binary `opensymphony update --migrate-only` path,
// which is the operator-facing seam. The migration unit tests live next to
// the implementation in `project_set_migration.rs`; here we verify the
// wiring through `update_repo::run_update`.
// ---------------------------------------------------------------------------

fn legacy_workflow_source() -> String {
    let mut workflow = String::from("---\n");
    workflow.push_str("tracker:\n");
    workflow.push_str("  kind: linear\n");
    workflow.push_str("  project_slug: opensymphony-bootstrap\n");
    workflow.push_str("  active_states:\n    - Todo\n");
    workflow.push_str("  terminal_states:\n    - Done\n");
    workflow.push_str("polling:\n  interval_ms: 5000\n");
    workflow.push_str("agent:\n  max_concurrent_agents: 8\n  max_turns: 40\n");
    workflow.push_str("workspace:\n  root: ~/.opensymphony/workspaces\n");
    workflow.push_str("hooks:\n  after_create: |\n    opensymphony workspace clone\n");
    workflow.push_str("---\n\nYou are working on a Linear ticket.\n");
    workflow
}

fn write_legacy_target_repo(repo: &Path) {
    fs::write(repo.join("WORKFLOW.md"), legacy_workflow_source()).expect("workflow write");
    fs::write(
        repo.join("config.yaml"),
        "openhands:\n  tool_dir: ~/.opensymphony\n",
    )
    .expect("config write");
    init_git_remote(repo);
}

fn init_git_remote(repo: &Path) {
    fs::create_dir_all(repo.join(".git")).expect("git dir");
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(repo)
        .output()
        .expect("git init");
    assert!(init.status.success(), "git init should succeed");
    let remote = std::process::Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://example.com/test-repo.git",
        ])
        .current_dir(repo)
        .output()
        .expect("git remote add");
    assert!(remote.status.success(), "git remote add should succeed");
}

#[tokio::test]
async fn migrate_only_creates_project_set_and_cleans_workflow() {
    let server = UpdateServer::start(env!("CARGO_PKG_VERSION")).await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    write_legacy_target_repo(repo.path());

    let output = run_update(repo.path(), &cargo_log, &server, &["--migrate-only"]).await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "migrate-only should succeed: stdout={stdout}, stderr={stderr}"
    );
    assert_eq!(
        cargo_invocation_count(&cargo_log),
        0,
        "migrate-only must skip `cargo install`"
    );
    assert!(
        repo.path().join(".opensymphony/project-set.yaml").is_file(),
        "project-set.yaml should be created"
    );
    let project_set_yaml = fs::read_to_string(repo.path().join(".opensymphony/project-set.yaml"))
        .expect("project-set should exist");
    assert!(
        project_set_yaml.contains("schema_version: 1"),
        "project-set.yaml should pin schema_version: {project_set_yaml}"
    );
    assert!(
        project_set_yaml.contains("slug: default-project-set"),
        "project-set.yaml should include the project-set slug: {project_set_yaml}"
    );
    assert!(
        project_set_yaml.contains("api_key_env: LINEAR_API_KEY"),
        "omitted auth should migrate to LINEAR_API_KEY: {project_set_yaml}"
    );
    assert!(
        project_set_yaml.contains("interval_ms: 5000"),
        "polling interval should migrate: {project_set_yaml}"
    );
    assert!(
        project_set_yaml.contains("max_concurrent_agents: 8"),
        "agent concurrency should migrate: {project_set_yaml}"
    );
    assert!(
        project_set_yaml.contains("url: https://example.com/test-repo.git"),
        "repo URL should be captured from git remote: {project_set_yaml}"
    );

    let workflow_after =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow exists");
    assert!(
        !workflow_after.contains("kind: linear"),
        "tracker.kind should be removed: {workflow_after}"
    );
    assert!(
        !workflow_after.contains("interval_ms"),
        "polling.interval_ms should be removed: {workflow_after}"
    );
    assert!(
        !workflow_after.contains("max_concurrent_agents: 8"),
        "agent.max_concurrent_agents should be removed: {workflow_after}"
    );
    assert!(
        workflow_after.contains("max_turns: 40"),
        "repo-local agent.max_turns should survive: {workflow_after}"
    );
    assert!(
        workflow_after.contains("workspace:") && workflow_after.contains("hooks:"),
        "workspace and hooks should survive: {workflow_after}"
    );
    assert!(
        workflow_after.contains("You are working on a Linear ticket.\n"),
        "prompt body should survive byte-identical"
    );

    assert!(
        stdout.contains("Migration: created") && stdout.contains(".opensymphony/project-set.yaml"),
        "stdout should report the created project-set file: {stdout}"
    );
    assert!(
        stdout.contains("Migration: rewrote") && stdout.contains("WORKFLOW.md"),
        "stdout should report the rewritten WORKFLOW.md: {stdout}"
    );
}

#[tokio::test]
async fn migrate_only_is_idempotent_on_repeat_invocation() {
    let server = UpdateServer::start(env!("CARGO_PKG_VERSION")).await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    write_legacy_target_repo(repo.path());

    let first = run_update(repo.path(), &cargo_log, &server, &["--migrate-only"]).await;
    assert!(
        first.status.success(),
        "first migrate-only should succeed: stdout={}, stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let project_set_after_first =
        fs::read_to_string(repo.path().join(".opensymphony/project-set.yaml"))
            .expect("project-set should exist after first run");
    let workflow_after_first =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow exists");

    let second = run_update(repo.path(), &cargo_log, &server, &["--migrate-only"]).await;
    assert!(
        second.status.success(),
        "second migrate-only should succeed: stdout={}, stderr={}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let stdout_second = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout_second.contains("already project-set mode"),
        "second invocation should report no-op: {stdout_second}"
    );
    let project_set_after_second =
        fs::read_to_string(repo.path().join(".opensymphony/project-set.yaml"))
            .expect("project-set should exist after second run");
    let workflow_after_second =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow exists");
    assert_eq!(
        project_set_after_first, project_set_after_second,
        "project-set.yaml must not churn between runs"
    );
    assert_eq!(
        workflow_after_first, workflow_after_second,
        "WORKFLOW.md must not churn between runs"
    );
}

#[tokio::test]
async fn migrate_only_rejects_literal_api_key_without_writing_files() {
    let server = UpdateServer::start(env!("CARGO_PKG_VERSION")).await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");

    let mut workflow = String::from("---\n");
    workflow.push_str("tracker:\n");
    workflow.push_str("  kind: linear\n");
    workflow.push_str("  api_key: lin_api_abcdef0123456789\n");
    workflow.push_str("workspace:\n  root: ~/.opensymphony/workspaces\n");
    workflow.push_str("---\n\nPrompt body.\n");
    fs::write(repo.path().join("WORKFLOW.md"), &workflow).expect("workflow write");
    fs::write(
        repo.path().join("config.yaml"),
        "openhands:\n  tool_dir: ~/.opensymphony\n",
    )
    .expect("config write");
    init_git_remote(repo.path());

    let output = run_update(repo.path(), &cargo_log, &server, &["--migrate-only"]).await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success(),
        "literal api_key should fail: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stderr.contains("literal token"),
        "stderr should explain the literal-token failure: {stderr}"
    );
    assert!(
        !repo.path().join(".opensymphony/project-set.yaml").exists(),
        "no project-set.yaml should be written when migration aborts"
    );
    let workflow_after =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow exists");
    assert_eq!(
        workflow_after, workflow,
        "WORKFLOW.md must be untouched when migration aborts"
    );
}

#[tokio::test]
async fn migrate_only_rejects_missing_remote_without_writing_files() {
    let server = UpdateServer::start(env!("CARGO_PKG_VERSION")).await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");

    fs::write(repo.path().join("WORKFLOW.md"), legacy_workflow_source()).expect("workflow write");
    fs::write(
        repo.path().join("config.yaml"),
        "openhands:\n  tool_dir: ~/.opensymphony\n",
    )
    .expect("config write");
    // No .git directory and no remote configured -> AmbiguousRemote.

    let output = run_update(repo.path(), &cargo_log, &server, &["--migrate-only"]).await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success(),
        "missing remote should fail: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stderr.contains("git remote"),
        "stderr should explain how to add a git remote: {stderr}"
    );
    assert!(
        !repo.path().join(".opensymphony/project-set.yaml").exists(),
        "no project-set.yaml should be written when migration aborts"
    );
}

#[tokio::test]
async fn update_rejects_migrate_only_combined_with_skip_migration() {
    // `--migrate-only` requests the migration step alone, while
    // `--skip-migration` suppresses it. Passing both is an operator
    // mistake that the CLI must surface explicitly instead of silently
    // picking one of the two behaviours (LOC-20 PR feedback #2).
    let server = UpdateServer::start(env!("CARGO_PKG_VERSION")).await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    write_legacy_target_repo(repo.path());

    let output = run_update(
        repo.path(),
        &cargo_log,
        &server,
        &["--migrate-only", "--skip-migration"],
    )
    .await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success(),
        "conflicting flags should fail: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stderr.contains("--migrate-only") && stderr.contains("--skip-migration"),
        "stderr should name both flags: {stderr}"
    );
    assert!(
        !repo.path().join(".opensymphony/project-set.yaml").exists(),
        "no project-set.yaml should be written when the flag combination is invalid"
    );
}

#[tokio::test]
async fn update_skip_migration_omits_project_set_and_preserves_legacy_workflow() {
    // `--skip-migration` is the explicit opt-out for already-migrated
    // repos (or any repo where the operator does not want the migration
    // step to run). It must (LOC-20):
    //
    // * not write `.opensymphony/project-set.yaml`;
    // * leave the legacy `WORKFLOW.md` untouched (no field rewrites);
    // * continue with the rest of the update flow (self-update check,
    //   skill refresh, memory init).
    let server = UpdateServer::start(env!("CARGO_PKG_VERSION")).await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    write_legacy_target_repo(repo.path());

    let workflow_before =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow exists");

    let output = run_update(repo.path(), &cargo_log, &server, &["--skip-migration"]).await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "skip-migration should still let the rest of `update` succeed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        !repo.path().join(".opensymphony/project-set.yaml").exists(),
        "skip-migration must not create the project-set file"
    );
    let workflow_after =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow exists");
    assert_eq!(
        workflow_before, workflow_after,
        "skip-migration must leave WORKFLOW.md byte-identical"
    );
    assert!(
        !stdout.contains("Migration: created") && !stdout.contains("Migration: rewrote"),
        "skip-migration must not run the migration step: {stdout}"
    );
}

#[tokio::test]
async fn update_migration_runs_even_when_cargo_self_update_fails() {
    // The migration must run BEFORE the self-update path so a broken
    // `cargo install` (or a simulated network failure) does not block the
    // local repo from being migrated to the strict project-set boundary.
    // This test simulates the failure by replacing the fake `cargo` with a
    // script that exits non-zero when `cargo install opensymphony` is
    // invoked. The legacy repo should still get migrated before the
    // self-update attempt, so the migration succeeds and the overall
    // command exits with the cargo-install failure status. We check both
    // observable outcomes: the project-set file is created and the
    // non-zero exit status reflects the cargo failure.
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    write_legacy_target_repo(repo.path());

    let fake_bin_dir = repo.path().join(".test-bin");
    fs::create_dir_all(&fake_bin_dir).expect("fake bin dir");
    // Replace `cargo` with a failing script for the self-update path.
    let cargo_path = fake_bin_dir.join("cargo");
    fs::write(
        &cargo_path,
        format!(
            "#!/bin/sh\nset +e\nprintf 'ARGS=%s\\n' \"$*\" >> \"{}\"\nif [ \"$1\" = \"install\" ] && [ \"$2\" = \"opensymphony\" ]; then\n  echo 'simulated network failure' >&2\n  exit 7\nfi\nexit 0\n",
            cargo_log.display()
        ),
    )
    .expect("fake cargo write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&cargo_path)
            .expect("fake cargo metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&cargo_path, permissions).expect("fake cargo chmod");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("update")
        .current_dir(repo.path())
        .env("PATH", path_only(fake_bin_dir.as_path()))
        .env("OPENSYMPHONY_TEMPLATE_BASE_URL", server.base_url())
        .env(
            "OPENSYMPHONY_UPDATE_CRATE_METADATA_URL",
            server.crate_metadata_url(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .expect("update command should run");

    // The migration must have run before the failing self-update step.
    assert!(
        repo.path().join(".opensymphony/project-set.yaml").is_file(),
        "migration must complete before the self-update path attempts"
    );
    let workflow_after =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow exists");
    assert!(
        !workflow_after.contains("kind: linear"),
        "stale tracker fields must be removed even when self-update fails"
    );
    assert!(
        !output.status.success(),
        "overall command should fail because cargo install exits non-zero: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
