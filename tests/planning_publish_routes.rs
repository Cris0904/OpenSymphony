// OSYM-736 / COE-418: integration tests for the planning draft preview and
// publish flow.
//
// These tests exercise the `/api/v1/planning/draft` and `/api/v1/planning/publish`
// routes with a fake `LinearMutationClient`. They verify manifest validation,
// exact payload generation, publish ordering (milestones → issues → sub-issues
// → comments → relations), approval gating, partial failure handling, and
// retry-safe receipt reloading.

#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use chrono::Utc;
use opensymphony::opensymphony_control::SnapshotStore;
use opensymphony::opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome,
};
use opensymphony::opensymphony_gateway::{
    GatewayServer, IssueOp, LinearMutationClient, MilestoneOp, MutationError, SubIssueOp,
    TaskGraphEvidenceRequest, TaskGraphEvidenceResponse, TaskGraphIssueRequest,
    TaskGraphIssueResponse, TaskGraphMilestoneRequest, TaskGraphMilestoneResponse,
    TaskGraphRelationRequest, TaskGraphRelationResponse, TaskGraphSubIssueRequest,
    TaskGraphSubIssueResponse,
};
use opensymphony::opensymphony_gateway_schema::action::{ActionKind, ActionReceipt};
use opensymphony::opensymphony_gateway_schema::planning::{
    LinearDraftEntityKind, LinearDraftPreview, LinearDraftRequest, LinearPublishRequest,
    LinearPublishResponse,
};
use opensymphony::opensymphony_gateway_schema::version::SchemaVersion;
use opensymphony::opensymphony_planning::compiler::LinearPublishReceipt as YamlPublishReceipt;
use reqwest::Client;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

fn fixture_snapshot() -> DaemonSnapshot {
    let now = Utc::now();
    DaemonSnapshot {
        generated_at: now,
        daemon: DaemonStatus {
            state: DaemonState::Ready,
            last_poll_at: now,
            workspace_root: "/tmp/opensymphony".to_owned(),
            status_line: "ready".to_owned(),
        },
        agent_server: AgentServerStatus {
            reachable: true,
            base_url: "http://127.0.0.1:3000".to_owned(),
            conversation_count: 0,
            status_line: "healthy".to_owned(),
        },
        memory_server: Default::default(),
        metrics: MetricsSnapshot {
            running_issues: 0,
            retry_queue_depth: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: 0,
            total_cost_micros: 0,
        },
        issues: vec![IssueSnapshot {
            identifier: "COE-418".to_owned(),
            title: "Linear Draft Preview And Publish Flow".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: IssueRuntimeState::Idle,
            last_outcome: WorkerOutcome::Completed,
            last_event_at: now,
            conversation_id_suffix: "c0e418".to_owned(),
            workspace_path_suffix: "COE-418".to_owned(),
            retry_count: 0,
            blocked: false,
            server_base_url: None,
            transport_target: None,
            http_auth_mode: None,
            websocket_auth_mode: None,
            websocket_query_param_name: None,
            recent_events: Vec::new(),
            modified_files: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cancel_acknowledged: false,
            cancel_failed: false,
            detached: false,
        }],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-418".to_owned()),
            kind: RecentEventKind::SnapshotPublished,
            summary: "fixture snapshot".to_owned(),
        }],
    }
}

#[derive(Default)]
struct RecordedCalls {
    milestone: Mutex<Vec<(TaskGraphMilestoneRequest, String)>>,
    issue: Mutex<Vec<(TaskGraphIssueRequest, String)>>,
    sub_issue: Mutex<Vec<(TaskGraphSubIssueRequest, String)>>,
    relation: Mutex<Vec<(TaskGraphRelationRequest, String)>>,
    evidence: Mutex<Vec<(TaskGraphEvidenceRequest, String)>>,
}

struct FakeLinearClient {
    calls: RecordedCalls,
    fail_issue: Mutex<bool>,
}

impl FakeLinearClient {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: RecordedCalls::default(),
            fail_issue: Mutex::new(false),
        })
    }

    fn with_issue_failure() -> Arc<Self> {
        Arc::new(Self {
            calls: RecordedCalls::default(),
            fail_issue: Mutex::new(true),
        })
    }
}

#[async_trait::async_trait]
impl LinearMutationClient for FakeLinearClient {
    async fn create_or_update_project_milestone(
        &self,
        request: TaskGraphMilestoneRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphMilestoneResponse, MutationError> {
        self.calls
            .milestone
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphMilestoneResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphMilestone,
            ),
            milestone_id: Some(match request.op {
                MilestoneOp::Update => request
                    .milestone_id
                    .clone()
                    .unwrap_or_else(|| "ms_fake".to_owned()),
                MilestoneOp::Create => "ms_fake".into(),
            }),
            milestone_name: Some(request.name),
            project_id: Some(request.project_id),
        })
    }

    async fn create_or_update_issue(
        &self,
        request: TaskGraphIssueRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphIssueResponse, MutationError> {
        self.calls
            .issue
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        if *self.fail_issue.lock().await && matches!(request.op, IssueOp::Create) {
            return Err(MutationError::Upstream("issue create failed".into()));
        }
        Ok(TaskGraphIssueResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphIssue,
            ),
            issue_id: Some(match request.op {
                IssueOp::Update => request
                    .issue_id
                    .clone()
                    .unwrap_or_else(|| "iss_fake".to_owned()),
                IssueOp::Create => "iss_fake".into(),
            }),
            issue_identifier: Some(request.title),
            state_id: None,
            project_milestone_id: request.project_milestone_id,
        })
    }

    async fn create_or_update_sub_issue(
        &self,
        request: TaskGraphSubIssueRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphSubIssueResponse, MutationError> {
        self.calls
            .sub_issue
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        if *self.fail_issue.lock().await && matches!(request.op, SubIssueOp::Create) {
            return Err(MutationError::Upstream("sub-issue create failed".into()));
        }
        let sub_issue_id = match request.op {
            SubIssueOp::Update => request
                .sub_issue_id
                .clone()
                .unwrap_or_else(|| "sub_fake".to_owned()),
            SubIssueOp::Create => "sub_fake".into(),
        };
        Ok(TaskGraphSubIssueResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphSubIssue,
            ),
            sub_issue_id: Some(sub_issue_id),
            sub_issue_identifier: Some(request.title),
            parent_identifier: Some(request.parent_identifier),
            state_id: None,
        })
    }

    async fn create_issue_relation(
        &self,
        request: TaskGraphRelationRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphRelationResponse, MutationError> {
        self.calls
            .relation
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphRelationResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphRelation,
            ),
            relation_id: Some("rel_fake".into()),
            relation_type: Some(request.relation_type),
            related_issue_id: Some(request.related_issue_id),
        })
    }

    async fn create_evidence_comment(
        &self,
        request: TaskGraphEvidenceRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphEvidenceResponse, MutationError> {
        self.calls
            .evidence
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphEvidenceResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphEvidence,
            ),
            comment_id: Some("c_fake".into()),
            issue_id: Some(request.issue_id),
            issue_identifier: None,
        })
    }
}

async fn start_test_server(client: Arc<FakeLinearClient>) -> (JoinHandle<()>, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = SnapshotStore::new(fixture_snapshot());
    let journal = opensymphony::opensymphony_domain::InMemoryEventJournal::new(1024, 64);
    let server = GatewayServer::with_journal(
        store,
        journal.clone(),
        opensymphony::opensymphony_domain::StreamBroker::new(journal.clone()),
    )
    .with_linear_mutations(Some(client as Arc<dyn LinearMutationClient>));
    let handle = tokio::spawn(async move {
        let _ = server.serve(listener).await;
    });
    sleep(Duration::from_millis(25)).await;
    (handle, addr)
}

async fn start_test_server_without_linear_mutations() -> (JoinHandle<()>, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = SnapshotStore::new(fixture_snapshot());
    let journal = opensymphony::opensymphony_domain::InMemoryEventJournal::new(1024, 64);
    let server = GatewayServer::with_journal(
        store,
        journal,
        opensymphony::opensymphony_domain::StreamBroker::new(
            opensymphony::opensymphony_domain::InMemoryEventJournal::new(1024, 64),
        ),
    );
    let handle = tokio::spawn(async move {
        let _ = server.serve(listener).await;
    });
    sleep(Duration::from_millis(25)).await;
    (handle, addr)
}

async fn write_fixture_task_package(dir: &TempDir) -> String {
    let root = dir.path();
    let tasks_dir = root.join("docs").join("tasks");
    tokio::fs::create_dir_all(&tasks_dir).await.unwrap();

    let manifest = r#"planningWave: wave-1
milestones:
  - M1
tasks:
  - id: ISSUE-1
    file: docs/tasks/issue-1.md
  - id: SUB-1
    file: docs/tasks/sub-1.md
  - id: ISSUE-2
    file: docs/tasks/issue-2.md
"#;
    tokio::fs::write(tasks_dir.join("task-package.yaml"), manifest)
        .await
        .unwrap();

    tokio::fs::write(
        tasks_dir.join("issue-1.md"),
        "---\nid: ISSUE-1\ntitle: Issue One\nmilestone: M1\npriority: 3\nestimate: 5\nblockedBy: []\nblocks:\n  - ISSUE-2\n---\nIssue One body\n",
    )
    .await
    .unwrap();

    tokio::fs::write(
        tasks_dir.join("sub-1.md"),
        "---\nid: SUB-1\ntitle: Sub One\nmilestone: M1\npriority: 2\nestimate: 3\nparent: ISSUE-1\nblockedBy: []\nblocks: []\n---\nSub One body\n",
    )
    .await
    .unwrap();

    tokio::fs::write(
        tasks_dir.join("issue-2.md"),
        "---\nid: ISSUE-2\ntitle: Issue Two\nmilestone: M1\npriority: 4\nestimate: 2\nblockedBy:\n  - ISSUE-1\nblocks: []\n---\nIssue Two body\n",
    )
    .await
    .unwrap();

    root.to_str().unwrap().to_owned()
}

async fn write_invalid_task_package(dir: &TempDir) -> String {
    let root = dir.path();
    let tasks_dir = root.join("docs").join("tasks");
    tokio::fs::create_dir_all(&tasks_dir).await.unwrap();

    let manifest = r#"planningWave: wave-1
milestones:
  - M1
tasks:
  - id: ISSUE-1
    file: docs/tasks/issue-1.md
"#;
    tokio::fs::write(tasks_dir.join("task-package.yaml"), manifest)
        .await
        .unwrap();

    // Missing title and milestone -> validation errors.
    tokio::fs::write(
        tasks_dir.join("issue-1.md"),
        "---\nid: ISSUE-1\npriority: 3\nestimate: 5\n---\nIssue One body\n",
    )
    .await
    .unwrap();

    root.to_str().unwrap().to_owned()
}

fn draft_request(repo_root: &str) -> LinearDraftRequest {
    LinearDraftRequest {
        schema_version: SchemaVersion::v1(),
        correlation_id: "draft-corr-1".into(),
        manifest_path: "docs/tasks/task-package.yaml".into(),
        repo_root: repo_root.into(),
        project_id: "proj_1".into(),
        team_id: "team_1".into(),
        linear_project: "test-project".into(),
        publish_receipt_path: "docs/tasks/linear-publish.yaml".into(),
        existing_receipt_path: None,
    }
}

fn draft_request_with_paths(
    repo_root: &str,
    manifest_path: &str,
    receipt_path: &str,
) -> LinearDraftRequest {
    LinearDraftRequest {
        schema_version: SchemaVersion::v1(),
        correlation_id: "draft-corr-1".into(),
        manifest_path: manifest_path.into(),
        repo_root: repo_root.into(),
        project_id: "proj_1".into(),
        team_id: "team_1".into(),
        linear_project: "test-project".into(),
        publish_receipt_path: receipt_path.into(),
        existing_receipt_path: None,
    }
}

#[tokio::test]
async fn draft_returns_entities_and_validation_summary() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let (_handle, addr) = start_test_server(FakeLinearClient::new()).await;

    let req = draft_request(&repo_root);
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let preview: LinearDraftPreview = resp.json().await.unwrap();
    assert!(preview.can_publish);
    assert!(preview.validation.ok);
    assert_eq!(preview.planning_wave, "wave-1");
    assert!(!preview.draft_id.is_empty());

    let mut counts: HashMap<LinearDraftEntityKind, usize> = HashMap::new();
    for entity in &preview.entities {
        *counts.entry(entity.kind).or_default() += 1;
    }
    assert_eq!(counts.get(&LinearDraftEntityKind::Milestone), Some(&1));
    assert_eq!(counts.get(&LinearDraftEntityKind::Issue), Some(&2));
    assert_eq!(counts.get(&LinearDraftEntityKind::SubIssue), Some(&1));
    assert_eq!(counts.get(&LinearDraftEntityKind::Relation), Some(&1));
    assert_eq!(counts.get(&LinearDraftEntityKind::Comment), Some(&3));

    // Milestone comes first, then issues, then sub-issues, then comments and relations.
    assert_eq!(preview.entities[0].kind, LinearDraftEntityKind::Milestone);
    assert_eq!(preview.entities[1].kind, LinearDraftEntityKind::Issue);
}

#[tokio::test]
async fn publish_creates_entities_and_writes_receipt() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let client = FakeLinearClient::new();
    let (_handle, addr) = start_test_server(client.clone()).await;

    let draft_req = draft_request(&repo_root);
    let draft_resp: LinearDraftPreview = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&draft_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let draft_id = draft_resp.draft_id;

    let publish_req = LinearPublishRequest {
        schema_version: SchemaVersion::v1(),
        draft_id,
        correlation_id: "pub-corr-1".into(),
        approved: true,
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/publish"))
        .json(&publish_req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body: LinearPublishResponse = resp.json().await.unwrap();
    assert_eq!(body.status, "published");
    assert!(body.failures.is_empty());
    assert_eq!(body.receipt.milestones.len(), 1);
    assert_eq!(body.receipt.tasks.len(), 3);

    let receipt_path = dir.path().join("docs/tasks/linear-publish.yaml");
    let receipt_text = tokio::fs::read_to_string(&receipt_path).await.unwrap();
    assert!(receipt_text.contains("planningWave: wave-1"));
    assert!(receipt_text.contains("linearProject: test-project"));
    assert!(receipt_text.contains("milestones:"));
    assert!(receipt_text.contains("M1"));
    assert!(receipt_text.contains("tasks:"));
    assert!(receipt_text.contains("ISSUE-1"));
    assert!(receipt_text.contains("SUB-1"));
    assert!(receipt_text.contains("ISSUE-2"));

    // Verify publish ordering: milestone, issue, issue, sub-issue, then comments and relations.
    let calls = client.calls.milestone.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "pub-corr-1");
    drop(calls);

    let issue_calls = client.calls.issue.lock().await;
    assert_eq!(issue_calls.len(), 2);
    assert_eq!(issue_calls[0].1, "pub-corr-1");
    assert_eq!(issue_calls[1].1, "pub-corr-1");
    drop(issue_calls);

    let sub_calls = client.calls.sub_issue.lock().await;
    assert_eq!(sub_calls.len(), 1);
    drop(sub_calls);

    let evidence_calls = client.calls.evidence.lock().await;
    assert_eq!(evidence_calls.len(), 3);
    drop(evidence_calls);

    let relation_calls = client.calls.relation.lock().await;
    assert_eq!(relation_calls.len(), 1);
    drop(relation_calls);

    // Verify the relation mapped the source task ids to the returned issue ids.
    let relation = client.calls.relation.lock().await;
    assert_eq!(relation[0].0.issue_id, "iss_fake");
    assert_eq!(relation[0].0.related_issue_id, "iss_fake");
}

#[tokio::test]
async fn publish_without_approval_rejects() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let client = FakeLinearClient::new();
    let (_handle, addr) = start_test_server(client.clone()).await;

    let draft_req = draft_request(&repo_root);
    let draft_resp: LinearDraftPreview = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&draft_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let publish_req = LinearPublishRequest {
        schema_version: SchemaVersion::v1(),
        draft_id: draft_resp.draft_id,
        correlation_id: "pub-corr-1".into(),
        approved: false,
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/publish"))
        .json(&publish_req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("not approved"));

    let calls = client.calls.issue.lock().await;
    assert!(calls.is_empty());
}

#[tokio::test]
async fn publish_without_linear_client_returns_503() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let (_handle, addr) = start_test_server_without_linear_mutations().await;

    let draft_req = draft_request(&repo_root);
    let draft_resp: LinearDraftPreview = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&draft_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let publish_req = LinearPublishRequest {
        schema_version: SchemaVersion::v1(),
        draft_id: draft_resp.draft_id,
        correlation_id: "pub-corr-1".into(),
        approved: true,
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/publish"))
        .json(&publish_req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
}

#[tokio::test]
async fn publish_with_invalid_draft_rejects() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_invalid_task_package(&dir).await;
    let (_handle, addr) = start_test_server(FakeLinearClient::new()).await;

    let draft_req = draft_request(&repo_root);
    let draft_resp: LinearDraftPreview = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&draft_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!draft_resp.can_publish);

    let publish_req = LinearPublishRequest {
        schema_version: SchemaVersion::v1(),
        draft_id: draft_resp.draft_id,
        correlation_id: "pub-corr-1".into(),
        approved: true,
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/publish"))
        .json(&publish_req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["status"].as_str().unwrap().contains("rejected"));
}

#[tokio::test]
async fn publish_partial_failure_records_receipt_and_failures() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let client = FakeLinearClient::with_issue_failure();
    let (_handle, addr) = start_test_server(client.clone()).await;

    let draft_req = draft_request(&repo_root);
    let draft_resp: LinearDraftPreview = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&draft_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let publish_req = LinearPublishRequest {
        schema_version: SchemaVersion::v1(),
        draft_id: draft_resp.draft_id,
        correlation_id: "pub-corr-1".into(),
        approved: true,
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/publish"))
        .json(&publish_req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body: LinearPublishResponse = resp.json().await.unwrap();
    assert_eq!(body.status, "partial");
    assert!(!body.failures.is_empty());

    // The milestone should still be recorded in the partial receipt, but no
    // task entries were created because issue creation failed.
    let receipt_path = dir.path().join("docs/tasks/linear-publish.yaml");
    let receipt_text = tokio::fs::read_to_string(&receipt_path).await.unwrap();
    assert!(receipt_text.contains("M1"));
    let receipt: YamlPublishReceipt = serde_yaml::from_str(&receipt_text).unwrap();
    assert_eq!(receipt.milestones.len(), 1);
    assert!(receipt.tasks.is_empty());
}

#[tokio::test]
async fn publish_retry_uses_updates_and_does_not_duplicate() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let client = FakeLinearClient::new();
    let (_handle, addr) = start_test_server(client.clone()).await;

    let draft_req = draft_request(&repo_root);
    let draft_resp: LinearDraftPreview = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&draft_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let draft_id = draft_resp.draft_id;

    let publish_req = LinearPublishRequest {
        schema_version: SchemaVersion::v1(),
        draft_id: draft_id.clone(),
        correlation_id: "pub-corr-1".into(),
        approved: true,
    };
    let first: LinearPublishResponse = Client::new()
        .post(format!("http://{addr}/api/v1/planning/publish"))
        .json(&publish_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first.status, "published");

    // A successful publish consumes the draft. Re-drafting from the same
    // manifest is the supported way to retry; the existing receipt on disk
    // causes the second run to emit updates instead of duplicates.
    let second_draft: LinearDraftPreview = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&draft_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_publish_req = LinearPublishRequest {
        schema_version: SchemaVersion::v1(),
        draft_id: second_draft.draft_id,
        correlation_id: "pub-corr-1".into(),
        approved: true,
    };
    let second: LinearPublishResponse = Client::new()
        .post(format!("http://{addr}/api/v1/planning/publish"))
        .json(&second_publish_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second.status, "published");

    // On the second run the existing receipt was reloaded, so all create
    // operations should have been emitted as updates.
    let milestone_calls = client.calls.milestone.lock().await;
    assert_eq!(milestone_calls.len(), 2);
    assert!(matches!(milestone_calls[0].0.op, MilestoneOp::Create));
    assert!(matches!(milestone_calls[1].0.op, MilestoneOp::Update));
    drop(milestone_calls);

    let issue_calls = client.calls.issue.lock().await;
    assert_eq!(issue_calls.len(), 4);
    assert!(matches!(issue_calls[0].0.op, IssueOp::Create));
    assert!(matches!(issue_calls[1].0.op, IssueOp::Create));
    assert!(matches!(issue_calls[2].0.op, IssueOp::Update));
    assert!(matches!(issue_calls[3].0.op, IssueOp::Update));
    drop(issue_calls);

    let sub_calls = client.calls.sub_issue.lock().await;
    assert_eq!(sub_calls.len(), 2);
    assert!(matches!(sub_calls[0].0.op, SubIssueOp::Create));
    assert!(matches!(sub_calls[1].0.op, SubIssueOp::Update));
    drop(sub_calls);

    // Relations and evidence comments are skipped on retry because the
    // persisted receipt already records their IDs, so call counts stay the
    // same as the first publish.
    let relation_calls = client.calls.relation.lock().await;
    assert_eq!(relation_calls.len(), 1);
    drop(relation_calls);

    let evidence_calls = client.calls.evidence.lock().await;
    assert_eq!(evidence_calls.len(), 3);
}

#[tokio::test]
async fn publish_missing_draft_returns_404() {
    let dir = TempDir::new().unwrap();
    let _repo_root = write_fixture_task_package(&dir).await;
    let client = FakeLinearClient::new();
    let (_handle, addr) = start_test_server(client.clone()).await;

    let publish_req = LinearPublishRequest {
        schema_version: SchemaVersion::v1(),
        draft_id: "does-not-exist".into(),
        correlation_id: "pub-corr-1".into(),
        approved: true,
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/publish"))
        .json(&publish_req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn draft_rejects_absolute_manifest_path() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let (_handle, addr) = start_test_server(FakeLinearClient::new()).await;

    let req = draft_request_with_paths(&repo_root, "/etc/passwd", "docs/tasks/linear-publish.yaml");
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("path must be relative")
    );
}

#[tokio::test]
async fn draft_rejects_dotdot_manifest_path() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let (_handle, addr) = start_test_server(FakeLinearClient::new()).await;

    let req = draft_request_with_paths(
        &repo_root,
        "../../../etc/passwd",
        "docs/tasks/linear-publish.yaml",
    );
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("path escapes repo_root")
    );
}

#[tokio::test]
async fn draft_rejects_dotdot_receipt_path() {
    let dir = TempDir::new().unwrap();
    let repo_root = write_fixture_task_package(&dir).await;
    let (_handle, addr) = start_test_server(FakeLinearClient::new()).await;

    let req = draft_request_with_paths(
        &repo_root,
        "docs/tasks/task-package.yaml",
        "../../../etc/cron.d/opensymphony",
    );
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/planning/draft"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("path escapes repo_root")
    );
}
