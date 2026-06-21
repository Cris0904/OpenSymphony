use std::{process::Stdio, time::Duration};

use crate::opensymphony_testkit::FakeOpenHandsServer;
use axum::{Json, Router, routing::post};
use serde_json::json;
use tempfile::TempDir;
use tokio::{
    net::TcpListener,
    process::{Child, Command},
    task::JoinHandle,
    time::{Instant, sleep},
};

#[tokio::test]
async fn run_auto_detects_config_and_workflow_from_project_directory() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();

    write_project_files(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        format!("control_plane:\n  bind: {bind_addr}\n"),
    );
    write_memory_config(project.path());

    let mut child = spawn_run_child(project.path(), &[]);

    wait_for_health(&format!("http://{bind_addr}/healthz"))
        .await
        .expect("run command should become healthy from the project directory");
    wait_for_http_ok(&format!("http://{bind_addr}/api/v1/capabilities"))
        .await
        .expect("run command should expose gateway capabilities");
    wait_for_http_ok(&format!("http://{bind_addr}/api/v1/dashboard/snapshot"))
        .await
        .expect("run command should expose the dashboard snapshot API");

    terminate_child(&mut child).await;
}

#[tokio::test]
async fn run_config_flag_overrides_auto_detected_config_file() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start().await;
    let project = TempDir::new().expect("temp project should exist");
    let default_bind = reserve_socket_addr();
    let override_bind = reserve_socket_addr();

    write_project_files(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        format!("control_plane:\n  bind: {default_bind}\n"),
    );
    write_memory_config(project.path());
    std::fs::write(
        project.path().join("override.yaml"),
        format!("control_plane:\n  bind: {override_bind}\n"),
    )
    .expect("override config should be written");

    let mut child = spawn_run_child(project.path(), &["--config", "override.yaml"]);

    wait_for_health(&format!("http://{override_bind}/healthz"))
        .await
        .expect("explicit --config should control the bind address");
    assert!(
        !health_endpoint_ready(&format!("http://{default_bind}/healthz")).await,
        "default auto-detected config should not be used when --config is passed",
    );

    terminate_child(&mut child).await;
}

#[tokio::test]
async fn run_accepts_existing_repo_config_shape_with_extra_doctor_fields() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();

    write_project_files(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        format!(
            "target_repo: .\ncontrol_plane:\n  bind: {bind_addr}\nopenhands:\n  probe_model: fake-model\n  probe_api_key_env: FAKE_API_KEY\nlinear:\n  enabled: false\n"
        ),
    );
    write_memory_config(project.path());

    let mut child = spawn_run_child(project.path(), &[]);

    wait_for_health(&format!("http://{bind_addr}/healthz"))
        .await
        .expect("run command should ignore doctor-only config fields");

    terminate_child(&mut child).await;
}

#[test]
fn run_fails_with_install_guidance_when_managed_local_tooling_is_missing() {
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();
    std::fs::write(
        project.path().join("WORKFLOW.md"),
        r#"---
tracker:
  kind: linear
  endpoint: http://127.0.0.1:9/graphql
  project_slug: test-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: ./var/workspaces
openhands:
  transport:
    base_url: http://127.0.0.1:8000
---

# Test Workflow

Run the scheduler.
"#,
    )
    .expect("workflow should be written");
    std::fs::write(
        project.path().join("config.yaml"),
        format!(
            "control_plane:\n  bind: {bind_addr}\nopenhands:\n  tool_dir: ./managed/openhands-server\nlinear:\n  enabled: false\n"
        ),
    )
    .expect("config should be written");
    write_memory_config(project.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("run")
        .current_dir(project.path())
        .env("LINEAR_API_KEY", "test-linear-key")
        .output()
        .expect("run command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "run should fail when managed-local tooling is missing: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stderr.contains("opensymphony install openhands")
            && stderr.contains("opensymphony doctor --config <path>"),
        "run should explain how to provision the managed-local tooling: stderr={stderr}",
    );
}

// -----------------------------------------------------------------------
// LOC-18: Strict project-set boundary integration tests
// -----------------------------------------------------------------------

/// Minimal `WORKFLOW.md` body that omits every project-set-owned field —
/// the migrated shape that strict project-set mode requires (LOC-18).
const MIGRATED_WORKFLOW_BODY: &str = r#"---
workspace:
  root: ./var/workspaces
openhands:
  transport:
    base_url: http://127.0.0.1:8000
    session_api_key_env: OPENHANDS_API_KEY
---

# Migrated Workflow
"#;

/// Minimal project-set YAML used by the strict-mode run tests.
const STRICT_PROJECT_SET_YAML: &str = r#"---
schema_version: 1

project_set:
  slug: opensymphony-updates
  name: OpenSymphony Updates

  linear:
    endpoint: http://127.0.0.1:9/graphql
    project_slug: opensymphony-bootstrap-e7b957855cb7
    api_key_env: LINEAR_API_KEY
    active_states:
      - Todo
      - In Progress
    terminal_states:
      - Done

  polling:
    interval_ms: 5000

  agent:
    max_concurrent_agents: 4

  projects:
    - slug: opensymphony
      name: OpenSymphony
      repos:
        - slug: opensymphony
          url: git@github.com:Cris0904/OpenSymphony.git
          default_branch: main
"#;

/// WORKFLOW.md body that still defines project-set-owned fields — used by
/// the stale-fields run test (LOC-18).
const STALE_WORKFLOW_BODY: &str = r#"---
tracker:
  kind: linear
  endpoint: http://127.0.0.1:9/graphql
  project_slug: legacy-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: ./var/workspaces
openhands:
  transport:
    base_url: http://127.0.0.1:8000
    session_api_key_env: OPENHANDS_API_KEY
---

# Stale Workflow
"#;

/// Writes a strict-mode project-set into `.opensymphony/project-set.yaml`
/// plus a `config.yaml` that points `target_repo` at the temp project.
fn write_strict_project_set(project_root: &std::path::Path) {
    let opensymphony_dir = project_root.join(".opensymphony");
    std::fs::create_dir_all(&opensymphony_dir).expect(".opensymphony dir should exist");
    std::fs::write(
        opensymphony_dir.join("project-set.yaml"),
        STRICT_PROJECT_SET_YAML,
    )
    .expect("project-set should be written");
    std::fs::write(
        project_root.join("config.yaml"),
        "openhands:\n  tool_dir: ./managed/openhands-server\nlinear:\n  enabled: false\n",
    )
    .expect("config should be written");
}

fn spawn_run_child(project_root: &std::path::Path, extra_args: &[&str]) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_opensymphony"));
    command
        .arg("run")
        .args(extra_args)
        .current_dir(project_root)
        .env("LINEAR_API_KEY", "test-linear-key")
        .env("OPENHANDS_API_KEY", "test-openhands-key")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.spawn().expect("run command should spawn")
}

fn write_project_files(
    project_root: &std::path::Path,
    linear_base_url: &str,
    openhands_base_url: &str,
    config_contents: String,
) {
    std::fs::write(
        project_root.join("WORKFLOW.md"),
        format!(
            "---\ntracker:\n  kind: linear\n  endpoint: {linear_base_url}\n  project_slug: test-project\n  active_states:\n    - In Progress\n  terminal_states:\n    - Done\nworkspace:\n  root: ./var/workspaces\nopenhands:\n  transport:\n    base_url: {openhands_base_url}\n    session_api_key_env: OPENHANDS_API_KEY\n---\n\n# Test Workflow\n\nRun the scheduler.\n"
        ),
    )
    .expect("workflow should be written");
    std::fs::write(project_root.join("config.yaml"), config_contents)
        .expect("config should be written");
}

fn write_memory_config(project_root: &std::path::Path) {
    let memory_dir = project_root.join(".opensymphony/memory");
    std::fs::create_dir_all(&memory_dir).expect("memory dir should be written");
    std::fs::write(memory_dir.join("memory.yaml"), "areas: {}\n")
        .expect("memory config should be written");
}

fn reserve_socket_addr() -> std::net::SocketAddr {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("temporary listener should bind");
    let address = listener
        .local_addr()
        .expect("temporary listener should expose its address");
    drop(listener);
    address
}

async fn wait_for_health(url: &str) -> Result<(), String> {
    wait_for_http_ok(url).await
}

async fn wait_for_http_ok(url: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if http_endpoint_ready(url).await {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    Err(format!("timed out waiting for {url}"))
}

async fn health_endpoint_ready(url: &str) -> bool {
    http_endpoint_ready(url).await
}

async fn http_endpoint_ready(url: &str) -> bool {
    match reqwest::Client::new().get(url).send().await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

async fn terminate_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

struct MockLinearGraphqlServer {
    base_url: String,
    task: JoinHandle<()>,
}

impl MockLinearGraphqlServer {
    async fn start() -> Self {
        let app = Router::new().route("/graphql", post(handle_graphql));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock Linear listener should bind");
        let address = listener
            .local_addr()
            .expect("mock Linear listener should expose an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock Linear server should run");
        });

        Self {
            base_url: format!("http://{address}/graphql"),
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for MockLinearGraphqlServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_graphql() -> Json<serde_json::Value> {
    Json(json!({
        "data": {
            "issues": {
                "nodes": [],
                "pageInfo": {
                    "hasNextPage": false,
                    "endCursor": null
                }
            }
        }
    }))
}

// -------------------------------------------------------------------------
// LOC-18: strict project-set boundary tests for `opensymphony run`
// -------------------------------------------------------------------------

#[test]
fn run_hard_fails_with_stale_fields_diagnostic_in_project_set_mode() {
    let project = TempDir::new().expect("temp project should exist");
    write_strict_project_set(project.path());
    std::fs::write(project.path().join("WORKFLOW.md"), STALE_WORKFLOW_BODY)
        .expect("workflow should be written");
    write_memory_config(project.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("run")
        .current_dir(project.path())
        .env("LINEAR_API_KEY", "test-linear-key")
        .env("OPENHANDS_API_KEY", "test-openhands-key")
        .env_remove("RUST_LOG")
        .output()
        .expect("run command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    assert!(
        !output.status.success(),
        "run should hard-fail when WORKFLOW.md still defines project-set-owned fields in project-set mode: status={:?}, stdout={stdout}, stderr={stderr}",
        output.status.code(),
    );
    // The diagnostic must list the stale moved fields and their project-set
    // destinations, so operators know exactly what to migrate (LOC-20).
    assert!(
        combined.contains("tracker.project_slug"),
        "stale-fields diagnostic should list `tracker.project_slug`: combined={combined}",
    );
    assert!(
        combined.contains("project_set.linear.project_slug"),
        "stale-fields diagnostic should point at `project_set.linear.project_slug`: combined={combined}",
    );
    // The legacy migration guidance points at LOC-20.
    assert!(
        combined.contains("LOC-20") || combined.contains("move them"),
        "stale-fields diagnostic should explain how to migrate: combined={combined}",
    );
}

#[test]
fn run_legacy_single_repo_mode_preserves_existing_behavior_when_project_set_absent() {
    // No `.opensymphony/project-set.yaml` written: the runtime must fall
    // back to the legacy single-repo flow unchanged (LOC-18 AC).
    let project = TempDir::new().expect("temp project should exist");
    std::fs::write(
        project.path().join("WORKFLOW.md"),
        r#"---
tracker:
  kind: linear
  endpoint: http://127.0.0.1:9/graphql
  project_slug: legacy-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: ./var/workspaces
openhands:
  transport:
    base_url: http://127.0.0.1:8000
    session_api_key_env: OPENHANDS_API_KEY
---

# Legacy Workflow
"#,
    )
    .expect("workflow should be written");
    std::fs::write(
        project.path().join("config.yaml"),
        "openhands:\n  tool_dir: ./managed/openhands-server\nlinear:\n  enabled: false\n",
    )
    .expect("config should be written");
    write_memory_config(project.path());

    // We don't expect success — the Linear mock is not running and the
    // managed local OpenHands tooling is absent. But the failure must NOT
    // be the stale-fields diagnostic; the legacy flow does not flag
    // `tracker.*` because no project-set is in play.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("run")
        .current_dir(project.path())
        .env("LINEAR_API_KEY", "test-linear-key")
        .env("OPENHANDS_API_KEY", "test-openhands-key")
        .env_remove("RUST_LOG")
        .output()
        .expect("run command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    assert!(
        !output.status.success(),
        "run without project-set and without managed-local tooling should fail at a later stage: combined={combined}",
    );
    assert!(
        !combined.contains("still defines project-set-owned fields"),
        "legacy mode must NOT emit the stale-fields diagnostic: combined={combined}",
    );
}

#[test]
fn run_strict_project_set_mode_succeeds_in_resolving_migrated_workflow() {
    // Strict project-set mode with a migrated WORKFLOW.md (omitting every
    // moved field) must NOT fail at the strict-resolution stage. The run
    // may still fail later for environment reasons (no managed local
    // tooling, no live Linear), but the failure must not be the stale
    // moved-fields diagnostic.
    let project = TempDir::new().expect("temp project should exist");
    write_strict_project_set(project.path());
    std::fs::write(project.path().join("WORKFLOW.md"), MIGRATED_WORKFLOW_BODY)
        .expect("workflow should be written");
    write_memory_config(project.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("run")
        .current_dir(project.path())
        .env("LINEAR_API_KEY", "test-linear-key")
        .env("OPENHANDS_API_KEY", "test-openhands-key")
        .env_remove("RUST_LOG")
        .output()
        .expect("run command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    assert!(
        !combined.contains("still defines project-set-owned fields"),
        "strict project-set mode must accept migrated WORKFLOW.md: combined={combined}",
    );
    assert!(
        !combined.contains("StaleProjectSetFields"),
        "strict project-set mode must not surface the stale-fields error: combined={combined}",
    );
}
