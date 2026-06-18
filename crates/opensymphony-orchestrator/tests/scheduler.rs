use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
};

use crate::opensymphony_orchestrator::{
    ConversationId, ConversationMetadata, IssueId, IssueIdentifier, IssueRef, IssueState,
    IssueStateCategory, NormalizedIssue, RecoveryRecord, ReleaseReason, RepoRef, RetryReason,
    RuntimeStreamState, Scheduler, SchedulerConfig, SchedulerStatus, TimestampMs, TrackerBackend,
    TrackerIssue, TrackerIssueRef, TrackerIssueState, TrackerIssueStateKind,
    TrackerIssueStateSnapshot, WorkerAbortReason, WorkerBackend, WorkerId, WorkerLaunch,
    WorkerOutcomeKind, WorkerOutcomeRecord, WorkerStartRequest, WorkerUpdate, WorkspaceBackend,
    WorkspaceKey, WorkspaceRecord,
};
use chrono::{TimeZone, Utc};

fn ts(value: u64) -> TimestampMs {
    TimestampMs::new(value)
}

fn dt(value: u64) -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(value as i64)
        .single()
        .expect("timestamp should be valid")
}

fn scheduler_config() -> SchedulerConfig {
    // LOC-13: default helper includes the project-set inventory used by
    // `repo_for_issue` so the LOC-14 dispatch gate (D6) does not block
    // the legacy "dispatch happy-path" tests below. Tests that exercise
    // the gate's missing-repo behavior use `scheduler_config_with_inventory`
    // explicitly with an empty or different inventory.
    SchedulerConfig {
        poll_interval_ms: 1_000,
        max_concurrent_agents: 2,
        max_turns: 4,
        max_concurrent_agents_by_state: BTreeMap::new(),
        retry_policy: Default::default(),
        stall_timeout_ms: Some(100),
        active_states: vec!["In Progress".to_string()],
        terminal_states: vec!["Done".to_string(), "Canceled".to_string()],
        project_set_inventory: default_repo_inventory(),
    }
}

fn default_repo_inventory() -> BTreeMap<String, RepoRef> {
    let mut map = BTreeMap::new();
    map.insert(
        "test-repo".to_string(),
        RepoRef {
            url: "https://example.com/test-repo.git".to_string(),
            key: "test-repo".to_string(),
            default_branch: Some("main".to_string()),
        },
    );
    map
}

fn tracker_issue(id: &str, identifier: &str, state: &str, created_at: u64) -> TrackerIssue {
    // LOC-14 D6 dispatch gate: by default we attach a single, resolvable
    // `repo:test-repo` label so the gate does not block the legacy
    // dispatch happy-path tests. Tests that exercise missing-repo
    // behavior override this with `tracker_issue_with_labels(..., &[])`
    // or an explicit unresolved label set.
    tracker_issue_with_labels(id, identifier, state, created_at, &["repo:test-repo"])
}

fn tracker_issue_with_labels(
    id: &str,
    identifier: &str,
    state: &str,
    created_at: u64,
    labels: &[&str],
) -> TrackerIssue {
    TrackerIssue {
        id: id.to_string(),
        identifier: identifier.to_string(),
        url: format!("https://linear.app/example/{identifier}"),
        title: format!("Issue {identifier}"),
        description: Some("scheduler test fixture".to_string()),
        priority: Some(1),
        state: state.to_string(),
        labels: labels.iter().map(|label| label.to_string()).collect(),
        parent_id: None,
        parent: None,
        project_milestone: None,
        blocked_by: Vec::new(),
        sub_issues: Vec::new(),
        created_at: dt(created_at),
        updated_at: dt(created_at),
    }
}

/// LOC-14 D10: build a parent tracker issue whose `sub_issues` carry
/// terminal children, so the LOC-14 parent-deferred gate can fire in
/// `dispatch_ready_issues`. `child_states` are the states of each child
/// (e.g. `&[("COE-CHILD", "Done")]`).
fn tracker_parent_with_children(
    id: &str,
    identifier: &str,
    state: &str,
    created_at: u64,
    labels: &[&str],
    child_states: &[(&str, &str)],
) -> TrackerIssue {
    let mut issue = tracker_issue_with_labels(id, identifier, state, created_at, labels);
    issue.sub_issues = child_states
        .iter()
        .map(|(child_identifier, child_state)| TrackerIssueRef {
            id: format!("lin-{child_identifier}"),
            identifier: child_identifier.to_string(),
            title: Some(format!("Child {child_identifier}")),
            url: Some(format!("https://linear.app/example/{child_identifier}")),
            state: child_state.to_string(),
        })
        .collect();
    issue
}

fn normalized_issue(id: &str, identifier: &str, state: &str) -> NormalizedIssue {
    NormalizedIssue {
        id: IssueId::new(id).expect("issue id should be valid"),
        identifier: IssueIdentifier::new(identifier).expect("issue identifier should be valid"),
        title: format!("Issue {identifier}"),
        description: None,
        priority: Some(1),
        state: IssueState {
            id: None,
            name: state.to_string(),
            category: if state == "In Progress" {
                IssueStateCategory::Active
            } else if matches!(state, "Done" | "Canceled") {
                IssueStateCategory::Terminal
            } else {
                IssueStateCategory::NonActive
            },
        },
        branch_name: None,
        url: Some(format!("https://linear.app/example/{identifier}")),
        labels: Vec::new(),
        parent_id: None,
        blocked_by: Vec::new(),
        sub_issues: vec![IssueRef {
            id: IssueId::new(format!("{id}-child")).expect("child id should be valid"),
            identifier: IssueIdentifier::new(format!("{identifier}-child"))
                .expect("child identifier should be valid"),
            state: "Done".to_string(),
        }],
        created_at: Some(ts(0)),
        updated_at: Some(ts(0)),
        execution_repo_ref: None,
    }
}

fn tracker_state_snapshot(
    id: &str,
    identifier: &str,
    state: &str,
    tracker_type: &str,
    updated_at: u64,
) -> TrackerIssueStateSnapshot {
    TrackerIssueStateSnapshot {
        id: id.to_string(),
        identifier: identifier.to_string(),
        state: TrackerIssueState {
            id: state.to_ascii_lowercase().replace(' ', "-"),
            name: state.to_string(),
            tracker_type: tracker_type.to_string(),
            kind: TrackerIssueStateKind::from_tracker_type(tracker_type),
        },
        updated_at: dt(updated_at),
    }
}

fn workspace_record(identifier: &str, path: &str) -> WorkspaceRecord {
    WorkspaceRecord {
        path: PathBuf::from(path),
        workspace_key: WorkspaceKey::new(identifier).expect("workspace key should be valid"),
        created_now: false,
        created_at: Some(ts(0)),
        updated_at: Some(ts(0)),
        last_seen_tracker_refresh_at: Some(ts(0)),
    }
}

fn conversation(worker_id: &WorkerId) -> ConversationMetadata {
    ConversationMetadata {
        conversation_id: ConversationId::new(format!("conv-{}", worker_id.as_str()))
            .expect("conversation id should be valid"),
        server_base_url: Some("http://127.0.0.1:8000".to_string()),
        transport_target: Some("loopback".to_string()),
        http_auth_mode: Some("none".to_string()),
        websocket_auth_mode: Some("none".to_string()),
        websocket_query_param_name: None,
        fresh_conversation: true,
        runtime_contract_version: Some("openhands-sdk-agent-server-v1".to_string()),
        stream_state: RuntimeStreamState::Ready,
        last_event_id: None,
        last_event_kind: None,
        last_event_at: None,
        last_event_summary: None,
        recent_activity: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 0,
        runtime_seconds: 0,
        next_activity_sequence: 0,
    }
}

#[derive(Debug, Clone)]
struct FakeError(String);

impl std::fmt::Display for FakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FakeError {}

#[derive(Default)]
struct FakeTracker {
    active: Vec<TrackerIssue>,
    terminal: Vec<TrackerIssue>,
    states: HashMap<String, TrackerIssueStateSnapshot>,
    state_requests: Vec<Vec<String>>,
}

impl TrackerBackend for FakeTracker {
    type Error = FakeError;

    async fn candidate_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        Ok(self.active.clone())
    }

    async fn terminal_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        Ok(self.terminal.clone())
    }

    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, Self::Error> {
        self.state_requests.push(issue_ids.to_vec());
        Ok(issue_ids
            .iter()
            .filter_map(|id| self.states.get(id).cloned())
            .collect())
    }
}

#[derive(Default)]
struct FakeWorkspace {
    recoveries: Vec<RecoveryRecord>,
    ensured: Vec<String>,
    cleaned: Vec<(String, bool)>,
    records: HashMap<String, WorkspaceRecord>,
}

impl WorkspaceBackend for FakeWorkspace {
    type Error = FakeError;

    async fn ensure_workspace(
        &mut self,
        issue: &NormalizedIssue,
        _observed_at: TimestampMs,
    ) -> Result<WorkspaceRecord, Self::Error> {
        self.ensured.push(issue.identifier.to_string());
        let record = self
            .records
            .entry(issue.id.to_string())
            .or_insert_with(|| {
                workspace_record(
                    issue.identifier.as_str(),
                    &format!("/tmp/workspaces/{}", issue.identifier),
                )
            })
            .clone();
        Ok(record)
    }

    async fn recover_workspaces(&mut self) -> Result<Vec<RecoveryRecord>, Self::Error> {
        Ok(self.recoveries.clone())
    }

    async fn cleanup_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
        terminal: bool,
    ) -> Result<(), Self::Error> {
        self.cleaned
            .push((workspace.workspace_key.to_string(), terminal));
        Ok(())
    }
}

#[derive(Default)]
struct FakeWorker {
    launches: Vec<WorkerStartRequest>,
    updates: VecDeque<WorkerUpdate>,
    aborted: Vec<(String, WorkerAbortReason)>,
    launch_results: VecDeque<Result<WorkerLaunch, FakeError>>,
}

impl WorkerBackend for FakeWorker {
    type Error = FakeError;

    async fn start_worker(
        &mut self,
        request: WorkerStartRequest,
    ) -> Result<WorkerLaunch, Self::Error> {
        self.launches.push(request.clone());
        match self.launch_results.pop_front() {
            Some(result) => result,
            None => Ok(WorkerLaunch {
                conversation: conversation(&request.run.worker_id),
            }),
        }
    }

    async fn poll_updates(&mut self) -> Result<Vec<WorkerUpdate>, Self::Error> {
        Ok(self.updates.drain(..).collect())
    }

    async fn abort_worker(
        &mut self,
        worker_id: &WorkerId,
        reason: WorkerAbortReason,
    ) -> Result<(), Self::Error> {
        self.aborted.push((worker_id.to_string(), reason));
        Ok(())
    }
}

#[tokio::test]
async fn successful_worker_exit_queues_continuation_retry_for_active_issue() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-268", "COE-268", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-268").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("execution should exist")
            .status(),
        SchedulerStatus::Running
    );
    assert_eq!(scheduler.worker().launches.len(), 1);

    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Succeeded,
                ts(200),
                Some("worker exited cleanly".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("second tick should succeed");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    let retry = execution.retry().expect("retry metadata should exist");
    assert_eq!(retry.reason, RetryReason::Continuation);
    assert_eq!(retry.due_at, ts(1_200));

    scheduler
        .tick(ts(1_300))
        .await
        .expect("third tick should redispatch the issue");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(scheduler.worker().launches.len(), 2);
    let second_run = &scheduler.worker().launches[1].run;
    assert_eq!(
        second_run
            .attempt
            .expect("retry run should carry a retry attempt")
            .get(),
        1
    );
    assert_eq!(second_run.normal_retry_count, 1);
}

#[tokio::test]
async fn failures_schedule_exponential_backoff() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-269", "COE-269", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-269").expect("issue id should be valid");
    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Failed,
                ts(200),
                Some("worker failed".to_string()),
                Some("boom".to_string()),
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("failure tick should succeed");

    let retry = scheduler
        .execution(&issue_id)
        .expect("execution should exist")
        .retry()
        .expect("retry should exist")
        .clone();
    assert_eq!(retry.reason, RetryReason::Failure);
    assert_eq!(retry.due_at, ts(10_200));

    scheduler
        .tick(ts(10_200))
        .await
        .expect("first retry dispatch should succeed");

    let second_run = scheduler.worker().launches[1].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: second_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &second_run,
                WorkerOutcomeKind::Failed,
                ts(10_400),
                Some("worker failed again".to_string()),
                Some("still broken".to_string()),
            ),
        });

    scheduler
        .tick(ts(10_400))
        .await
        .expect("second failure tick should succeed");

    let retry = scheduler
        .execution(&issue_id)
        .expect("execution should exist")
        .retry()
        .expect("retry should exist")
        .clone();
    assert_eq!(
        retry.attempt.get(),
        2,
        "second retry should increment the retry attempt"
    );
    assert_eq!(retry.due_at, ts(30_400));
}

#[tokio::test]
async fn per_state_capacity_releases_slot_after_worker_finishes() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-275", "COE-275", "In Progress", 0),
            tracker_issue("lin-276", "COE-276", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch the first issue");

    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Succeeded,
                ts(200),
                Some("worker exited cleanly".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("finish tick should free the state slot for the next issue");

    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-276"
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-275").expect("issue id should be valid"))
            .expect("finished issue should still exist")
            .status(),
        SchedulerStatus::RetryQueued
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-276").expect("issue id should be valid"))
            .expect("second issue should be running")
            .status(),
        SchedulerStatus::Running
    );
}

#[tokio::test]
async fn terminal_reconciliation_aborts_running_worker_and_cleans_up_workspace() {
    let issue = tracker_issue("lin-270", "COE-270", "In Progress", 0);
    let tracker = FakeTracker {
        active: vec![
            issue.clone(),
            tracker_issue("lin-270-b", "COE-270-B", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    scheduler.tracker_mut().active =
        vec![tracker_issue("lin-270-b", "COE-270-B", "In Progress", 1)];
    scheduler.tracker_mut().terminal = vec![tracker_issue("lin-270", "COE-270", "Done", 0)];

    scheduler
        .tick(ts(200))
        .await
        .expect("terminal reconciliation should succeed");

    let issue_id = IssueId::new("lin-270").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("released execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::Released);
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerTerminal);
        }
        other => panic!("expected released state, got {other:?}"),
    }
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::TrackerTerminal
    );
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-270".to_string(), true)]
    );
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-270-B"
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-270-b").expect("issue id should be valid"))
            .expect("replacement issue should be running")
            .status(),
        SchedulerStatus::Running
    );
}

#[tokio::test]
async fn runtime_events_extend_stall_deadlines_before_retrying_a_stalled_worker() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-271", "COE-271", "In Progress", 0),
            tracker_issue("lin-271-b", "COE-271-B", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(0))
        .await
        .expect("first tick should succeed");

    let running = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::RuntimeEvent {
            worker_id: running.worker_id.clone(),
            observed_at: ts(50),
            event_id: Some("evt-1".to_string()),
            event_kind: Some("conversation_state_update".to_string()),
            summary: Some("agent still making progress".to_string()),
        });

    scheduler
        .tick(ts(50))
        .await
        .expect("runtime event tick should succeed");
    let snapshot = scheduler.snapshot(ts(50));
    assert_eq!(snapshot.issues[0].runtime.stalled_at, Some(ts(150)));

    scheduler
        .tick(ts(120))
        .await
        .expect("pre-stall tick should succeed");
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-271").expect("issue id should be valid"))
            .expect("execution should exist")
            .status(),
        SchedulerStatus::Running
    );

    scheduler
        .tick(ts(160))
        .await
        .expect("stall tick should succeed");

    let execution = scheduler
        .execution(&IssueId::new("lin-271").expect("issue id should be valid"))
        .expect("execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(scheduler.worker().aborted[0].1, WorkerAbortReason::Stalled);
    assert_eq!(
        execution.retry().expect("retry should exist").reason,
        RetryReason::Stalled
    );
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-271-B"
    );
}

#[tokio::test]
async fn recovery_reuses_manifest_workspace_for_active_issue_dispatch() {
    let recovered_workspace = workspace_record("COE-272", "/tmp/recovered/COE-272");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-272", "COE-272", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-272", "COE-272", "In Progress"),
            workspace: recovered_workspace.clone(),
            had_in_flight_run: true,
        }],
        records: HashMap::from([("lin-272".to_string(), recovered_workspace.clone())]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("recovery tick should succeed");

    let issue_id = IssueId::new("lin-272").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should exist after recovery");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(
        execution
            .workspace()
            .expect("workspace should be attached")
            .path,
        recovered_workspace.path
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0].workspace.path,
        recovered_workspace.path
    );
    assert!(scheduler.workspace().cleaned.is_empty());
}

#[tokio::test]
async fn tracker_inactive_release_frees_the_per_state_slot() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-277", "COE-277", "In Progress", 0),
            tracker_issue("lin-278", "COE-278", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch the first issue");

    scheduler.tracker_mut().active = vec![tracker_issue("lin-278", "COE-278", "In Progress", 1)];
    scheduler.tracker_mut().states.insert(
        "lin-277".to_string(),
        tracker_state_snapshot("lin-277", "COE-277", "Todo", "unstarted", 200),
    );

    scheduler
        .tick(ts(200))
        .await
        .expect("inactive reconciliation should release and replace the running issue");

    let released = scheduler
        .execution(&IssueId::new("lin-277").expect("issue id should be valid"))
        .expect("released issue should still exist");
    assert_eq!(released.status(), SchedulerStatus::Released);
    match released.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerInactive);
        }
        other => panic!("expected released state, got {other:?}"),
    }
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::TrackerInactive
    );
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-278"
    );
}

#[tokio::test]
async fn running_count_follows_active_state_reconciliation() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-280", "COE-280", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_concurrent_agents = 3;
    config.stall_timeout_ms = None;
    config.active_states.push("Code Review".to_string());
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    config
        .max_concurrent_agents_by_state
        .insert("Code Review".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch the initial issue");

    scheduler.tracker_mut().active = vec![
        tracker_issue("lin-280", "COE-280", "Code Review", 0),
        tracker_issue("lin-281", "COE-281", "In Progress", 1),
        tracker_issue("lin-282", "COE-282", "Code Review", 2),
    ];

    scheduler
        .tick(ts(200))
        .await
        .expect("active-state reconciliation should update running counts");

    let refreshed = scheduler
        .execution(&IssueId::new("lin-280").expect("issue id should be valid"))
        .expect("original issue should still be running");
    assert_eq!(refreshed.status(), SchedulerStatus::Running);
    assert_eq!(refreshed.issue().state.name, "Code Review");
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-281"
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-282").expect("issue id should be valid"))
            .expect("reconciled active issue should exist")
            .status(),
        SchedulerStatus::Unclaimed
    );
}

#[tokio::test]
async fn recovery_does_not_count_released_issues_as_running_capacity() {
    let recovered_workspace = workspace_record("COE-283-A", "/tmp/recovered/COE-283-A");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-283-b", "COE-283-B", "In Progress", 1)],
        states: HashMap::from([(
            "lin-283-a".to_string(),
            tracker_state_snapshot("lin-283-a", "COE-283-A", "Todo", "unstarted", 100),
        )]),
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-283-a", "COE-283-A", "In Progress"),
            workspace: recovered_workspace,
            had_in_flight_run: true,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("recovery tick should not reserve running capacity for released issues");

    let recovered = scheduler
        .execution(&IssueId::new("lin-283-a").expect("issue id should be valid"))
        .expect("recovered issue should still exist");
    assert_eq!(recovered.status(), SchedulerStatus::Released);
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0].issue.identifier.as_str(),
        "COE-283-B"
    );
}

#[tokio::test]
async fn per_state_capacity_limits_dispatches_even_when_multiple_issues_are_ready() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-273", "COE-273", "In Progress", 0),
            tracker_issue("lin-274", "COE-274", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler.tick(ts(100)).await.expect("tick should succeed");

    assert_eq!(scheduler.worker().launches.len(), 1);
    let running = scheduler
        .executions()
        .values()
        .filter(|execution| execution.status() == SchedulerStatus::Running)
        .count();
    let unclaimed = scheduler
        .executions()
        .values()
        .filter(|execution| execution.status() == SchedulerStatus::Unclaimed)
        .count();
    assert_eq!(running, 1);
    assert_eq!(unclaimed, 1);
}

#[tokio::test]
async fn detached_outcome_does_not_schedule_retry() {
    // When a worker reports a Detached outcome (stop/cancel failed or unsupported),
    // the scheduler should NOT schedule a retry to avoid duplicating still-active work.
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-300", "COE-300", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None; // Disable stall timeout to isolate the test
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch");

    let issue_id = IssueId::new("lin-300").expect("issue id should be valid");
    assert_eq!(
        scheduler.worker().launches.len(),
        1,
        "should have one launch"
    );

    let running = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: running.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &running,
                WorkerOutcomeKind::Detached,
                ts(200),
                Some("underlying run could not be stopped".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("detached outcome tick should succeed");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");

    // Should be Released, not RetryQueued or Running
    assert_eq!(
        execution.status(),
        SchedulerStatus::Released,
        "detached outcome should release the execution"
    );
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerInactive);
        }
        other => panic!("expected released state, got {other:?}"),
    }

    // No retry should be scheduled
    assert!(execution.retry().is_none());
    // No new launches should have occurred
    assert_eq!(scheduler.worker().launches.len(), 1);
}

#[tokio::test]
async fn cancel_failed_outcome_does_not_schedule_retry() {
    // When a worker reports a CancelFailed outcome (cancel/stop was attempted but refused),
    // the scheduler should NOT schedule a retry to avoid duplicating still-active work.
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-301", "COE-301", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None; // Disable stall timeout to isolate the test
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch");

    let issue_id = IssueId::new("lin-301").expect("issue id should be valid");
    let running = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: running.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &running,
                WorkerOutcomeKind::CancelFailed,
                ts(200),
                Some("cancel/stop was refused by runtime".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("cancel-failed outcome tick should succeed");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");

    // Should be Released, not RetryQueued
    assert_eq!(execution.status(), SchedulerStatus::Released);
    // No retry should be scheduled
    assert!(execution.retry().is_none());
    // No new launches should have occurred
    assert_eq!(scheduler.worker().launches.len(), 1);
}

// --- LOC-13: `normalize_tracker_issue` wires `repo_for_issue` end-to-end ---

fn scheduler_config_with_inventory(inventory: BTreeMap<String, RepoRef>) -> SchedulerConfig {
    SchedulerConfig {
        poll_interval_ms: 1_000,
        max_concurrent_agents: 2,
        max_turns: 4,
        max_concurrent_agents_by_state: BTreeMap::new(),
        retry_policy: Default::default(),
        stall_timeout_ms: Some(100),
        active_states: vec!["In Progress".to_string()],
        terminal_states: vec!["Done".to_string(), "Canceled".to_string()],
        project_set_inventory: inventory,
    }
}

fn single_repo_inventory(slug: &str, url: &str) -> BTreeMap<String, RepoRef> {
    let mut map = BTreeMap::new();
    map.insert(
        slug.to_string(),
        RepoRef {
            url: url.to_string(),
            key: slug.to_string(),
            default_branch: Some("main".to_string()),
        },
    );
    map
}

#[tokio::test]
async fn poll_normalize_populates_execution_repo_ref_from_known_slug() {
    let tracker = FakeTracker {
        active: vec![tracker_issue_with_labels(
            "lin-repo-1",
            "COE-REPO-1",
            "In Progress",
            0,
            &["area:linear", "repo:test-repo"],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        worker,
        scheduler_config_with_inventory(single_repo_inventory(
            "test-repo",
            "https://example.com/test-repo.git",
        )),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-repo-1").expect("issue id should be valid");
    let execution_repo_ref = scheduler
        .execution(&issue_id)
        .expect("execution should exist")
        .issue()
        .execution_repo_ref
        .clone();
    assert_eq!(
        execution_repo_ref.expect("repo should resolve").key,
        "test-repo"
    );
}

#[tokio::test]
async fn poll_normalize_leaves_execution_repo_ref_none_for_unknown_slug() {
    let tracker = FakeTracker {
        active: vec![tracker_issue_with_labels(
            "lin-repo-2",
            "COE-REPO-2",
            "In Progress",
            0,
            &["repo:unknown-slug"],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        worker,
        scheduler_config_with_inventory(single_repo_inventory(
            "test-repo",
            "https://example.com/test-repo.git",
        )),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-repo-2").expect("issue id should be valid");
    let execution_repo_ref = scheduler
        .execution(&issue_id)
        .expect("execution should exist")
        .issue()
        .execution_repo_ref
        .clone();
    assert!(
        execution_repo_ref.is_none(),
        "unknown slug must not resolve"
    );
}

#[tokio::test]
async fn poll_normalize_leaves_execution_repo_ref_none_when_inventory_empty() {
    let tracker = FakeTracker {
        active: vec![tracker_issue_with_labels(
            "lin-repo-3",
            "COE-REPO-3",
            "In Progress",
            0,
            &["repo:test-repo"],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        worker,
        scheduler_config_with_inventory(BTreeMap::new()),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-repo-3").expect("issue id should be valid");
    let execution_repo_ref = scheduler
        .execution(&issue_id)
        .expect("execution should exist")
        .issue()
        .execution_repo_ref
        .clone();
    assert!(
        execution_repo_ref.is_none(),
        "empty inventory must not resolve"
    );
}

// --- LOC-14 dispatch-gate integration tests -------------------------------

fn assert_release_reason(
    execution: &crate::opensymphony_orchestrator::IssueExecution,
    expected: ReleaseReason,
) {
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, expected, "release reason mismatch");
        }
        other => panic!("expected Released state, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_gate_blocks_terminal_leaf_with_no_repo_label() {
    // AC1: terminal leaf without any `repo:` label is never dispatched
    // (no clone, no agent). The execution is released with `MissingRepo`.
    let tracker = FakeTracker {
        active: vec![tracker_issue_with_labels(
            "lin-missing-repo",
            "COE-MISSING-REPO",
            "In Progress",
            0,
            &[],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        worker,
        scheduler_config_with_inventory(default_repo_inventory()),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-missing-repo").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist after gate fires");

    // Operator-visible release reason.
    assert_release_reason(execution, ReleaseReason::MissingRepo);

    // Gate fired BEFORE ensure_workspace — no workspace should have been
    // attached and no worker should have been launched.
    assert_eq!(
        scheduler.workspace().ensured,
        Vec::<String>::new(),
        "gate must skip ensure_workspace"
    );
    assert_eq!(
        scheduler.worker().launches.len(),
        0,
        "gate must skip worker launch"
    );
    assert_eq!(
        scheduler.workspace().cleaned,
        Vec::<(String, bool)>::new(),
        "no workspace cleanup should happen"
    );
}

#[tokio::test]
async fn dispatch_gate_releases_terminal_leaf_with_multiple_repo_labels() {
    // AC3: a terminal leaf with multiple `repo:` labels is treated as
    // unresolved and blocked — the repo resolver returns None (D5).
    let tracker = FakeTracker {
        active: vec![tracker_issue_with_labels(
            "lin-multi-repo",
            "COE-MULTI-REPO",
            "In Progress",
            0,
            &["repo:alpha", "repo:beta"],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config =
        scheduler_config_with_inventory(default_repo_inventory_with(&[("alpha", "alpha.git")]));
    // Use `beta` so the resolver also has a single-label match available,
    // proving the multi-label case is what trips the gate.
    let _ = config
        .project_set_inventory
        .insert("beta".to_string(), test_repo_ref("beta", "beta.git"));
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-multi-repo").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist after gate fires");

    assert_release_reason(execution, ReleaseReason::MissingRepo);
    assert_eq!(
        scheduler.workspace().ensured,
        Vec::<String>::new(),
        "multi-repo labels must be treated as unresolved"
    );
    assert_eq!(scheduler.worker().launches.len(), 0);
}

#[tokio::test]
async fn dispatch_gate_releases_terminal_leaf_with_unknown_repo_slug() {
    // D6: a `repo:` label whose slug is absent from the project-set
    // inventory is also unresolved. Same operator-visible reason.
    let tracker = FakeTracker {
        active: vec![tracker_issue_with_labels(
            "lin-unknown-slug",
            "COE-UNKNOWN-SLUG",
            "In Progress",
            0,
            &["repo:unknown-slug"],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        worker,
        scheduler_config_with_inventory(default_repo_inventory()),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-unknown-slug").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist after gate fires");

    assert_release_reason(execution, ReleaseReason::MissingRepo);
    assert_eq!(scheduler.worker().launches.len(), 0);
}

#[tokio::test]
async fn dispatch_gate_skips_parent_with_all_terminal_children() {
    // AC4: a parent whose children are all terminal does NOT enter the
    // work-clone path. The execution is released with `ParentDeferred`.
    let tracker = FakeTracker {
        active: vec![tracker_parent_with_children(
            "lin-parent",
            "COE-PARENT",
            "In Progress",
            0,
            &[],
            &[("COE-CHILD-A", "Done"), ("COE-CHILD-B", "Canceled")],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-parent").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist after gate fires");

    // Operator-visible deferral reason.
    assert_release_reason(execution, ReleaseReason::ParentDeferred);

    // Gate fired BEFORE ensure_workspace — no workspace, no worker.
    assert_eq!(scheduler.workspace().ensured, Vec::<String>::new());
    assert_eq!(scheduler.worker().launches.len(), 0);
}

#[tokio::test]
async fn dispatch_gate_skips_parent_with_accidental_repo_label() {
    // AC5: a parent that accidentally carries `repo:` labels still does
    // not enter the work-clone path. The repo resolver already returns
    // None for parents (D3); the gate then emits `ParentDeferred`
    // rather than `MissingRepo` so operators see the right reason.
    let tracker = FakeTracker {
        active: vec![tracker_parent_with_children(
            "lin-parent-repo",
            "COE-PARENT-REPO",
            "In Progress",
            0,
            &["repo:test-repo"],
            &[("COE-CHILD-A", "Done")],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-parent-repo").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist after gate fires");

    assert_release_reason(execution, ReleaseReason::ParentDeferred);
    assert_eq!(scheduler.workspace().ensured, Vec::<String>::new());
    assert_eq!(scheduler.worker().launches.len(), 0);

    // Sanity: parent's `execution_repo_ref` is None because the resolver
    // ignores `repo:` on parents. The gate therefore cannot have fired
    // the missing-repo branch.
    assert!(
        execution.issue().execution_repo_ref.is_none(),
        "parent must not carry a repo, even with stray labels"
    );
}

#[tokio::test]
async fn dispatch_gate_releases_terminal_leaf_as_missing_repo_with_inventory_present() {
    // AC2 sanity: with a single resolvable `repo:` label and a matching
    // inventory entry, dispatch proceeds normally (no gate fires).
    let tracker = FakeTracker {
        active: vec![tracker_issue_with_labels(
            "lin-good-repo",
            "COE-GOOD-REPO",
            "In Progress",
            0,
            &["repo:test-repo"],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-good-repo").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should exist");

    // Dispatched normally — Running, not Released.
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.workspace().ensured,
        vec!["COE-GOOD-REPO".to_string()]
    );
}

#[tokio::test]
async fn missing_repo_release_preserves_reactivation_state() {
    // AC7: after a MissingRepo release, adding a valid `repo:` label
    // and re-polling must reopen the execution so it can dispatch.
    let tracker = FakeTracker {
        active: vec![tracker_issue_with_labels(
            "lin-recover",
            "COE-RECOVER",
            "In Progress",
            0,
            &[],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        worker,
        scheduler_config_with_inventory(default_repo_inventory()),
    );

    // Tick 1: gate fires, execution released with MissingRepo.
    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");
    let issue_id = IssueId::new("lin-recover").expect("issue id should be valid");
    assert_release_reason(
        scheduler
            .execution(&issue_id)
            .expect("execution should exist"),
        ReleaseReason::MissingRepo,
    );

    // Tick 2: operator adds a resolvable `repo:` label. The tracker
    // snapshot now reports the updated labels. The reconciliation path
    // refreshes the issue data and reopens the execution because the
    // release reason preserves reactivation state.
    scheduler.tracker_mut().active = vec![tracker_issue_with_labels(
        "lin-recover",
        "COE-RECOVER",
        "In Progress",
        100,
        &["repo:test-repo"],
    )];
    scheduler
        .tick(ts(200))
        .await
        .expect("second tick should reactivate and dispatch");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");
    assert_eq!(
        execution.status(),
        SchedulerStatus::Running,
        "execution should reopen and dispatch once a resolvable repo label is present"
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.workspace().ensured,
        vec!["COE-RECOVER".to_string()]
    );
}

#[tokio::test]
async fn parent_deferred_release_does_not_preserve_reactivation_state() {
    // AC7: once a parent is deferred, it stays deferred even if the
    // tracker state oscillates. `ParentDeferred` is sticky because
    // parents are always review nodes in Phase 1.
    let tracker = FakeTracker {
        active: vec![tracker_parent_with_children(
            "lin-parent-sticky",
            "COE-PARENT-STICKY",
            "In Progress",
            0,
            &[],
            &[("COE-CHILD-A", "Done")],
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should defer the parent");
    let issue_id = IssueId::new("lin-parent-sticky").expect("issue id should be valid");
    assert_release_reason(
        scheduler
            .execution(&issue_id)
            .expect("execution should exist"),
        ReleaseReason::ParentDeferred,
    );

    // Tick 2: tracker still shows the same active parent with terminal
    // children. Reconciliation may reopen it; the dispatch gate then
    // fires again and re-releases it as ParentDeferred. The execution
    // must never reach the work-clone path.
    scheduler
        .tick(ts(200))
        .await
        .expect("second tick should keep the parent deferred");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");
    assert_release_reason(execution, ReleaseReason::ParentDeferred);
    assert_eq!(
        scheduler.worker().launches.len(),
        0,
        "parent must never launch a worker"
    );
    assert_eq!(scheduler.workspace().ensured, Vec::<String>::new());
}

// --- helpers used by LOC-14 tests ---------------------------------------

fn test_repo_ref(slug: &str, url_suffix: &str) -> RepoRef {
    RepoRef {
        url: format!("https://example.com/{url_suffix}"),
        key: slug.to_string(),
        default_branch: Some("main".to_string()),
    }
}

fn default_repo_inventory_with(extra: &[(&str, &str)]) -> BTreeMap<String, RepoRef> {
    let mut map = default_repo_inventory();
    for (slug, suffix) in extra {
        map.insert((*slug).to_string(), test_repo_ref(slug, suffix));
    }
    map
}
