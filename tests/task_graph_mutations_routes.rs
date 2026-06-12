// OSYM-721 / COE-405: integration tests for the gateway-mediated Linear
// mutation pipeline.
//
// The host client must only reach Linear through the gateway. These tests
// confirm that the `/api/v1/taskgraph/milestones`, `/issues`,
// `/sub-issues`, `/relations`, and `/evidence` routes:
//   * forward the request body and correlation_id to a fake
//     `LinearMutationClient`;
//   * return an `ActionReceipt` whose `status` reflects the Linear result;
//   * tag the receipt with the expected task-graph-update follow-up event.

#![allow(clippy::unwrap_used)]

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
use opensymphony::opensymphony_gateway_schema::action::{
    ActionKind, ActionReceipt, ActionStatus, ExpectedFollowup,
};
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

fn fixture_snapshot(step: u64) -> DaemonSnapshot {
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
            conversation_count: 2,
            status_line: "healthy".to_owned(),
        },
        memory_server: Default::default(),
        metrics: MetricsSnapshot {
            running_issues: 1,
            retry_queue_depth: 0,
            input_tokens: 2048,
            output_tokens: 2048,
            cache_read_tokens: 512,
            total_tokens: 4096 + step,
            total_cost_micros: 120_000,
        },
        issues: vec![IssueSnapshot {
            identifier: "COE-405".to_owned(),
            title: "Linear Milestone, Issue, And Sub-Issue Mutations".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: IssueRuntimeState::Idle,
            last_outcome: WorkerOutcome::Completed,
            last_event_at: now,
            conversation_id_suffix: "c0e405".to_owned(),
            workspace_path_suffix: "COE-405".to_owned(),
            retry_count: 0,
            blocked: false,
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            transport_target: Some("loopback".to_owned()),
            http_auth_mode: Some("none".to_owned()),
            websocket_auth_mode: Some("none".to_owned()),
            websocket_query_param_name: None,
            recent_events: Vec::new(),
            modified_files: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
        }],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-405".to_owned()),
            kind: RecentEventKind::SnapshotPublished,
            summary: format!("published step {step}"),
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
}

impl FakeLinearClient {
    fn new() -> Self {
        Self {
            calls: RecordedCalls::default(),
        }
    }
}

#[async_trait::async_trait]
impl LinearMutationClient for FakeLinearClient {
    async fn create_project_milestone(
        &self,
        request: TaskGraphMilestoneRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphMilestoneResponse, MutationError> {
        self.calls
            .milestone
            .lock()
            .unwrap()
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphMilestoneResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphMilestone,
            ),
            milestone_id: Some("ms_fake".into()),
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
            .unwrap()
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphIssueResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphIssue,
            ),
            issue_id: Some("iss_fake".into()),
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
            .unwrap()
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphSubIssueResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphSubIssue,
            ),
            sub_issue_id: Some("sub_fake".into()),
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
            .unwrap()
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
            .unwrap()
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
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store)
        .with_linear_mutations(Some(client as Arc<dyn LinearMutationClient>));
    let handle = tokio::spawn(async move {
        let _ = server.serve(listener).await;
    });
    sleep(Duration::from_millis(25)).await;
    (handle, addr)
}

#[tokio::test]
async fn milestones_create_returns_accepted_receipt_with_correlation_id() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphMilestoneRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-milestone-create".into(),
        op: MilestoneOp::Create,
        idempotency_key: None,
        project_id: "proj_1".into(),
        milestone_id: None,
        name: "M1 demo".into(),
        description: Some("desc".into()),
        target_date: None,
        sort_order: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/milestones"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphMilestoneResponse = resp.json().await.unwrap();
    assert_eq!(body.milestone_id.as_deref(), Some("ms_fake"));
    assert_eq!(body.receipt.status, ActionStatus::Accepted);
    assert_eq!(body.receipt.correlation_id, "corr-milestone-create");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.milestone.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "corr-milestone-create");
    handle.abort();
}

#[tokio::test]
async fn milestones_update_forwards_existing_id_to_fake_client() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphMilestoneRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-milestone-update".into(),
        op: MilestoneOp::Update,
        idempotency_key: None,
        project_id: "proj_1".into(),
        milestone_id: Some("ms_existing".into()),
        name: "M1 renamed".into(),
        description: None,
        target_date: None,
        sort_order: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/milestones"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphMilestoneResponse = resp.json().await.unwrap();
    assert_eq!(body.receipt.correlation_id, "corr-milestone-update");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.milestone.lock().unwrap();
    let (forwarded, cid) = &calls[0];
    assert_eq!(cid, "corr-milestone-update");
    assert_eq!(forwarded.milestone_id.as_deref(), Some("ms_existing"));
    handle.abort();
}

#[tokio::test]
async fn issues_create_forwards_request_and_returns_receipt() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphIssueRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-issue-create".into(),
        op: IssueOp::Create,
        idempotency_key: None,
        team_id: "team_1".into(),
        issue_id: None,
        title: "Demo issue".into(),
        description: Some("body".into()),
        priority: Some(2.0),
        estimate: Some(3.0),
        state_name: Some("Todo".into()),
        assignee_id: Some("user_1".into()),
        project_id: Some("proj_1".into()),
        project_milestone_id: None,
        label_ids: Some(vec!["label_a".into()]),
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/issues"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphIssueResponse = resp.json().await.unwrap();
    assert_eq!(body.issue_id.as_deref(), Some("iss_fake"));
    assert_eq!(body.receipt.status, ActionStatus::Accepted);
    assert_eq!(body.receipt.correlation_id, "corr-issue-create");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.issue.lock().unwrap();
    let (forwarded, cid) = &calls[0];
    assert_eq!(cid, "corr-issue-create");
    assert_eq!(forwarded.title, "Demo issue");
    assert_eq!(forwarded.team_id, "team_1");
    handle.abort();
}

#[tokio::test]
async fn issues_update_forwards_issue_id_to_fake_client() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphIssueRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-issue-update".into(),
        op: IssueOp::Update,
        idempotency_key: None,
        team_id: "team_1".into(),
        issue_id: Some("iss_existing".into()),
        title: "Renamed issue".into(),
        description: None,
        priority: None,
        estimate: None,
        state_name: Some("In Progress".into()),
        assignee_id: None,
        project_id: None,
        project_milestone_id: None,
        label_ids: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/issues"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphIssueResponse = resp.json().await.unwrap();
    assert_eq!(body.receipt.correlation_id, "corr-issue-update");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.issue.lock().unwrap();
    let (forwarded, cid) = &calls[0];
    assert_eq!(cid, "corr-issue-update");
    assert_eq!(forwarded.issue_id.as_deref(), Some("iss_existing"));
    handle.abort();
}

#[tokio::test]
async fn sub_issue_create_forwards_parent_identifier_and_returns_receipt() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphSubIssueRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-sub-issue-create".into(),
        op: SubIssueOp::Create,
        idempotency_key: None,
        team_id: "team_1".into(),
        parent_id: "parent_1".into(),
        sub_issue_id: None,
        parent_identifier: "COE-405".into(),
        title: "Sub issue".into(),
        description: None,
        priority: Some(3.0),
        estimate: None,
        state_name: None,
        assignee_id: None,
        project_id: None,
        project_milestone_id: None,
        label_ids: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/sub-issues"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphSubIssueResponse = resp.json().await.unwrap();
    assert_eq!(body.sub_issue_id.as_deref(), Some("sub_fake"));
    assert_eq!(body.receipt.status, ActionStatus::Accepted);
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.sub_issue.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.parent_identifier, "COE-405");
    handle.abort();
}

#[tokio::test]
async fn relations_create_preserves_dependency_metadata() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphRelationRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-relation".into(),
        idempotency_key: None,
        relation_type: "blocks".into(),
        issue_id: "COE-405".into(),
        related_issue_id: "COE-411".into(),
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/relations"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphRelationResponse = resp.json().await.unwrap();
    assert_eq!(body.relation_id.as_deref(), Some("rel_fake"));
    assert_eq!(body.related_issue_id.as_deref(), Some("COE-411"));
    assert_eq!(body.relation_type.as_deref(), Some("blocks"));
    assert_eq!(body.receipt.correlation_id, "corr-relation");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.relation.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.related_issue_id, "COE-411");
    handle.abort();
}

#[tokio::test]
async fn evidence_create_returns_comment_receipt_with_taskgraph_followup() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphEvidenceRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-evidence".into(),
        idempotency_key: None,
        issue_id: "COE-405".into(),
        body: "evidence body".into(),
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/evidence"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphEvidenceResponse = resp.json().await.unwrap();
    assert_eq!(body.comment_id.as_deref(), Some("c_fake"));
    assert_eq!(body.receipt.status, ActionStatus::Accepted);
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.evidence.lock().unwrap();
    assert_eq!(calls.len(), 1);
    handle.abort();
}
