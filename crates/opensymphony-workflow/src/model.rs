use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

pub const DEFAULT_PROMPT_TEMPLATE: &str = "You are working on an issue from Linear.";
pub const DEFAULT_LINEAR_ENDPOINT: &str = "https://api.linear.app/graphql";
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 30_000;
pub const DEFAULT_WORKSPACE_ROOT: &str = "/symphony_workspaces";
pub const DEFAULT_HOOK_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_MAX_CONCURRENT_AGENTS: u64 = 10;
pub const DEFAULT_MAX_TURNS: u64 = 20;
pub const DEFAULT_MAX_RETRY_BACKOFF_MS: u64 = 300_000;
pub const DEFAULT_STALL_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_OPENHANDS_BASE_URL: &str = "http://127.0.0.1:8000";
pub const DEFAULT_OPENHANDS_STARTUP_TIMEOUT_MS: u64 = 180_000;
pub const DEFAULT_OPENHANDS_READINESS_PROBE_PATH: &str = "/openapi.json";
pub const DEFAULT_OPENHANDS_PERSISTENCE_DIR: &str = ".opensymphony/openhands";
pub const DEFAULT_OPENHANDS_MAX_ITERATIONS: u64 = 500;
pub const DEFAULT_OPENHANDS_CONFIRMATION_POLICY_KIND: &str = "NeverConfirm";
pub const DEFAULT_OPENHANDS_AGENT_KIND: &str = "Agent";
pub const DEFAULT_OPENHANDS_AGENT_TOOLS: &[&str] = &["terminal", "file_editor"];
pub const DEFAULT_OPENHANDS_READY_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_OPENHANDS_RECONNECT_INITIAL_MS: u64 = 1_000;
pub const DEFAULT_OPENHANDS_RECONNECT_MAX_MS: u64 = 30_000;
pub const DEFAULT_OPENHANDS_AUTH_MODE: &str = "auto";
pub const DEFAULT_OPENHANDS_QUERY_PARAM_NAME: &str = "session_api_key";
pub const DEFAULT_OPENHANDS_LLM_MODEL: &str = "openai/gpt-5.4";
pub const DEFAULT_OPENHANDS_CONDENSER_MAX_SIZE: u64 = 240;
pub const DEFAULT_OPENHANDS_CONDENSER_KEEP_FIRST: u64 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowDefinition {
    pub front_matter: WorkflowFrontMatter,
    pub prompt_template: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct WorkflowFrontMatter {
    #[serde(default)]
    pub tracker: TrackerFrontMatter,
    #[serde(default)]
    pub polling: PollingFrontMatter,
    #[serde(default)]
    pub workspace: WorkspaceFrontMatter,
    #[serde(default)]
    pub hooks: HooksFrontMatter,
    #[serde(default)]
    pub agent: AgentFrontMatter,
    #[serde(default)]
    pub openhands: OpenHandsFrontMatter,
    #[serde(default)]
    pub codex: Option<BTreeMap<String, serde_yaml::Value>>,
    #[serde(default)]
    pub logging: Option<BTreeMap<String, serde_yaml::Value>>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrackerFrontMatter {
    pub kind: Option<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub project_slug: Option<String>,
    pub active_states: Option<Vec<String>>,
    pub terminal_states: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PollingFrontMatter {
    pub interval_ms: Option<IntegerLike>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFrontMatter {
    pub root: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HooksFrontMatter {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    pub timeout_ms: Option<IntegerLike>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentFrontMatter {
    pub max_concurrent_agents: Option<IntegerLike>,
    pub max_turns: Option<IntegerLike>,
    pub max_retry_backoff_ms: Option<IntegerLike>,
    pub stall_timeout_ms: Option<IntegerLike>,
    pub max_concurrent_agents_by_state: Option<BTreeMap<String, IntegerLike>>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsFrontMatter {
    #[serde(default)]
    pub transport: OpenHandsTransportFrontMatter,
    #[serde(default)]
    pub local_server: OpenHandsLocalServerFrontMatter,
    #[serde(default)]
    pub conversation: OpenHandsConversationFrontMatter,
    #[serde(default)]
    pub websocket: OpenHandsWebSocketFrontMatter,
    #[serde(default, rename = "mcp")]
    pub legacy_linear_bridge: Option<serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsTransportFrontMatter {
    pub base_url: Option<String>,
    pub session_api_key_env: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsLocalServerFrontMatter {
    pub enabled: Option<bool>,
    pub command: Option<Vec<String>>,
    pub startup_timeout_ms: Option<IntegerLike>,
    pub readiness_probe_path: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsConversationFrontMatter {
    pub reuse_policy: Option<String>,
    pub persistence_dir_relative: Option<String>,
    pub max_iterations: Option<IntegerLike>,
    pub stuck_detection: Option<bool>,
    pub confirmation_policy: Option<OpenHandsConfirmationPolicyFrontMatter>,
    pub agent: Option<OpenHandsConversationAgentFrontMatter>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct OpenHandsConfirmationPolicyFrontMatter {
    pub kind: Option<String>,
    #[serde(flatten)]
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OpenHandsConfirmationPolicy {
    pub kind: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct OpenHandsConversationAgentFrontMatter {
    pub kind: Option<String>,
    pub llm: Option<OpenHandsLlmFrontMatter>,
    pub condenser: Option<OpenHandsConversationCondenserFrontMatter>,
    pub tools: Option<Vec<OpenHandsConversationToolFrontMatter>>,
    pub include_default_tools: Option<Vec<String>>,
    pub log_completions: Option<bool>,
    #[serde(flatten)]
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsConversationToolFrontMatter {
    pub name: String,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsConversationCondenserFrontMatter {
    pub enabled: Option<bool>,
    pub max_size: Option<IntegerLike>,
    pub keep_first: Option<IntegerLike>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct OpenHandsLlmFrontMatter {
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url_env: Option<String>,
    #[serde(flatten)]
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsWebSocketFrontMatter {
    pub enabled: Option<bool>,
    pub ready_timeout_ms: Option<IntegerLike>,
    pub reconnect_initial_ms: Option<IntegerLike>,
    pub reconnect_max_ms: Option<IntegerLike>,
    pub auth_mode: Option<String>,
    pub query_param_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum IntegerLike {
    Integer(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedWorkflow {
    pub config: WorkflowConfig,
    pub extensions: WorkflowExtensions,
    pub prompt_template: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowConfig {
    pub tracker: TrackerConfig,
    pub polling: PollingConfig,
    pub workspace: WorkspaceConfig,
    pub hooks: HooksConfig,
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExtensions {
    pub openhands: OpenHandsConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerKind {
    Linear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerConfig {
    pub kind: TrackerKind,
    pub endpoint: String,
    pub api_key: String,
    pub project_slug: String,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingConfig {
    pub interval_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceConfig {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksConfig {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub max_concurrent_agents: u64,
    pub max_turns: u64,
    pub max_retry_backoff_ms: u64,
    pub stall_timeout_ms: Option<u64>,
    pub max_concurrent_agents_by_state: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenHandsConfig {
    pub transport: OpenHandsTransportConfig,
    pub local_server: OpenHandsLocalServerConfig,
    pub conversation: OpenHandsConversationConfig,
    pub websocket: OpenHandsWebSocketConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsTransportConfig {
    pub base_url: String,
    pub session_api_key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsLocalServerConfig {
    pub enabled: bool,
    pub command: Option<Vec<String>>,
    pub startup_timeout_ms: u64,
    pub readiness_probe_path: String,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenHandsConversationConfig {
    pub reuse_policy: String,
    pub persistence_dir_relative: PathBuf,
    pub max_iterations: u64,
    pub stuck_detection: bool,
    pub confirmation_policy: OpenHandsConfirmationPolicy,
    pub agent: OpenHandsConversationAgentConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenHandsConversationAgentConfig {
    pub kind: String,
    pub llm: Option<OpenHandsLlmConfig>,
    pub condenser: Option<OpenHandsConversationCondenserConfig>,
    pub tools: Option<Vec<OpenHandsConversationToolConfig>>,
    pub include_default_tools: Option<Vec<String>>,
    pub log_completions: bool,
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsConversationToolConfig {
    pub name: String,
    pub params: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsConversationCondenserConfig {
    pub max_size: u64,
    pub keep_first: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenHandsLlmConfig {
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url_env: Option<String>,
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsWebSocketConfig {
    pub enabled: bool,
    pub ready_timeout_ms: u64,
    pub reconnect_initial_ms: u64,
    pub reconnect_max_ms: u64,
    pub auth_mode: String,
    pub query_param_name: String,
}

pub trait Environment {
    fn get(&self, name: &str) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
    }
}

impl Environment for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

impl Environment for HashMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptContext<'a, T>
where
    T: Serialize,
{
    pub issue: &'a T,
    pub attempt: Option<u32>,
}

// ---------------------------------------------------------------------------
// Project-set config (`.opensymphony/project-set.yaml`)
// ---------------------------------------------------------------------------

pub const PROJECT_SET_SCHEMA_VERSION: u64 = 1;

/// Raw deserialized shape of `.opensymphony/project-set.yaml`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ProjectSetFrontMatter {
    pub schema_version: Option<u64>,
    #[serde(default)]
    pub project_set: ProjectSetBody,
}

/// Top-level `project_set:` mapping.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ProjectSetBody {
    pub slug: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub linear: ProjectSetLinearFrontMatter,
    #[serde(default)]
    pub polling: ProjectSetPollingFrontMatter,
    #[serde(default)]
    pub agent: ProjectSetAgentFrontMatter,
    #[serde(default)]
    pub projects: Vec<ProjectEntry>,
}

/// Per-project entry inside `projects:`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ProjectEntry {
    pub slug: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub repos: Vec<RepoEntry>,
}

/// Per-repo entry inside a project.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct RepoEntry {
    pub slug: Option<String>,
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// Optional local path metadata (NOT part of RepoRef).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Linear tracker configuration at the project-set level.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct ProjectSetLinearFrontMatter {
    pub endpoint: Option<String>,
    pub project_slug: Option<String>,
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub active_states: Option<Vec<String>>,
    #[serde(default)]
    pub terminal_states: Option<Vec<String>>,
}

/// Polling configuration at the project-set level.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct ProjectSetPollingFrontMatter {
    pub interval_ms: Option<IntegerLike>,
}

/// Agent configuration at the project-set level.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct ProjectSetAgentFrontMatter {
    pub max_concurrent_agents: Option<IntegerLike>,
}

/// A field that has moved out of `WORKFLOW.md` into
/// `.opensymphony/project-set.yaml` (LOC-18).
///
/// In project-set mode these fields are no longer live runtime inputs from the
/// repo workflow; their presence in `WORKFLOW.md` is stale config that must
/// be migrated away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleMovedField {
    /// Dotted field path inside `WORKFLOW.md` front matter.
    pub field: &'static str,
    /// Diagnostic destination for migration (project-set path / env key).
    pub destination: &'static str,
}

/// One or more `WORKFLOW.md` fields are stale in project-set mode (LOC-18).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaleMovedProjectSetFields {
    pub fields: Vec<(String, String)>,
}

impl std::fmt::Display for StaleMovedProjectSetFields {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.fields.is_empty() {
            return formatter.write_str("<none>");
        }
        let mut first = true;
        for (field, destination) in &self.fields {
            if !first {
                formatter.write_str(", ")?;
            }
            first = false;
            write!(formatter, "{field} -> {destination}")?;
        }
        Ok(())
    }
}

/// Canonical list of `WORKFLOW.md` fields that move to `.opensymphony/project-set.yaml`.
///
/// Keep in sync with the `TrackerFrontMatter`, `PollingFrontMatter`, and
/// `AgentFrontMatter` fields that `ProjectSetConfig` owns in strict project-set
/// mode. Adding a new entry requires extending [`detect_stale_project_set_fields`].
pub const STALE_MOVED_FIELDS: &[StaleMovedField] = &[
    StaleMovedField {
        field: "tracker.kind",
        destination: "project_set.linear (kind implied: linear)",
    },
    StaleMovedField {
        field: "tracker.endpoint",
        destination: "project_set.linear.endpoint",
    },
    StaleMovedField {
        field: "tracker.project_slug",
        destination: "project_set.linear.project_slug",
    },
    StaleMovedField {
        field: "tracker.api_key",
        destination: "project_set.linear.api_key_env",
    },
    StaleMovedField {
        field: "tracker.active_states",
        destination: "project_set.linear.active_states",
    },
    StaleMovedField {
        field: "tracker.terminal_states",
        destination: "project_set.linear.terminal_states",
    },
    StaleMovedField {
        field: "polling.interval_ms",
        destination: "project_set.polling.interval_ms",
    },
    StaleMovedField {
        field: "agent.max_concurrent_agents",
        destination: "project_set.agent.max_concurrent_agents",
    },
];

/// Resolved project-set config produced by [`resolve_project_set`].
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProjectSet {
    pub config: ProjectSetConfig,
    /// Flat slug → RepoRef lookup for the resolver (LOC-13).
    inventory: BTreeMap<String, crate::opensymphony_domain::RepoRef>,
}

impl ResolvedProjectSet {
    pub(crate) fn new(
        config: ProjectSetConfig,
        inventory: BTreeMap<String, crate::opensymphony_domain::RepoRef>,
    ) -> Self {
        Self { config, inventory }
    }

    /// Returns a reference to the inventory map (slug → RepoRef).
    pub fn inventory(&self) -> &BTreeMap<String, crate::opensymphony_domain::RepoRef> {
        &self.inventory
    }

    /// Looks up a single repo by slug.
    pub fn repo_by_slug(&self, slug: &str) -> Option<&crate::opensymphony_domain::RepoRef> {
        self.inventory.get(slug)
    }
}

/// Resolved project-set configuration values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSetConfig {
    pub schema_version: u64,
    pub slug: String,
    pub name: String,
    pub linear: ProjectSetLinearConfig,
    pub polling: ProjectSetPollingConfig,
    pub agent: ProjectSetAgentConfig,
    pub projects: Vec<ResolvedProject>,
}

/// Resolved linear tracker config at project-set level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSetLinearConfig {
    pub endpoint: String,
    pub project_slug: String,
    /// Name of the env var that supplied the api key (defaults to
    /// `LINEAR_API_KEY` when `api_key_env` is unset). Preserved so doctor and
    /// operator-facing diagnostics can point operators at the right env var
    /// (LOC-18).
    pub api_key_env: String,
    pub api_key: String,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
}

/// Resolved polling config at project-set level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectSetPollingConfig {
    pub interval_ms: u64,
}

/// Resolved agent config at project-set level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectSetAgentConfig {
    pub max_concurrent_agents: u64,
}

/// A resolved project entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProject {
    pub slug: String,
    pub name: String,
    pub repos: Vec<ResolvedRepoEntry>,
}

/// A resolved repo entry (url + default_branch mapped, path kept as metadata).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRepoEntry {
    pub slug: String,
    pub url: String,
    pub default_branch: Option<String>,
    /// Optional local path metadata (not part of RepoRef).
    pub path: Option<String>,
}
