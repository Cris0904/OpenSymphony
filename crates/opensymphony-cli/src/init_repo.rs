use std::{
    collections::BTreeSet,
    env, fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::{ExitCode, Output},
    time::Duration,
};

use super::memory_init_summary::record_memory_init_changes;
use super::project_set_writer::ProjectSetAppliedOutcome;
use super::repo_detection::{
    GitRemoteDetection, derive_repo_slug_from_dir, derive_repo_slug_from_remote,
    detect_git_default_branch, detect_git_remote_url,
};
use super::util::trimmed_non_empty;
// `git_remote_url` and `git_remote_name` were the `init`-private free
// functions; their equivalents now live as inherent methods on
// [`GitRemoteDetection`] in [`super::repo_detection`] (LOC-19).
use crate::opensymphony_memory::ensure_memory_initialized;
use clap::{Args, ValueEnum};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use thiserror::Error;

const DEFAULT_TEMPLATE_BASE_URL: &str =
    "https://raw.githubusercontent.com/kumanday/OpenSymphony-template/refs/heads/main/";
const DEFAULT_TEMPLATE_TREE_URL: &str =
    "https://api.github.com/repos/kumanday/OpenSymphony-template/git/trees/main?recursive=1";
const DEFAULT_TEMPLATE_FETCH_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_LLM_MODEL: &str = "openai/accounts/fireworks/models/glm-5p1";
const DEFAULT_LLM_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
const DEFAULT_AI_REVIEW_PROVIDER_KIND: &str = "openai-compatible";
const DEFAULT_AI_REVIEW_MODEL_ID: &str = "accounts/fireworks/models/glm-5p1";
const DEFAULT_AI_REVIEW_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
const DEFAULT_AI_REVIEW_STYLE: &str = "standard";
const DEFAULT_AI_REVIEW_REQUIRE_EVIDENCE: &str = "true";
const DEFAULT_AI_REVIEW_SECRET_NAME: &str = "AI_REVIEW_API_KEY";
const AI_REVIEW_LABEL_DESCRIPTION: &str = "Trigger AI PR review";
const OPENHANDS_PR_REVIEW_PLUGIN_URL: &str =
    "https://github.com/OpenHands/extensions/tree/main/plugins/pr-review";
const OPENHANDS_PR_REVIEW_DOCS_URL: &str =
    "https://docs.openhands.dev/sdk/guides/github-workflows/pr-review";
const OPENHANDS_PR_REVIEW_SETUP_GUIDE_URL: &str =
    "https://github.com/kumanday/OpenSymphony/blob/main/docs/ai-pr-review-human-setup.md";
const AI_REVIEW_LABEL_NAME: &str = "review-this";
const AGENTS_EXAMPLE_PATH: &str = "AGENTS-example.md";

/// Default repo slug used when neither the `--repo-slug` flag, the git
/// remote URL, nor the directory name produces a usable slug. Phase-1
/// `init` is single-project, so a literal sentinel keeps the inventory
/// commit-ready. Operators who onboard a second repo under a different
/// project-set slug should rerun `init --repo-slug <slug>` to override
/// before committing the project-set file (LOC-19 AI review feedback on
/// the silent `DEFAULT_REPO_SLUG_FALLBACK` usage).
const DEFAULT_REPO_SLUG_FALLBACK: &str = "repo";

/// Sentinel for the project-set slug when no `--project-set-slug` flag
/// was supplied. Phase-1 `init` writes a one-repo project-set so a
/// fixed, recognisable literal is preferable to deriving from
/// `repo_slug` (which would silently couple the inventory to whatever
/// slug the operator happened to land on). Operators who onboard a
/// second repo under a different project-set slug should rerun
/// `init --project-set-slug <slug>` to rename the file before
/// committing it (LOC-19 AI review feedback on the literal
/// `DEFAULT_PROJECT_SET_SLUG_FALLBACK`).
const DEFAULT_PROJECT_SET_SLUG_FALLBACK: &str = "default-project-set";

/// Default Linear `api_key_env` for fresh project-set bootstrap. Operators can
/// override via flags or by editing the generated `project-set.yaml` after init.
const DEFAULT_LINEAR_API_KEY_ENV: &str = "LINEAR_API_KEY";

#[derive(Debug, Args, Clone, Default)]
pub struct InitArgs {
    #[arg(
        long,
        help = "Run without interactive prompts; missing required choices fail fast"
    )]
    non_interactive: bool,
    #[arg(
        long,
        help = "Scaffold automated OpenHands AI PR review without prompting"
    )]
    ai_pr_review: bool,
    #[arg(
        long,
        value_name = "SLUG",
        help = "Linear project slug/key to write into \
                `<cwd>/.opensymphony/project-set.yaml` (`project_set.linear.project_slug`)"
    )]
    linear_project_slug: Option<String>,
    #[arg(
        long,
        value_name = "URL",
        help = "Override the git remote URL used for the project-set inventory entry \
                (`slug -> { url, default_branch }`). When omitted, init probes the local \
                git remote; in non-interactive mode a missing/ambiguous remote fails fast."
    )]
    repo_url: Option<String>,
    #[arg(
        long,
        value_name = "SLUG",
        help = "Override the repo slug used for the project-set inventory entry. \
                Defaults to the last path segment of the detected remote URL, \
                falling back to the directory name when no remote is available."
    )]
    repo_slug: Option<String>,
    #[arg(
        long,
        value_name = "BRANCH",
        help = "Override the default branch recorded in the project-set inventory entry. \
                Defaults to the symbolic ref `refs/remotes/<remote>/HEAD` when it can be \
                determined; omitted otherwise."
    )]
    repo_default_branch: Option<String>,
    #[arg(
        long,
        value_enum,
        value_name = "POLICY",
        help = "Apply this policy for existing generated files instead of prompting"
    )]
    conflict_policy: Option<InitConflictPolicy>,
    #[arg(
        long,
        help = "Commit and push generated bootstrap files when git preflight allows it"
    )]
    commit_and_push: bool,
    #[arg(
        long,
        help = "Configure GitHub Actions variables, secret, and review label with gh"
    )]
    configure_github: bool,
    #[arg(
        long,
        value_enum,
        value_name = "KIND",
        help = "AI review provider kind for scaffolded PR review"
    )]
    ai_review_provider_kind: Option<AiReviewProviderKindArg>,
    #[arg(long, value_name = "MODEL", help = "AI review model id")]
    ai_review_model_id: Option<String>,
    #[arg(
        long,
        value_name = "URL",
        help = "AI review base URL; only valid with --ai-review-provider-kind openai-compatible"
    )]
    ai_review_base_url: Option<String>,
    #[arg(long, value_name = "STYLE", help = "AI review style")]
    ai_review_style: Option<String>,
    #[arg(
        long,
        value_name = "BOOL",
        help = "Whether AI review findings should require evidence"
    )]
    ai_review_require_evidence: Option<bool>,
    #[arg(
        long,
        value_name = "ENV_VAR",
        help = "Read the AI review GitHub secret value from this environment variable"
    )]
    ai_review_secret_env: Option<String>,
    #[arg(
        long,
        help = "Reuse LLM_API_KEY as the AI review GitHub secret when configuring gh"
    )]
    reuse_llm_api_key_for_ai_review_secret: bool,
    #[arg(
        long,
        help = "Leave the AI review GitHub secret unchanged when configuring gh"
    )]
    skip_ai_review_secret: bool,
    #[arg(
        long,
        value_name = "MODEL",
        help = "Model shown in the LLM_MODEL export snippet when LLM_MODEL is unset"
    )]
    llm_model: Option<String>,
    #[arg(
        long,
        value_name = "PLACEHOLDER",
        help = "Placeholder shown in the LLM_API_KEY export snippet when LLM_API_KEY is unset"
    )]
    llm_api_key_placeholder: Option<String>,
    #[arg(
        long,
        value_name = "URL",
        help = "Base URL shown in the LLM_BASE_URL export snippet when LLM_BASE_URL is unset"
    )]
    llm_base_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum InitConflictPolicy {
    Skip,
    Overwrite,
    Abort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum AiReviewProviderKindArg {
    OpenaiCompatible,
    LitellmNative,
}

impl AiReviewProviderKindArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::OpenaiCompatible => "openai-compatible",
            Self::LitellmNative => "litellm-native",
        }
    }
}

impl InitArgs {
    fn validate(&self) -> Result<(), InitCommandError> {
        let secret_sources = [
            self.ai_review_secret_env.is_some(),
            self.reuse_llm_api_key_for_ai_review_secret,
            self.skip_ai_review_secret,
        ]
        .into_iter()
        .filter(|enabled| *enabled)
        .count();
        if secret_sources > 1 {
            return Err(InitCommandError::InvalidArgument(
                "choose only one of --ai-review-secret-env, --reuse-llm-api-key-for-ai-review-secret, or --skip-ai-review-secret"
                    .to_string(),
            ));
        }
        if matches!(
            self.ai_review_provider_kind,
            Some(AiReviewProviderKindArg::LitellmNative)
        ) && trimmed_non_empty(self.ai_review_base_url.as_deref()).is_some()
        {
            return Err(InitCommandError::InvalidArgument(
                "--ai-review-base-url can only be used with --ai-review-provider-kind openai-compatible"
                    .to_string(),
            ));
        }

        Ok(())
    }

    fn ai_pr_review_requested_by_flags(&self) -> bool {
        self.ai_pr_review
            || self.configure_github
            || self.ai_review_config_plan_from_flags().requested_by_flags
            || self.ai_review_secret_flags_present()
    }

    fn ai_review_secret_flags_present(&self) -> bool {
        self.ai_review_secret_env.is_some()
            || self.reuse_llm_api_key_for_ai_review_secret
            || self.skip_ai_review_secret
    }

    fn ai_review_config_from_flags(&self) -> AiReviewConfig {
        self.ai_review_config_plan_from_flags().config
    }

    fn ai_review_config_plan_from_flags(&self) -> AiReviewConfigPlan {
        let provider_kind_override = self
            .ai_review_provider_kind
            .map(AiReviewProviderKindArg::as_str);
        let model_id_override = trimmed_non_empty(self.ai_review_model_id.as_deref());
        let base_url_override = trimmed_non_empty(self.ai_review_base_url.as_deref());
        let style_override = trimmed_non_empty(self.ai_review_style.as_deref());

        let provider_kind = provider_kind_override
            .unwrap_or(DEFAULT_AI_REVIEW_PROVIDER_KIND)
            .to_string();
        let base_url = if provider_kind == "openai-compatible" {
            Some(base_url_override.unwrap_or_else(|| DEFAULT_AI_REVIEW_BASE_URL.to_string()))
        } else {
            None
        };

        AiReviewConfigPlan {
            config: AiReviewConfig {
                provider_kind,
                model_id: model_id_override
                    .unwrap_or_else(|| DEFAULT_AI_REVIEW_MODEL_ID.to_string()),
                base_url,
                style: style_override.unwrap_or_else(|| DEFAULT_AI_REVIEW_STYLE.to_string()),
                require_evidence: self
                    .ai_review_require_evidence
                    .unwrap_or(DEFAULT_AI_REVIEW_REQUIRE_EVIDENCE == "true"),
            },
            requested_by_flags: provider_kind_override.is_some()
                || self.ai_review_model_id.is_some()
                || self.ai_review_base_url.is_some()
                || self.ai_review_style.is_some()
                || self.ai_review_require_evidence.is_some(),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum InitCommandError {
    #[error("failed to determine the current working directory: {0}")]
    CurrentDir(#[source] io::Error),
    #[error("failed to build the template fetch client: {0}")]
    HttpClient(#[source] reqwest::Error),
    #[error("invalid template base URL `{value}`: {source}")]
    InvalidTemplateBaseUrl {
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error("failed to fetch template asset {path} from {url}: {source}")]
    FetchTemplate {
        path: String,
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to fetch template asset {path} from {url}: HTTP {status}")]
    FetchTemplateStatus {
        path: String,
        url: String,
        status: StatusCode,
    },
    #[error("template asset {path} from {url} was not valid UTF-8: {source}")]
    DecodeTemplate {
        path: String,
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to fetch template tree from {url}: {source}")]
    FetchTemplateTree {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to fetch template tree from {url}: HTTP {status}")]
    FetchTemplateTreeStatus { url: String, status: StatusCode },
    #[error("template tree from {url} was not valid JSON: {source}")]
    DecodeTemplateTree {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("template directory {path} had no files in tree {url}")]
    MissingTemplateDirectory { path: &'static str, url: String },
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to create {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read interactive input: {0}")]
    PromptIo(#[source] io::Error),
    #[error("input closed while waiting for a response")]
    PromptClosed,
    #[error(
        "non-interactive init cannot prompt for `{prompt}`; pass the relevant init flag or run without --non-interactive"
    )]
    NonInteractivePrompt { prompt: String },
    #[error(
        "non-interactive init requires {decision}; pass {flag} or run without --non-interactive"
    )]
    NonInteractiveMissing {
        decision: String,
        flag: &'static str,
    },
    #[error("missing environment variable `{name}` required by {flag}")]
    MissingEnvVar { name: String, flag: &'static str },
    #[error("invalid init argument: {0}")]
    InvalidArgument(String),
    #[error("initialization aborted")]
    AbortedByUser,
    #[error("initialization aborted by --conflict-policy abort for existing `{path}`")]
    ConflictPolicyAbort { path: String },
    #[error("failed to initialize project memory: {0}")]
    MemoryInit(#[from] crate::opensymphony_memory::MemoryError),
    #[error(transparent)]
    ProjectSetUpsert(#[from] super::project_set_writer::ProjectSetUpsertError),
    #[error("could not register the repo in the project-set inventory: {guidance}")]
    ProjectSetRemoteMissing { guidance: String },
    #[error("could not register the repo in the project-set inventory (remotes: {}): {guidance}", remotes.join(", "))]
    #[allow(dead_code)] // Reserved for the interactive ambiguity prompt path; not yet wired.
    ProjectSetRemoteAmbiguous {
        remotes: Vec<String>,
        guidance: String,
    },
    #[error("could not register the repo in the project-set inventory: {guidance}")]
    #[allow(dead_code)]
    // Reserved for the interactive slug-resolution prompt path; not yet wired.
    ProjectSetSlugUnresolved { guidance: String },
}

#[derive(Clone, Copy)]
struct TemplateAsset {
    path: &'static str,
    kind: AssetKind,
}

#[derive(Clone, Copy)]
struct TemplateDirectory {
    path: &'static str,
    kind: AssetKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Standard,
    Agents,
    Workflow,
}

#[derive(Clone)]
pub(crate) struct FetchedAsset {
    pub(crate) path: String,
    kind: AssetKind,
    pub(crate) contents: String,
}

#[derive(Debug, Deserialize)]
struct TemplateTreeResponse {
    tree: Vec<TemplateTreeEntry>,
}

#[derive(Debug, Deserialize)]
struct TemplateTreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AiReviewConfig {
    provider_kind: String,
    model_id: String,
    base_url: Option<String>,
    style: String,
    require_evidence: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AiReviewConfigPlan {
    config: AiReviewConfig,
    requested_by_flags: bool,
}

impl Default for AiReviewConfig {
    fn default() -> Self {
        Self {
            provider_kind: DEFAULT_AI_REVIEW_PROVIDER_KIND.to_string(),
            model_id: DEFAULT_AI_REVIEW_MODEL_ID.to_string(),
            base_url: Some(DEFAULT_AI_REVIEW_BASE_URL.to_string()),
            style: DEFAULT_AI_REVIEW_STYLE.to_string(),
            require_evidence: DEFAULT_AI_REVIEW_REQUIRE_EVIDENCE == "true",
        }
    }
}

impl AiReviewConfig {
    fn require_evidence_value(&self) -> &'static str {
        if self.require_evidence {
            "true"
        } else {
            "false"
        }
    }
}

enum PlannedAction {
    Create,
    Prompt,
    Overwrite,
    Skip,
    Unchanged,
    CustomizeWorkflow,
}

struct PlannedAsset {
    asset: FetchedAsset,
    existing: Option<String>,
    action: PlannedAction,
}

enum AppliedChange {
    Created,
    Overwritten,
    Updated,
    Skipped,
    Unchanged,
}

enum GhRepoAutomationStatus {
    Ready,
    MissingCli,
    RepoAccessUnavailable { details: String },
}

struct AiReviewGhAutomationResult {
    secret_updated: bool,
}

trait EnvLookup {
    fn get(&self, name: &str) -> Option<String>;
}

struct ProcessEnvironment;

impl EnvLookup for ProcessEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }
}

struct PromptUi<R, W> {
    reader: R,
    writer: W,
    allow_prompts: bool,
}

impl<R, W> PromptUi<R, W>
where
    R: BufRead,
    W: Write,
{
    fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            allow_prompts: true,
        }
    }

    fn set_allow_prompts(&mut self, allow_prompts: bool) {
        self.allow_prompts = allow_prompts;
    }

    fn allow_prompts(&self) -> bool {
        self.allow_prompts
    }

    fn line(&mut self, message: impl AsRef<str>) -> Result<(), InitCommandError> {
        writeln!(self.writer, "{}", message.as_ref()).map_err(InitCommandError::PromptIo)
    }

    fn blank_line(&mut self) -> Result<(), InitCommandError> {
        writeln!(self.writer).map_err(InitCommandError::PromptIo)
    }

    fn prompt(&mut self, prompt: &str) -> Result<String, InitCommandError> {
        if !self.allow_prompts {
            return Err(InitCommandError::NonInteractivePrompt {
                prompt: prompt.trim().to_string(),
            });
        }

        write!(self.writer, "{prompt}").map_err(InitCommandError::PromptIo)?;
        self.writer.flush().map_err(InitCommandError::PromptIo)?;

        let mut response = String::new();
        let bytes = self
            .reader
            .read_line(&mut response)
            .map_err(InitCommandError::PromptIo)?;
        if bytes == 0 {
            return Err(InitCommandError::PromptClosed);
        }

        while response.ends_with('\n') || response.ends_with('\r') {
            response.pop();
        }
        Ok(response)
    }
}

const CORE_TEMPLATE_ASSETS: &[TemplateAsset] = &[
    TemplateAsset {
        path: "WORKFLOW.md",
        kind: AssetKind::Workflow,
    },
    TemplateAsset {
        path: "AGENTS.md",
        kind: AssetKind::Agents,
    },
    TemplateAsset {
        path: "config.yaml",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".github/CODEOWNERS",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".github/pull_request_template.md",
        kind: AssetKind::Standard,
    },
];

const CORE_TEMPLATE_DIRECTORIES: &[TemplateDirectory] = &[TemplateDirectory {
    path: ".agents/skills",
    kind: AssetKind::Standard,
}];

const AI_REVIEW_TEMPLATE_ASSETS: &[TemplateAsset] = &[TemplateAsset {
    path: ".github/workflows/ai-pr-review.yml",
    kind: AssetKind::Standard,
}];

const AI_REVIEW_CUSTOM_GUIDE_ASSET: TemplateAsset = TemplateAsset {
    path: ".agents/skills/custom-codereview-guide.md",
    kind: AssetKind::Standard,
};

pub async fn run_command(args: InitArgs) -> ExitCode {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut ui = PromptUi::new(stdin.lock(), stdout.lock());

    match run_init(args, &ProcessEnvironment, &mut ui).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = ui.blank_line();
            let _ = ui.line(format!("opensymphony init failed: {error}"));
            ExitCode::from(1)
        }
    }
}

async fn run_init<R, W, E>(
    args: InitArgs,
    env_lookup: &E,
    ui: &mut PromptUi<R, W>,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
    E: EnvLookup,
{
    args.validate()?;
    ui.set_allow_prompts(!args.non_interactive);

    let target_repo = env::current_dir().map_err(InitCommandError::CurrentDir)?;
    ui.line(format!(
        "Initializing OpenSymphony files in {}",
        target_repo.display()
    ))?;
    let enable_ai_pr_review = if args.ai_pr_review_requested_by_flags() {
        true
    } else if args.non_interactive {
        false
    } else {
        prompt_yes_no(
            ui,
            "Also scaffold automated OpenHands AI PR review? [y/N]: ",
            false,
        )?
    };
    let ai_review_config = if enable_ai_pr_review {
        Some(
            if args.non_interactive || args.ai_pr_review_requested_by_flags() {
                args.ai_review_config_from_flags()
            } else {
                prompt_ai_review_config(ui)?
            },
        )
    } else {
        None
    };
    let client = Client::builder()
        .user_agent(concat!("opensymphony-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(template_fetch_timeout())
        .build()
        .map_err(InitCommandError::HttpClient)?;
    ui.line("Fetching the current template payload from GitHub...")?;

    let mut fetched_assets =
        fetch_template_assets(&client, CORE_TEMPLATE_ASSETS, CORE_TEMPLATE_DIRECTORIES).await?;
    if ai_review_config.is_some() {
        fetched_assets
            .extend(fetch_template_assets(&client, AI_REVIEW_TEMPLATE_ASSETS, &[]).await?);
        fetched_assets.extend(generated_ai_review_assets());
    }
    let mut planned_assets = plan_assets(&target_repo, fetched_assets)?;
    resolve_conflicts(&mut planned_assets, ui, &args)?;

    let workflow_will_change = planned_assets.iter().any(|planned| {
        planned.asset.kind == AssetKind::Workflow
            && matches!(
                planned.action,
                PlannedAction::Create | PlannedAction::Overwrite | PlannedAction::CustomizeWorkflow
            )
    });

    let git_remote = detect_git_remote_url(&target_repo);
    match &git_remote {
        GitRemoteDetection::Selected { remote_name, url } => {
            ui.line(format!(
                "Detected git remote `{remote_name}` -> {url}; `WORKFLOW.md` will use it for the clone hook."
            ))?;
        }
        GitRemoteDetection::None => {
            ui.line(
                "No git remote URL detected; `WORKFLOW.md` will keep its clone URL placeholder.",
            )?;
        }
        GitRemoteDetection::Ambiguous(remotes) => {
            ui.line(format!(
                "Found multiple git remotes without `origin` ({}); `WORKFLOW.md` will keep its clone URL placeholder.",
                remotes.join(", ")
            ))?;
        }
    }

    let linear_project_slug = if workflow_will_change {
        // Phase 1 ([LOC-19](https://linear.app/localgputokenscrazy/issue/LOC-19/init-multi-repo-onboarding)):
        // the Linear project slug/key now lives in `<cwd>/.opensymphony/project-set.yaml`
        // (`project_set.linear.project_slug`), not in `WORKFLOW.md`. We still
        // capture the same value at init time so we can populate it into the
        // project-set file; if the operator skips, the project-set writer
        // falls back to a sensible default slug derived from the directory
        // name.
        if let Some(slug) = args
            .linear_project_slug
            .as_deref()
            .map(str::trim)
            .filter(|slug| !slug.is_empty())
        {
            Some(slug.to_string())
        } else if args.non_interactive {
            None
        } else {
            let response = ui.prompt(
                "Enter your Linear project slug/key (leave blank to set it later in \
                 `<cwd>/.opensymphony/project-set.yaml`): ",
            )?;
            let response = response.trim();
            (!response.is_empty()).then(|| response.to_owned())
        }
    } else {
        None
    };

    let mut created = Vec::new();
    let mut overwritten = Vec::new();
    let mut updated = Vec::new();
    let mut skipped = Vec::new();
    let mut unchanged = Vec::new();
    let mut wrote_config = false;
    let mut agents_example_available = false;

    for planned in planned_assets {
        let destination = target_repo.join(&planned.asset.path);
        let relative_path = planned.asset.path.clone();
        if relative_path == AGENTS_EXAMPLE_PATH {
            agents_example_available = true;
        }

        let final_result = apply_asset(&destination, planned)?;

        match final_result {
            AppliedChange::Created => {
                if relative_path == "config.yaml" {
                    wrote_config = true;
                }
                created.push(relative_path);
            }
            AppliedChange::Overwritten => {
                if relative_path == "config.yaml" {
                    wrote_config = true;
                }
                overwritten.push(relative_path);
            }
            AppliedChange::Updated => {
                if relative_path == "config.yaml" {
                    wrote_config = true;
                }
                updated.push(relative_path);
            }
            AppliedChange::Skipped => skipped.push(relative_path),
            AppliedChange::Unchanged => unchanged.push(relative_path),
        }
    }

    record_memory_init_changes(
        &ensure_memory_initialized(&target_repo, None)?,
        &target_repo,
        &mut created,
        &mut updated,
        &mut unchanged,
    );

    // Phase 1 multi-repo onboarding ([LOC-19](https://linear.app/localgputokenscrazy/issue/LOC-19/init-multi-repo-onboarding)):
    // register the freshly onboarded repo into the project-set inventory
    // under `<cwd>/.opensymphony/project-set.yaml`. The `config_root` for
    // Phase 1 is always the cwd, so we never touch any external project-set
    // root.
    match upsert_project_set_entry_for_init(
        &target_repo,
        &git_remote,
        RepoOverrides {
            repo_url: args.repo_url.as_deref(),
            repo_slug: args.repo_slug.as_deref(),
            repo_default_branch: args.repo_default_branch.as_deref(),
        },
        linear_project_slug.as_deref(),
        ui,
    )? {
        Some(applied) => match applied {
            ProjectSetAppliedOutcome::Created(path) => {
                ui.line(format!(
                    "Created project-set inventory entry at {}; commit it as a regular config file.",
                    path.display()
                ))?;
                created.push(".opensymphony/project-set.yaml".to_string());
            }
            ProjectSetAppliedOutcome::Updated(path) => {
                ui.line(format!(
                    "Updated project-set inventory entry at {}.",
                    path.display()
                ))?;
                updated.push(".opensymphony/project-set.yaml".to_string());
            }
            ProjectSetAppliedOutcome::Unchanged(path) => {
                ui.line(format!(
                    "Project-set inventory entry at {} is already up to date.",
                    path.display()
                ))?;
                unchanged.push(".opensymphony/project-set.yaml".to_string());
            }
        },
        None => {
            // Operator chose to skip project-set registration; that's a
            // supported Phase 1 outcome. The next `opensymphony update`
            // run (or a re-invocation of `init`) can pick it up.
            ui.line(
                "Skipped project-set inventory registration; the repo is not yet in `<cwd>/.opensymphony/project-set.yaml`.",
            )?;
        }
    }

    ui.blank_line()?;
    ui.line("Initialization summary:")?;
    print_group(ui, "Created", &created)?;
    print_group(ui, "Overwritten", &overwritten)?;
    print_group(ui, "Updated", &updated)?;
    print_group(ui, "Skipped", &skipped)?;
    print_group(ui, "Unchanged", &unchanged)?;

    if agents_example_available {
        ui.blank_line()?;
        ui.line(
            "`AGENTS.md` already existed, so OpenSymphony left it untouched and wrote the starter guidance to `AGENTS-example.md`. Review or ask an agent to review the example, copy over any relevant guidance, then delete `AGENTS-example.md`.",
        )?;
    }

    if wrote_config {
        ui.blank_line()?;
        ui.line(
            "For the managed local OpenHands server, run `opensymphony install openhands` to provision the pinned tooling into the configured `openhands.tool_dir`.",
        )?;
    }

    if let Some(config) = ai_review_config.as_ref() {
        handle_ai_pr_review_setup(ui, env_lookup, &target_repo, &git_remote, config, &args)?;
    }

    prompt_for_missing_llm_env(env_lookup, ui, &args)?;

    let changed_paths = changed_paths_for_commit(&created, &overwritten, &updated);
    prompt_to_commit_and_push(ui, &target_repo, &git_remote, &changed_paths, &args)?;

    ui.blank_line()?;
    ui.line("OpenSymphony init complete.")?;
    Ok(())
}

async fn fetch_template_assets(
    client: &Client,
    assets: &[TemplateAsset],
    directories: &[TemplateDirectory],
) -> Result<Vec<FetchedAsset>, InitCommandError> {
    let base_url = env::var("OPENSYMPHONY_TEMPLATE_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_TEMPLATE_BASE_URL.to_string());
    let base_url =
        Url::parse(&base_url).map_err(|source| InitCommandError::InvalidTemplateBaseUrl {
            value: base_url.clone(),
            source,
        })?;
    let tree_url = match env::var("OPENSYMPHONY_TEMPLATE_TREE_URL") {
        Ok(tree_url) => {
            Url::parse(&tree_url).map_err(|source| InitCommandError::InvalidTemplateBaseUrl {
                value: tree_url,
                source,
            })?
        }
        Err(_) if env::var_os("OPENSYMPHONY_TEMPLATE_BASE_URL").is_some() => base_url
            .join("__tree.json")
            .map_err(|source| InitCommandError::InvalidTemplateBaseUrl {
                value: format!("{base_url}__tree.json"),
                source,
            })?,
        Err(_) => {
            Url::parse(DEFAULT_TEMPLATE_TREE_URL).expect("default template tree URL is valid")
        }
    };

    let tree_paths = if directories.is_empty() {
        Vec::new()
    } else {
        fetch_template_tree(client, &tree_url).await?
    };

    let mut fetched = Vec::new();
    for definition in assets {
        fetched
            .push(fetch_template_file(client, &base_url, definition.path, definition.kind).await?);
    }

    for directory in directories {
        let prefix = format!("{}/", directory.path.trim_end_matches('/'));
        let mut matched_paths = tree_paths
            .iter()
            .filter(|path| path.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        matched_paths.sort();

        if matched_paths.is_empty() {
            return Err(InitCommandError::MissingTemplateDirectory {
                path: directory.path,
                url: tree_url.to_string(),
            });
        }

        for path in matched_paths {
            fetched.push(fetch_template_file(client, &base_url, &path, directory.kind).await?);
        }
    }

    Ok(fetched)
}

async fn fetch_template_tree(
    client: &Client,
    tree_url: &Url,
) -> Result<Vec<String>, InitCommandError> {
    let response = client
        .get(tree_url.clone())
        .send()
        .await
        .map_err(|source| InitCommandError::FetchTemplateTree {
            url: tree_url.to_string(),
            source,
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(InitCommandError::FetchTemplateTreeStatus {
            url: tree_url.to_string(),
            status,
        });
    }

    let tree = response
        .json::<TemplateTreeResponse>()
        .await
        .map_err(|source| InitCommandError::DecodeTemplateTree {
            url: tree_url.to_string(),
            source,
        })?;

    Ok(tree
        .tree
        .into_iter()
        .filter(|entry| entry.entry_type == "blob")
        .map(|entry| entry.path)
        .collect())
}

async fn fetch_template_file(
    client: &Client,
    base_url: &Url,
    path: &str,
    kind: AssetKind,
) -> Result<FetchedAsset, InitCommandError> {
    let url = base_url
        .join(path)
        .map_err(|source| InitCommandError::InvalidTemplateBaseUrl {
            value: format!("{base_url}{path}"),
            source,
        })?;
    let response =
        client
            .get(url.clone())
            .send()
            .await
            .map_err(|source| InitCommandError::FetchTemplate {
                path: path.to_string(),
                url: url.to_string(),
                source,
            })?;

    let status = response.status();
    if !status.is_success() {
        return Err(InitCommandError::FetchTemplateStatus {
            path: path.to_string(),
            url: url.to_string(),
            status,
        });
    }

    let contents = response
        .text()
        .await
        .map_err(|source| InitCommandError::DecodeTemplate {
            path: path.to_string(),
            url: url.to_string(),
            source,
        })?;

    Ok(FetchedAsset {
        path: path.to_string(),
        kind,
        contents,
    })
}

fn generated_ai_review_assets() -> Vec<FetchedAsset> {
    vec![FetchedAsset {
        path: AI_REVIEW_CUSTOM_GUIDE_ASSET.path.to_string(),
        kind: AI_REVIEW_CUSTOM_GUIDE_ASSET.kind,
        contents: custom_codereview_guide_contents(),
    }]
}

fn plan_assets(
    target_repo: &Path,
    assets: Vec<FetchedAsset>,
) -> Result<Vec<PlannedAsset>, InitCommandError> {
    let mut planned = Vec::with_capacity(assets.len());
    let first_initialization = !target_repo.join("config.yaml").is_file();

    for asset in assets {
        let destination = target_repo.join(&asset.path);
        match fs::read_to_string(&destination) {
            Ok(existing) => {
                let action = match asset.kind {
                    AssetKind::Agents => {
                        if first_initialization {
                            planned.push(plan_agents_example_asset(target_repo, asset)?);
                            continue;
                        } else {
                            PlannedAction::Unchanged
                        }
                    }
                    AssetKind::Workflow => {
                        if comparable_text(&existing) == comparable_text(&asset.contents) {
                            PlannedAction::CustomizeWorkflow
                        } else {
                            PlannedAction::Prompt
                        }
                    }
                    AssetKind::Standard => {
                        if comparable_text(&existing) == comparable_text(&asset.contents) {
                            PlannedAction::Unchanged
                        } else {
                            PlannedAction::Prompt
                        }
                    }
                };

                planned.push(PlannedAsset {
                    asset,
                    existing: Some(existing),
                    action,
                });
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                planned.push(PlannedAsset {
                    asset,
                    existing: None,
                    action: PlannedAction::Create,
                });
            }
            Err(source) => {
                return Err(InitCommandError::ReadFile {
                    path: destination,
                    source,
                });
            }
        }
    }

    Ok(planned)
}

fn plan_agents_example_asset(
    target_repo: &Path,
    mut asset: FetchedAsset,
) -> Result<PlannedAsset, InitCommandError> {
    asset.path = AGENTS_EXAMPLE_PATH.to_string();
    let destination = target_repo.join(&asset.path);
    match fs::read_to_string(&destination) {
        Ok(existing) => {
            let action = if comparable_text(&existing) == comparable_text(&asset.contents) {
                PlannedAction::Unchanged
            } else {
                PlannedAction::Overwrite
            };
            Ok(PlannedAsset {
                asset,
                existing: Some(existing),
                action,
            })
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(PlannedAsset {
            asset,
            existing: None,
            action: PlannedAction::Create,
        }),
        Err(source) => Err(InitCommandError::ReadFile {
            path: destination,
            source,
        }),
    }
}

fn resolve_conflicts<R, W>(
    planned_assets: &mut [PlannedAsset],
    ui: &mut PromptUi<R, W>,
    args: &InitArgs,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
{
    for planned in planned_assets {
        if !matches!(planned.action, PlannedAction::Prompt) {
            continue;
        }

        let relative_path = Path::new(&planned.asset.path);
        let display_path = relative_path.display();

        if let Some(policy) = args.conflict_policy {
            planned.action = match policy {
                InitConflictPolicy::Skip => PlannedAction::Skip,
                InitConflictPolicy::Overwrite => PlannedAction::Overwrite,
                InitConflictPolicy::Abort => {
                    return Err(InitCommandError::ConflictPolicyAbort {
                        path: display_path.to_string(),
                    });
                }
            };
            continue;
        }

        if args.non_interactive {
            return Err(InitCommandError::NonInteractiveMissing {
                decision: format!("a conflict policy for existing `{display_path}`"),
                flag: "--conflict-policy skip|overwrite|abort",
            });
        }

        loop {
            ui.blank_line()?;
            ui.line(format!("`{display_path}` already exists."))?;
            let response = ui.prompt("Choose [s]kip, [o]verwrite, or [a]bort: ")?;
            match response.trim().to_ascii_lowercase().as_str() {
                "s" | "skip" => {
                    planned.action = PlannedAction::Skip;
                    break;
                }
                "o" | "overwrite" => {
                    planned.action = PlannedAction::Overwrite;
                    break;
                }
                "a" | "abort" => return Err(InitCommandError::AbortedByUser),
                _ => {
                    ui.line("Please answer with `skip`, `overwrite`, or `abort`.")?;
                }
            }
        }
    }

    Ok(())
}

fn apply_asset(
    destination: &Path,
    planned: PlannedAsset,
) -> Result<AppliedChange, InitCommandError> {
    let existing = planned.existing.as_deref();

    let Some(final_contents) = build_final_contents(&planned.asset, &planned.action) else {
        return Ok(match planned.action {
            PlannedAction::Skip => AppliedChange::Skipped,
            PlannedAction::Unchanged => AppliedChange::Unchanged,
            PlannedAction::Create
            | PlannedAction::Overwrite
            | PlannedAction::CustomizeWorkflow
            | PlannedAction::Prompt => AppliedChange::Unchanged,
        });
    };

    if let Some(existing) = existing
        && comparable_text(existing) == comparable_text(&final_contents)
    {
        return Ok(AppliedChange::Unchanged);
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| InitCommandError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(destination, final_contents).map_err(|source| InitCommandError::WriteFile {
        path: destination.to_path_buf(),
        source,
    })?;

    Ok(match planned.action {
        PlannedAction::Create => AppliedChange::Created,
        PlannedAction::Overwrite => AppliedChange::Overwritten,
        PlannedAction::CustomizeWorkflow => AppliedChange::Updated,
        PlannedAction::Skip => AppliedChange::Skipped,
        PlannedAction::Unchanged => AppliedChange::Unchanged,
        PlannedAction::Prompt => unreachable!("conflicts should be resolved before apply"),
    })
}

fn build_final_contents(asset: &FetchedAsset, action: &PlannedAction) -> Option<String> {
    match action {
        PlannedAction::Create | PlannedAction::Overwrite => Some(match asset.kind {
            AssetKind::Workflow => customize_workflow(&asset.contents),
            _ => asset.contents.clone(),
        }),
        PlannedAction::CustomizeWorkflow => Some(customize_workflow(&asset.contents)),
        PlannedAction::Skip | PlannedAction::Unchanged => None,
        PlannedAction::Prompt => None,
    }
}

fn prompt_for_missing_llm_env<R, W, E>(
    env_lookup: &E,
    ui: &mut PromptUi<R, W>,
    args: &InitArgs,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
    E: EnvLookup,
{
    let mut exports = Vec::new();

    if env_lookup.get("LLM_MODEL").is_none() {
        let value = if let Some(value) = trimmed_non_empty(args.llm_model.as_deref()) {
            value
        } else if args.non_interactive {
            DEFAULT_LLM_MODEL.to_string()
        } else {
            let response = ui.prompt(&format!(
                "LLM_MODEL is not set. Enter a model now, or press Enter to use `{DEFAULT_LLM_MODEL}`: "
            ))?;
            match response.trim() {
                "" => DEFAULT_LLM_MODEL.to_string(),
                custom => custom.to_string(),
            }
        };
        exports.push(("LLM_MODEL", value));
    }

    if env_lookup.get("LLM_API_KEY").is_none() {
        let value = if let Some(value) = trimmed_non_empty(args.llm_api_key_placeholder.as_deref())
        {
            value
        } else if args.non_interactive {
            "<your-llm-api-key>".to_string()
        } else {
            let response = ui.prompt(
                "LLM_API_KEY is not set. Press Enter to use the placeholder `<your-llm-api-key>` in the export snippet, or type a different placeholder label: ",
            )?;
            match response.trim() {
                "" => "<your-llm-api-key>".to_string(),
                custom => custom.to_string(),
            }
        };
        exports.push(("LLM_API_KEY", value));
    }

    if env_lookup.get("LLM_BASE_URL").is_none() {
        let value = if let Some(value) = trimmed_non_empty(args.llm_base_url.as_deref()) {
            value
        } else if args.non_interactive {
            DEFAULT_LLM_BASE_URL.to_string()
        } else {
            let response = ui.prompt(&format!(
                "LLM_BASE_URL is not set. Enter a base URL now, or press Enter to use `{DEFAULT_LLM_BASE_URL}`: "
            ))?;
            match response.trim() {
                "" => DEFAULT_LLM_BASE_URL.to_string(),
                custom => custom.to_string(),
            }
        };
        exports.push(("LLM_BASE_URL", value));
    }

    if exports.is_empty() {
        return Ok(());
    }

    ui.blank_line()?;
    ui.line("Before `opensymphony run`, export these in your shell:")?;
    for (name, value) in exports {
        ui.line(format!("export {name}={}", shell_single_quote(&value)))?;
    }
    Ok(())
}

fn changed_paths_for_commit(
    created: &[String],
    overwritten: &[String],
    updated: &[String],
) -> Vec<String> {
    created
        .iter()
        .chain(overwritten)
        .chain(updated)
        .filter(|path| !path.trim().is_empty())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn prompt_to_commit_and_push<R, W>(
    ui: &mut PromptUi<R, W>,
    target_repo: &Path,
    git_remote: &GitRemoteDetection,
    changed_paths: &[String],
    args: &InitArgs,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
{
    if changed_paths.is_empty() {
        return Ok(());
    }

    ui.blank_line()?;
    ui.line(
        "OpenSymphony wrote bootstrap files that should be committed and pushed before story work so shared agent skills and any AI PR Review setup are available.",
    )?;

    let Some(remote_name) = git_remote.remote_name() else {
        ui.line(
            "Skipping automatic commit/push because no single git remote was detected. Commit and push the generated OpenSymphony files before starting story work.",
        )?;
        return Ok(());
    };

    let branch_name = match current_git_branch(target_repo) {
        Ok(Some(branch_name)) => branch_name,
        Ok(None) => {
            ui.line(
                "Skipping automatic commit/push because the repository is not on a named branch. Commit and push the generated OpenSymphony files before starting story work.",
            )?;
            return Ok(());
        }
        Err(error) => {
            ui.line(format!(
                "Skipping automatic commit/push because git branch detection failed: {error}"
            ))?;
            return Ok(());
        }
    };

    match git_has_staged_changes(target_repo) {
        Ok(false) => {}
        Ok(true) => {
            ui.line(
                "Skipping automatic commit/push because there are already staged git changes. Commit or unstage those first, then commit and push the generated OpenSymphony files.",
            )?;
            return Ok(());
        }
        Err(error) => {
            ui.line(format!(
                "Skipping automatic commit/push because git status failed: {error}"
            ))?;
            return Ok(());
        }
    }

    let should_commit_and_push = if args.commit_and_push {
        true
    } else if args.non_interactive {
        ui.line(
            "Skipped automatic commit/push. Pass `--commit-and-push` to perform it during non-interactive init.",
        )?;
        false
    } else {
        prompt_yes_no(
            ui,
            &format!(
                "Commit and push these OpenSymphony bootstrap changes to `{remote_name}/{branch_name}` now? [y/N]: "
            ),
            false,
        )?
    };

    if !should_commit_and_push {
        ui.line(
            "Skipped commit/push for now. Before starting story work, commit and push the generated OpenSymphony files.",
        )?;
        return Ok(());
    }

    match commit_and_push_bootstrap_changes(target_repo, remote_name, changed_paths) {
        Ok(()) => {
            ui.line(format!(
                "Committed and pushed OpenSymphony bootstrap changes to `{remote_name}/{branch_name}`."
            ))?;
        }
        Err(error) => {
            ui.line(format!(
                "Git commit/push could not finish automatically: {error}"
            ))?;
            ui.line(
                "Review `git status`, then commit and push the generated OpenSymphony files before starting story work.",
            )?;
        }
    }

    Ok(())
}

fn current_git_branch(target_repo: &Path) -> Result<Option<String>, String> {
    let output = run_git_command(target_repo, &["branch", "--show-current"])
        .map_err(|source| format!("failed to run `git branch --show-current`: {source}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git branch --show-current` failed: {}",
            summarize_command_output(&output)
        ));
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!branch.is_empty()).then_some(branch))
}

fn git_has_staged_changes(target_repo: &Path) -> Result<bool, String> {
    let output = run_git_command(target_repo, &["diff", "--cached", "--quiet"])
        .map_err(|source| format!("failed to run `git diff --cached --quiet`: {source}"))?;
    if output.status.success() {
        return Ok(false);
    }

    match output.status.code() {
        Some(1) => Ok(true),
        _ => Err(format!(
            "`git diff --cached --quiet` failed: {}",
            summarize_command_output(&output)
        )),
    }
}

fn commit_and_push_bootstrap_changes(
    target_repo: &Path,
    remote_name: &str,
    changed_paths: &[String],
) -> Result<(), String> {
    let mut add_args = vec!["add".to_string(), "--".to_string()];
    add_args.extend(changed_paths.iter().cloned());
    run_git_command_checked(target_repo, &add_args)?;

    run_git_command_checked(
        target_repo,
        &[
            "commit".to_string(),
            "-m".to_string(),
            "chore: bootstrap OpenSymphony".to_string(),
        ],
    )?;
    run_git_command_checked(
        target_repo,
        &[
            "push".to_string(),
            "-u".to_string(),
            remote_name.to_string(),
            "HEAD".to_string(),
        ],
    )?;

    Ok(())
}

fn prompt_yes_no<R, W>(
    ui: &mut PromptUi<R, W>,
    prompt: &str,
    default: bool,
) -> Result<bool, InitCommandError>
where
    R: BufRead,
    W: Write,
{
    loop {
        let response = ui.prompt(prompt)?;
        match response.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                ui.line("Please answer with `yes` or `no`.")?;
            }
        }
    }
}

fn prompt_with_default<R, W>(
    ui: &mut PromptUi<R, W>,
    prompt: &str,
    default: &str,
) -> Result<String, InitCommandError>
where
    R: BufRead,
    W: Write,
{
    let response = ui.prompt(prompt)?;
    let trimmed = response.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_ai_review_config<R, W>(
    ui: &mut PromptUi<R, W>,
) -> Result<AiReviewConfig, InitCommandError>
where
    R: BufRead,
    W: Write,
{
    ui.blank_line()?;
    ui.line("Configure the default AI PR review provider for this repository.")?;
    ui.line(
        "Fireworks is the starter example, but these values can target any supported provider.",
    )?;

    let provider_kind = loop {
        let response = prompt_with_default(
            ui,
            "AI review provider kind [openai-compatible/litellm-native] (default openai-compatible): ",
            DEFAULT_AI_REVIEW_PROVIDER_KIND,
        )?;
        match response.as_str() {
            "openai-compatible" | "litellm-native" => break response,
            _ => ui.line("Please enter `openai-compatible` or `litellm-native`.")?,
        }
    };

    let model_id = prompt_with_default(
        ui,
        &format!("AI review model id (default {DEFAULT_AI_REVIEW_MODEL_ID}): "),
        DEFAULT_AI_REVIEW_MODEL_ID,
    )?;

    let base_url = if provider_kind == "openai-compatible" {
        Some(prompt_with_default(
            ui,
            &format!("AI review base URL (default {DEFAULT_AI_REVIEW_BASE_URL}): "),
            DEFAULT_AI_REVIEW_BASE_URL,
        )?)
    } else {
        None
    };

    let style = prompt_with_default(
        ui,
        &format!("AI review style (default {DEFAULT_AI_REVIEW_STYLE}): "),
        DEFAULT_AI_REVIEW_STYLE,
    )?;
    let require_evidence = prompt_yes_no(
        ui,
        "Require evidence in AI PR review findings? [Y/n]: ",
        true,
    )?;

    Ok(AiReviewConfig {
        provider_kind,
        model_id,
        base_url,
        style,
        require_evidence,
    })
}

pub(crate) fn template_fetch_timeout() -> Duration {
    template_fetch_timeout_from_env(
        env::var("OPENSYMPHONY_TEMPLATE_FETCH_TIMEOUT_MS")
            .ok()
            .as_deref(),
    )
}

pub(crate) async fn fetch_template_skill_assets(
    client: &Client,
) -> Result<Vec<FetchedAsset>, InitCommandError> {
    fetch_template_assets(client, &[], CORE_TEMPLATE_DIRECTORIES).await
}

fn template_fetch_timeout_from_env(value: Option<&str>) -> Duration {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|timeout_ms| *timeout_ms > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_TEMPLATE_FETCH_TIMEOUT_MS))
}

fn customize_workflow(template: &str) -> String {
    // The customizer turns the template's `WORKFLOW.md` into a
    // project-set-mode-clean bootstrap:
    //
    // 1. The `after_create` clone hook is rewritten to the static
    //    `opensymphony workspace clone` command. The runtime injects the
    //    resolved `RepoRef` via env vars at clone time (D7); we no longer
    //    bake a hardcoded clone URL into `WORKFLOW.md`.
    // 2. Project-set-owned global fields (`tracker.*`, `polling.interval_ms`,
    //    `agent.max_concurrent_agents`) are stripped from the front matter
    //    so the file is already valid under strict project-set mode. The
    //    canonical list lives in
    //    [`crate::opensymphony_workflow::STALE_MOVED_FIELDS`] and is the
    //    single source of truth for which fields move to
    //    `.opensymphony/project-set.yaml`.
    let mut customized = rewrite_after_create_to_static_hook(template);
    customized = strip_project_set_owned_fields(&customized);
    customized
}

const STATIC_AFTER_CREATE_HOOK: &str = "opensymphony workspace clone";

fn rewrite_after_create_to_static_hook(template: &str) -> String {
    // The bootstrap template ships a `git clone --depth 1 <remote> .` placeholder
    // hook. Replace the entire placeholder line(s) with the static
    // `opensymphony workspace clone` command so the generated `WORKFLOW.md`
    // never contains a hardcoded URL.
    const PLACEHOLDER_URL: &str = "https://github.com/YOUR-ORG/YOUR-REPO.git";
    let needle = format!("git clone --depth 1 {PLACEHOLDER_URL} .");
    template.replace(&needle, STATIC_AFTER_CREATE_HOOK)
}

/// Strip project-set-owned global fields from the canonical `WORKFLOW.md`
/// template so the generated file is already valid under strict
/// project-set mode (no stale `tracker.*`, `polling.*`, or
/// `agent.max_concurrent_agents`).
///
/// The fields are derived from the canonical [`STALE_MOVED_FIELDS`] constant
/// in [`crate::opensymphony_workflow`], so adding a new migration field there
/// automatically extends the strip list.
///
/// **Known limitation (LOC-19):** the rewritten YAML is produced via
/// [`serde_yaml::to_string`], which does not preserve comments, key
/// ordering, or blank-line formatting from the original document. This
/// function is intended for the fresh-init template (which has no
/// hand-edited YAML comments yet) and is not currently reused for
/// `opensymphony update` (LOC-20); see the linked AI review feedback
/// on `init_repo.rs::strip_project_set_owned_fields` for the suggested
/// comment-preserving YAML library when the migration path lands.
fn strip_project_set_owned_fields(template: &str) -> String {
    // The template's front matter is fenced between two `---\n` markers.
    // If parsing fails for any reason we fall back to returning the
    // template unchanged — the strict project-set mode resolver will
    // surface the field-migration error to the operator.
    //
    // Reuse the canonical `split_front_matter` from
    // [`crate::opensymphony_workflow`] so we have exactly one parser
    // implementation. The canonical version is line-based, so a YAML
    // scalar that contains `---` (e.g. `description: "use --- as
    // separator"`) is correctly preserved instead of being mistaken for
    // the closing delimiter (LOC-19 AI review feedback on the divergent
    // `split_front_matter` parser).
    let split = match crate::opensymphony_workflow::split_front_matter(template) {
        Some(split) => split,
        None => return template.to_string(),
    };
    let Ok(mut value) = serde_yaml::from_str::<serde_yaml::Value>(split.front_matter) else {
        return template.to_string();
    };
    let Some(mapping) = value.as_mapping_mut() else {
        return template.to_string();
    };

    // Iterate the canonical [`STALE_MOVED_FIELDS`] list so a new field
    // added to the project-set migration contract is automatically stripped
    // from the generated `WORKFLOW.md`. This is the single source of truth
    // for which fields move to `.opensymphony/project-set.yaml` (LOC-19 AI
    // review feedback on the hardcoded strip list).
    for entry in crate::opensymphony_workflow::STALE_MOVED_FIELDS {
        match entry.field.split_once('.') {
            None => {
                // Top-level field (e.g. an exact-match single key). Drop
                // the whole key so we don't leave an empty mapping behind.
                mapping.remove(entry.field);
            }
            Some((parent, child)) => {
                if let Some(parent_mapping) =
                    mapping.get_mut(parent).and_then(|v| v.as_mapping_mut())
                {
                    parent_mapping.remove(child);
                }
            }
        }
    }

    // After removing stale sub-fields, drop the parent key if it is now
    // empty (e.g. `tracker` after every `tracker.*` was removed, or
    // `polling` after `polling.interval_ms` was the only sub-field). The
    // set of parent keys is derived from `STALE_MOVED_FIELDS` so it stays
    // in sync with the canonical contract.
    let stale_parents: BTreeSet<&str> = crate::opensymphony_workflow::STALE_MOVED_FIELDS
        .iter()
        .filter_map(|entry| entry.field.split_once('.').map(|(parent, _)| parent))
        .collect();
    for parent in &stale_parents {
        let is_empty_mapping = mapping
            .get(*parent)
            .and_then(|value| value.as_mapping())
            .is_some_and(|m| m.is_empty());
        if is_empty_mapping {
            mapping.remove(*parent);
        }
    }

    let serialized_front_matter = match serde_yaml::to_string(&value) {
        Ok(serialized) => serialized,
        Err(_) => return template.to_string(),
    };
    // The canonical `split_front_matter` returns a [`FrontMatterSplit`]
    // that already exposes the verbatim opening marker line (`head`),
    // the YAML slice, the verbatim closing marker line (`trailer`), and
    // the trailing body. We splice the serialized front matter back in
    // between `head` and `trailer`, then re-attach `body` — so the
    // rebuilt document keeps its exact framing byte-for-byte (LOC-19).
    // The slices are taken straight from the [`FrontMatterSplit`]
    // output, so we never have to do pointer arithmetic or re-match
    // against `---` appearing inside YAML scalars.
    let head = split.head;
    let trailer = split.trailer;
    let body = split.body;
    format!("{head}{serialized_front_matter}{trailer}{body}")
}

fn comparable_text(value: &str) -> String {
    value.replace("\r\n", "\n").trim_end().to_owned()
}

/// Operator-supplied overrides for the repo entry that `init` writes into
/// `.opensymphony/project-set.yaml`.
///
/// Groups the `--repo-url`, `--repo-slug`, and `--repo-default-branch`
/// flag overrides into one struct so the upsert call site stays
/// self-documenting (LOC-19 AI review feedback on the 7-argument
/// `upsert_project_set_entry_for_init` signature).
#[derive(Debug, Clone, Default)]
struct RepoOverrides<'a> {
    /// Operator-supplied repo URL override (e.g. when no git remote is
    /// configured, or when registering a different repo than cwd).
    repo_url: Option<&'a str>,
    /// Operator-supplied repo slug override (takes precedence over the
    /// remote-name and directory-name derived defaults).
    repo_slug: Option<&'a str>,
    /// Operator-supplied default-branch override (takes precedence over
    /// the value `git remote show <remote>` reports).
    repo_default_branch: Option<&'a str>,
}

/// Registers the freshly onboarded repo into the project-set inventory
/// under `<target_repo>/.opensymphony/project-set.yaml` (LOC-19 / D9).
///
/// Returns `Ok(None)` when the operator explicitly skips the registration
/// (either by passing `--skip-project-set-registration` or by answering
/// "no" to the interactive prompt). Returns `Ok(Some(_))` whenever the
/// upsert succeeded so callers can surface the change in the summary.
fn upsert_project_set_entry_for_init<R, W>(
    target_repo: &Path,
    git_remote: &GitRemoteDetection,
    overrides: RepoOverrides<'_>,
    linear_project_slug: Option<&str>,
    ui: &mut PromptUi<R, W>,
) -> Result<Option<ProjectSetAppliedOutcome>, InitCommandError>
where
    R: BufRead,
    W: Write,
{
    use super::project_set_writer::{
        ProjectSetUpsertPlan, project_set_path, upsert_project_set_yaml_with_path,
    };

    // The repo URL comes from one of these sources, in priority order:
    // 1. `--repo-url` flag (highest priority; lets operators register a repo
    //    that isn't the cwd).
    // 2. The confidently detected git remote URL.
    // 3. None (the operator will be prompted for one, unless non-interactive).
    let trimmed_override = trimmed_non_empty(overrides.repo_url);
    let detected_url = git_remote.url().map(ToOwned::to_owned);
    let repo_url = trimmed_override.clone().or_else(|| detected_url.clone());

    let repo_url = match repo_url {
        Some(url) => url,
        None => {
            // Phase-1 missing remote: prompt interactively; fail with actionable
            // guidance in non-interactive mode.
            let reason = "OpenSymphony detected no git remote and no `--repo-url` flag, so it cannot register this repo into the project-set inventory.";
            if ui.allow_prompts() {
                let response = ui.prompt(
                    "Enter the clone URL for this repo (leave blank to skip project-set registration): ",
                )?;
                let trimmed = response.trim();
                if trimmed.is_empty() {
                    return Ok(None);
                }
                trimmed.to_owned()
            } else {
                return Err(InitCommandError::ProjectSetRemoteMissing {
                    guidance: format!(
                        "{reason} Re-run `opensymphony init --repo-url <url>` to register the repo."
                    ),
                });
            }
        }
    };

    let trimmed_default_branch_override = trimmed_non_empty(overrides.repo_default_branch);
    let detected_default_branch = git_remote
        .remote_name()
        .and_then(|remote| detect_git_default_branch(target_repo, remote));
    let default_branch = trimmed_default_branch_override
        .clone()
        .or(detected_default_branch);

    // The slug chain runs from override → remote-name → directory-name.
    // When all three sources fail the literal `DEFAULT_REPO_SLUG_FALLBACK`
    // is used as a last resort. Surface that to the operator so the
    // inventory never silently picks up a low-quality slug like `repo`
    // or `default-project-set` (LOC-19 AI review feedback on the slug
    // derivation chain at `init_repo.rs::upsert_project_set_entry_for_init`).
    let derived_slug = trimmed_non_empty(overrides.repo_slug)
        .or_else(|| derive_repo_slug_from_remote(&repo_url))
        .or_else(|| derive_repo_slug_from_dir(target_repo));
    let repo_slug = match derived_slug {
        Some(slug) => slug,
        None => {
            ui.line(format!(
                "warning: could not derive a repo slug from `--repo-slug`, the git remote URL, or the directory name; falling back to `{DEFAULT_REPO_SLUG_FALLBACK}`. Re-run with `--repo-slug <slug>` to override."
            ))?;
            DEFAULT_REPO_SLUG_FALLBACK.to_owned()
        }
    };

    // The Linear project slug is reused for both `project_set.projects[].slug`
    // and `project_set.linear.project_slug`. When the operator skipped the
    // Linear prompt we still create a usable project-set file with sentinel
    // values so the inventory is committed and editable.
    let linear_project_slug =
        trimmed_non_empty(linear_project_slug).unwrap_or_else(|| repo_slug.clone());

    // Build the upsert plan. We let the writer fill in defaults (polling
    // interval, max concurrent agents, active/terminal states) when no
    // override was supplied — see `project_set_writer::apply_upsert_plan`.
    let plan = ProjectSetUpsertPlan {
        repo_slug: repo_slug.clone(),
        repo_url: repo_url.clone(),
        default_branch,
        project_set_slug: DEFAULT_PROJECT_SET_SLUG_FALLBACK.to_owned(),
        project_slug: linear_project_slug.clone(),
        linear_project_slug,
        linear_api_key_env: Some(DEFAULT_LINEAR_API_KEY_ENV.to_owned()),
        polling_interval_ms: None,
        max_concurrent_agents: None,
        linear_active_states: None,
        linear_terminal_states: None,
    };

    let path = project_set_path(target_repo);
    match upsert_project_set_yaml_with_path(&path, &plan) {
        Ok(applied) => Ok(Some(applied)),
        Err(source) => Err(InitCommandError::ProjectSetUpsert(source)),
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn custom_codereview_guide_contents() -> String {
    r#"---
name: custom-codereview-guide
description: |
  Repository-specific code review guidance for this project.
  Update this file so OpenHands PR review focuses on the right risks.
---

# Custom Code Review Guide

OpenHands PR review will load this file when it is present. Replace this starter content with repository-specific expectations.

## Default Priorities

- Prioritize correctness, regressions, security risks, and missing tests ahead of style-only feedback.
- Treat behavior changes as incomplete unless the PR includes concrete verification or evidence.
- Call out risky data migrations, auth changes, concurrency hazards, and production operability regressions explicitly.

## Customize For This Repository

- List the most security-sensitive paths or subsystems.
- List required validation commands reviewers should expect to see.
- Describe any architecture invariants that must not be broken.
- Add framework- or language-specific review heuristics that matter here.

## Evidence Expectations

- Behavior changes should include test or reproduction output.
- UI changes should include screenshots or recordings.
- Performance-sensitive changes should include benchmark data or timing notes.
"#
    .to_string()
}

fn handle_ai_pr_review_setup<R, W, E>(
    ui: &mut PromptUi<R, W>,
    env_lookup: &E,
    target_repo: &Path,
    git_remote: &GitRemoteDetection,
    config: &AiReviewConfig,
    args: &InitArgs,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
    E: EnvLookup,
{
    ui.blank_line()?;
    ui.line("OpenHands PR review scaffolding was added.")?;
    let Some(repo_slug) = git_remote.url().and_then(github_repo_slug_from_remote) else {
        ui.line(
            "GitHub automation was skipped because the detected git remote is missing or is not a GitHub repository URL.",
        )?;
        print_ai_review_setup_links(ui)?;
        return Ok(());
    };

    match check_gh_repo_automation(target_repo, &repo_slug) {
        GhRepoAutomationStatus::Ready => {}
        GhRepoAutomationStatus::MissingCli => {
            ui.line(
                "GitHub automation was skipped because `gh` is not installed or is not available on `PATH`.",
            )?;
            ui.line(
                "Install GitHub CLI, run `gh auth login`, and then run these commands when you're ready:",
            )?;
            print_ai_review_cli_fallback(ui, &repo_slug, config)?;
            return Ok(());
        }
        GhRepoAutomationStatus::RepoAccessUnavailable { details } => {
            ui.line(format!(
                "GitHub automation was skipped because `gh` could not access `{repo_slug}`."
            ))?;
            if !details.is_empty() {
                ui.line(format!("`gh` reported: {details}"))?;
            }
            ui.line(
                "Run `gh auth login` with an account that can manage this repository, then run these commands when you're ready:",
            )?;
            print_ai_review_cli_fallback(ui, &repo_slug, config)?;
            return Ok(());
        }
    }

    let configure_github = if args.configure_github {
        true
    } else if args.non_interactive {
        false
    } else {
        prompt_yes_no(
            ui,
            &format!(
                "Configure GitHub Actions variables, the optional secret, and the `{AI_REVIEW_LABEL_NAME}` label for `{repo_slug}` now with `gh`? [Y/n]: "
            ),
            true,
        )?
    };

    if !configure_github {
        ui.line("Skipped GitHub automation for now.")?;
        print_ai_review_setup_links(ui)?;
        return Ok(());
    }

    let secret_value = resolve_ai_review_secret(ui, env_lookup, args)?;
    match configure_ai_review_with_gh(target_repo, &repo_slug, config, secret_value.as_deref()) {
        Ok(result) => {
            ui.line(format!(
                "GitHub Actions settings for `{repo_slug}` were configured with `gh`."
            ))?;
            ui.line("- variables: AI_REVIEW_PROVIDER_KIND, AI_REVIEW_MODEL_ID, AI_REVIEW_BASE_URL, AI_REVIEW_STYLE, AI_REVIEW_REQUIRE_EVIDENCE")?;
            ui.line(format!("- label: `{AI_REVIEW_LABEL_NAME}` ensured"))?;
            if result.secret_updated {
                ui.line(format!(
                    "- secret: `{DEFAULT_AI_REVIEW_SECRET_NAME}` updated"
                ))?;
            } else {
                ui.line(format!(
                    "- secret: `{DEFAULT_AI_REVIEW_SECRET_NAME}` was left unchanged; set it later if needed"
                ))?;
            }
        }
        Err(error) => {
            ui.line(format!(
                "GitHub automation could not finish automatically: {error}"
            ))?;
            ui.line(
                "Make sure your account can manage repository variables, secrets, and labels, then finish the setup with the printed commands or the upstream guide.",
            )?;
            print_ai_review_setup_links(ui)?;
        }
    }

    print_ai_review_setup_links(ui)?;
    Ok(())
}

fn resolve_ai_review_secret<R, W, E>(
    ui: &mut PromptUi<R, W>,
    env_lookup: &E,
    args: &InitArgs,
) -> Result<Option<String>, InitCommandError>
where
    R: BufRead,
    W: Write,
    E: EnvLookup,
{
    if let Some(env_name) = args
        .ai_review_secret_env
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return require_non_empty_env(env_lookup, env_name, "--ai-review-secret-env").map(Some);
    }

    if args.reuse_llm_api_key_for_ai_review_secret {
        return require_non_empty_env(
            env_lookup,
            "LLM_API_KEY",
            "--reuse-llm-api-key-for-ai-review-secret",
        )
        .map(Some);
    }

    if args.skip_ai_review_secret {
        return Ok(None);
    }

    if args.non_interactive {
        return Err(InitCommandError::NonInteractiveMissing {
            decision: format!("an AI review secret choice for `{DEFAULT_AI_REVIEW_SECRET_NAME}`"),
            flag: "--ai-review-secret-env, --reuse-llm-api-key-for-ai-review-secret, or --skip-ai-review-secret",
        });
    }

    prompt_ai_review_secret(ui, env_lookup)
}

fn require_non_empty_env<E>(
    env_lookup: &E,
    name: &str,
    flag: &'static str,
) -> Result<String, InitCommandError>
where
    E: EnvLookup,
{
    let value = env_lookup
        .get(name)
        .ok_or_else(|| InitCommandError::MissingEnvVar {
            name: name.to_string(),
            flag,
        })?;
    if value.trim().is_empty() {
        return Err(InitCommandError::MissingEnvVar {
            name: name.to_string(),
            flag,
        });
    }
    Ok(value)
}

fn prompt_ai_review_secret<R, W, E>(
    ui: &mut PromptUi<R, W>,
    env_lookup: &E,
) -> Result<Option<String>, InitCommandError>
where
    R: BufRead,
    W: Write,
    E: EnvLookup,
{
    if let Some(llm_api_key) = env_lookup.get("LLM_API_KEY")
        && prompt_yes_no(
            ui,
            &format!(
                "Reuse the current `LLM_API_KEY` value for GitHub secret `{DEFAULT_AI_REVIEW_SECRET_NAME}`? [Y/n]: "
            ),
            true,
        )?
    {
        return Ok(Some(llm_api_key));
    }

    ui.line(format!(
        "`{DEFAULT_AI_REVIEW_SECRET_NAME}` is the provider key the GitHub Actions review workflow will use."
    ))?;
    let response = ui.prompt(&format!(
        "Enter a value for `{DEFAULT_AI_REVIEW_SECRET_NAME}` now (input is visible; leave blank to skip this step for now): "
    ))?;
    let response = response.trim();
    if response.is_empty() {
        Ok(None)
    } else {
        Ok(Some(response.to_string()))
    }
}

/// Derives the GitHub `owner/repo` slug used for `gh` CLI invocations from
/// a remote URL.
///
/// Distinct from
/// [`super::super::repo_detection::derive_repo_slug_from_remote`], which is
/// the project-set inventory slug (last path segment only). The AI PR
/// review path needs the full `owner/repo` form so `gh` accepts the
/// `repo view ... --json nameWithOwner` and `gh variable set ... -R owner/repo`
/// commands.
fn github_repo_slug_from_remote(remote_url: &str) -> Option<String> {
    if let Ok(url) = Url::parse(remote_url)
        && matches!(url.host_str(), Some("github.com" | "www.github.com"))
    {
        return normalize_github_repo_slug(url.path());
    }

    remote_url
        .strip_prefix("git@github.com:")
        .or_else(|| remote_url.strip_prefix("ssh://git@github.com/"))
        .and_then(normalize_github_repo_slug)
}

fn normalize_github_repo_slug(path: &str) -> Option<String> {
    let trimmed = path.trim_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let mut parts = trimmed.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

fn check_gh_repo_automation(target_repo: &Path, repo_slug: &str) -> GhRepoAutomationStatus {
    match run_gh_command(target_repo, &["--version"]) {
        Ok(output) if output.status.success() => {}
        Ok(_) => return GhRepoAutomationStatus::MissingCli,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return GhRepoAutomationStatus::MissingCli;
        }
        Err(source) => {
            return GhRepoAutomationStatus::RepoAccessUnavailable {
                details: source.to_string(),
            };
        }
    }

    match run_gh_command(
        target_repo,
        &["repo", "view", repo_slug, "--json", "nameWithOwner"],
    ) {
        Ok(output) if output.status.success() => GhRepoAutomationStatus::Ready,
        Ok(output) => GhRepoAutomationStatus::RepoAccessUnavailable {
            details: summarize_command_output(&output),
        },
        Err(source) => GhRepoAutomationStatus::RepoAccessUnavailable {
            details: source.to_string(),
        },
    }
}

fn configure_ai_review_with_gh(
    target_repo: &Path,
    repo_slug: &str,
    config: &AiReviewConfig,
    secret_value: Option<&str>,
) -> Result<AiReviewGhAutomationResult, String> {
    run_gh_command_checked(
        target_repo,
        &[
            "variable",
            "set",
            "AI_REVIEW_PROVIDER_KIND",
            "-R",
            repo_slug,
            "--body",
            &config.provider_kind,
        ],
    )?;
    run_gh_command_checked(
        target_repo,
        &[
            "variable",
            "set",
            "AI_REVIEW_MODEL_ID",
            "-R",
            repo_slug,
            "--body",
            &config.model_id,
        ],
    )?;
    run_gh_command_checked(
        target_repo,
        &[
            "variable",
            "set",
            "AI_REVIEW_BASE_URL",
            "-R",
            repo_slug,
            "--body",
            config.base_url.as_deref().unwrap_or(""),
        ],
    )?;
    run_gh_command_checked(
        target_repo,
        &[
            "variable",
            "set",
            "AI_REVIEW_STYLE",
            "-R",
            repo_slug,
            "--body",
            &config.style,
        ],
    )?;
    run_gh_command_checked(
        target_repo,
        &[
            "variable",
            "set",
            "AI_REVIEW_REQUIRE_EVIDENCE",
            "-R",
            repo_slug,
            "--body",
            config.require_evidence_value(),
        ],
    )?;
    run_gh_command_checked(
        target_repo,
        &[
            "label",
            "create",
            AI_REVIEW_LABEL_NAME,
            "-R",
            repo_slug,
            "--description",
            AI_REVIEW_LABEL_DESCRIPTION,
            "--color",
            "d73a4a",
            "--force",
        ],
    )?;

    let secret_updated = if let Some(secret_value) = secret_value {
        run_gh_secret_set(
            target_repo,
            repo_slug,
            DEFAULT_AI_REVIEW_SECRET_NAME,
            secret_value,
        )?;
        true
    } else {
        false
    };

    Ok(AiReviewGhAutomationResult { secret_updated })
}

fn print_ai_review_cli_fallback<R, W>(
    ui: &mut PromptUi<R, W>,
    repo_slug: &str,
    config: &AiReviewConfig,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
{
    ui.line(format!(
        "gh variable set AI_REVIEW_PROVIDER_KIND -R {repo_slug} --body {}",
        shell_single_quote(&config.provider_kind)
    ))?;
    ui.line(format!(
        "gh variable set AI_REVIEW_MODEL_ID -R {repo_slug} --body {}",
        shell_single_quote(&config.model_id)
    ))?;
    ui.line(format!(
        "gh variable set AI_REVIEW_BASE_URL -R {repo_slug} --body {}",
        shell_single_quote(config.base_url.as_deref().unwrap_or(""))
    ))?;
    ui.line(format!(
        "gh variable set AI_REVIEW_STYLE -R {repo_slug} --body {}",
        shell_single_quote(&config.style)
    ))?;
    ui.line(format!(
        "gh variable set AI_REVIEW_REQUIRE_EVIDENCE -R {repo_slug} --body {}",
        shell_single_quote(config.require_evidence_value())
    ))?;
    ui.line(format!(
        "gh secret set {DEFAULT_AI_REVIEW_SECRET_NAME} -R {repo_slug}"
    ))?;
    ui.line(format!(
        "gh label create {AI_REVIEW_LABEL_NAME} -R {repo_slug} --description {} --color d73a4a --force",
        shell_single_quote(AI_REVIEW_LABEL_DESCRIPTION)
    ))?;
    ui.line(
        "You can reuse the same value as `LLM_API_KEY` for `AI_REVIEW_API_KEY` if that is the provider key you want the review workflow to use.",
    )?;
    print_ai_review_setup_links(ui)?;
    Ok(())
}

fn print_ai_review_setup_links<R, W>(ui: &mut PromptUi<R, W>) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
{
    ui.line(format!(
        "Manual setup guide: {OPENHANDS_PR_REVIEW_SETUP_GUIDE_URL}"
    ))?;
    ui.line(format!("Plugin: {OPENHANDS_PR_REVIEW_PLUGIN_URL}"))?;
    ui.line(format!("Docs: {OPENHANDS_PR_REVIEW_DOCS_URL}"))?;
    Ok(())
}

fn run_gh_command(target_repo: &Path, args: &[&str]) -> io::Result<Output> {
    std::process::Command::new("gh")
        .args(args)
        .current_dir(target_repo)
        .output()
}

fn run_git_command(target_repo: &Path, args: &[&str]) -> io::Result<Output> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(target_repo)
        .output()
}

fn run_git_command_checked(target_repo: &Path, args: &[String]) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(target_repo)
        .output()
        .map_err(|source| format!("failed to run `git {}`: {source}", args.join(" ")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            summarize_command_output(&output)
        ))
    }
}

fn run_gh_command_checked(target_repo: &Path, args: &[&str]) -> Result<(), String> {
    let output = run_gh_command(target_repo, args)
        .map_err(|source| format!("failed to run `gh {}`: {source}", args.join(" ")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "`gh {}` failed: {}",
            args.join(" "),
            summarize_command_output(&output)
        ))
    }
}

fn run_gh_secret_set(
    target_repo: &Path,
    repo_slug: &str,
    secret_name: &str,
    secret_value: &str,
) -> Result<(), String> {
    let mut child = std::process::Command::new("gh")
        .args(["secret", "set", secret_name, "-R", repo_slug])
        .current_dir(target_repo)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|source| format!("failed to run `gh secret set {secret_name}`: {source}"))?;

    let Some(mut stdin) = child.stdin.take() else {
        return Err(format!(
            "failed to provide a value for `gh secret set {secret_name}`"
        ));
    };
    stdin
        .write_all(secret_value.as_bytes())
        .map_err(|source| format!("failed to write `{secret_name}` to `gh`: {source}"))?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|source| format!("failed to wait for `gh secret set {secret_name}`: {source}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "`gh secret set {secret_name}` failed: {}",
            summarize_command_output(&output)
        ))
    }
}

fn summarize_command_output(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return summarize_line(&stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return summarize_line(&stdout);
    }

    "command returned no output".to_string()
}

fn summarize_line(value: &str) -> String {
    const MAX_LEN: usize = 200;
    let first_line = value.lines().next().unwrap_or(value).trim();
    if first_line.len() > MAX_LEN {
        format!("{}...", &first_line[..MAX_LEN])
    } else {
        first_line.to_string()
    }
}

fn print_group<R, W>(
    ui: &mut PromptUi<R, W>,
    label: &str,
    items: &[String],
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
{
    if items.is_empty() {
        return Ok(());
    }

    ui.line(format!("{label}:"))?;
    for item in items {
        ui.line(format!("- {item}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::super::repo_detection::{derive_repo_slug_from_remote, select_remote_name};
    use super::{
        AiReviewConfig, AiReviewProviderKindArg, DEFAULT_AI_REVIEW_MODEL_ID, DEFAULT_LLM_BASE_URL,
        DEFAULT_LLM_MODEL, DEFAULT_TEMPLATE_FETCH_TIMEOUT_MS, GitRemoteDetection, InitArgs,
        InitCommandError, PromptUi, comparable_text, custom_codereview_guide_contents,
        customize_workflow, github_repo_slug_from_remote, normalize_github_repo_slug,
        prompt_ai_review_config, prompt_for_missing_llm_env, prompt_yes_no,
        resolve_ai_review_secret, shell_single_quote, template_fetch_timeout_from_env,
    };

    struct StubEnvironment {
        values: BTreeMap<String, String>,
    }

    impl super::EnvLookup for StubEnvironment {
        fn get(&self, name: &str) -> Option<String> {
            self.values.get(name).cloned()
        }
    }

    #[test]
    fn customize_workflow_emits_static_hook_and_strips_tracker_block() {
        let workflow = r#"---
tracker:
  project_slug: "YOUR-PROJECT-SLUG"
  endpoint: https://api.linear.app/graphql
hooks:
  after_create: |
    git clone --depth 1 https://github.com/YOUR-ORG/YOUR-REPO.git .
openhands:
  conversation:
    agent:
      llm:
        model: ${LLM_MODEL}
---
"#;

        let customized = customize_workflow(workflow);

        // Static clone hook is emitted; no hardcoded URL is left behind.
        assert!(
            customized.contains("opensymphony workspace clone"),
            "expected static hook in:\n{customized}"
        );
        assert!(
            !customized.contains("git clone"),
            "hardcoded git clone line must be removed; got:\n{customized}"
        );
        assert!(
            !customized.contains("YOUR-ORG/YOUR-REPO"),
            "placeholder URL must not leak through; got:\n{customized}"
        );
        // Project-set-owned global fields are stripped; the resulting
        // `WORKFLOW.md` is valid under strict project-set mode.
        assert!(
            !customized.contains("tracker:"),
            "tracker block must be removed; got:\n{customized}"
        );
        assert!(
            !customized.contains("YOUR-PROJECT-SLUG"),
            "Linear project slug placeholder must be removed; got:\n{customized}"
        );
        // Reputable non-project-set-owned fields are preserved.
        assert!(
            customized.contains("openhands:"),
            "openhands block must remain; got:\n{customized}"
        );
    }

    #[test]
    fn customize_workflow_strips_polling_and_agent_max_concurrent_agents() {
        let workflow = r#"---
tracker:
  kind: linear
polling:
  interval_ms: 5000
agent:
  max_concurrent_agents: 4
  max_turns: 32
hooks:
  after_create: |
    git clone --depth 1 https://github.com/YOUR-ORG/YOUR-REPO.git .
---
"#;

        let customized = customize_workflow(workflow);

        // Project-set-owned global fields are stripped.
        assert!(
            !customized.contains("tracker:"),
            "tracker block must be removed; got:\n{customized}"
        );
        assert!(
            !customized.contains("polling:"),
            "polling block must be removed; got:\n{customized}"
        );
        assert!(
            !customized.contains("max_concurrent_agents"),
            "agent.max_concurrent_agents must be removed; got:\n{customized}"
        );
        // Repo-local `agent.max_turns` is preserved.
        assert!(
            customized.contains("max_turns: 32"),
            "agent.max_turns must be preserved; got:\n{customized}"
        );
        assert!(
            customized.contains("opensymphony workspace clone"),
            "static hook must be emitted; got:\n{customized}"
        );
    }

    #[test]
    fn customize_workflow_preserves_body_when_stripping_fields() {
        let workflow = r#"---
tracker:
  project_slug: "YOUR-PROJECT-SLUG"
hooks:
  after_create: |
    git clone --depth 1 https://github.com/YOUR-ORG/YOUR-REPO.git .
---

# Workflow description

Some body content that must be preserved.
"#;

        let customized = customize_workflow(workflow);

        assert!(customized.contains("# Workflow description"));
        assert!(customized.contains("Some body content that must be preserved."));
        assert!(!customized.contains("tracker:"));
        assert!(customized.contains("opensymphony workspace clone"));
    }

    #[test]
    fn comparable_text_ignores_crlf_and_trailing_newlines() {
        assert_eq!(comparable_text("a\r\nb\r\n"), comparable_text("a\nb\n\n"));
    }

    #[test]
    fn existing_agents_gets_example_on_first_initialization_only() {
        let repo = tempfile::tempdir().expect("temp repo");
        std::fs::write(repo.path().join("AGENTS.md"), "# Existing\n")
            .expect("existing AGENTS should write");
        let assets = vec![super::FetchedAsset {
            path: "AGENTS.md".to_string(),
            kind: super::AssetKind::Agents,
            contents: "# Template\n".to_string(),
        }];

        let planned = super::plan_assets(repo.path(), assets).expect("plan should succeed");

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].asset.path, super::AGENTS_EXAMPLE_PATH);
        assert!(matches!(planned[0].action, super::PlannedAction::Create));
    }

    #[test]
    fn existing_agents_is_left_alone_after_config_exists() {
        let repo = tempfile::tempdir().expect("temp repo");
        std::fs::write(repo.path().join("AGENTS.md"), "# Existing\n")
            .expect("existing AGENTS should write");
        std::fs::write(repo.path().join("config.yaml"), "memory: {}\n")
            .expect("config should write");
        let assets = vec![super::FetchedAsset {
            path: "AGENTS.md".to_string(),
            kind: super::AssetKind::Agents,
            contents: "# Template\n".to_string(),
        }];

        let planned = super::plan_assets(repo.path(), assets).expect("plan should succeed");

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].asset.path, "AGENTS.md");
        assert!(matches!(planned[0].action, super::PlannedAction::Unchanged));
    }

    #[test]
    fn select_remote_prefers_origin_then_single_remote() {
        assert_eq!(
            select_remote_name(&["fork".to_string(), "origin".to_string()]),
            Some("origin".to_string())
        );
        assert_eq!(
            select_remote_name(&["upstream".to_string()]),
            Some("upstream".to_string())
        );
        assert_eq!(
            select_remote_name(&["fork".to_string(), "upstream".to_string()]),
            None
        );
    }

    #[test]
    fn git_remote_url_returns_selected_only() {
        let selected = GitRemoteDetection::Selected {
            remote_name: "origin".to_string(),
            url: "https://github.com/kumanday/OpenSymphony-template.git".to_string(),
        };
        assert_eq!(
            selected.url(),
            Some("https://github.com/kumanday/OpenSymphony-template.git")
        );
        assert_eq!(GitRemoteDetection::None.url(), None);
        assert_eq!(
            GitRemoteDetection::Ambiguous(vec!["fork".to_string()]).url(),
            None
        );
    }

    #[test]
    fn derive_repo_slug_parser_supports_https_and_ssh_remotes() {
        assert_eq!(
            derive_repo_slug_from_remote("https://github.com/kumanday/OpenSymphony.git"),
            Some("OpenSymphony".to_string())
        );
        assert_eq!(
            derive_repo_slug_from_remote("git@github.com:kumanday/OpenSymphony.git"),
            Some("OpenSymphony".to_string())
        );
        // Non-GitHub remotes still derive the last path segment so the project-set
        // inventory stays populated.
        assert_eq!(
            derive_repo_slug_from_remote("https://gitlab.com/kumanday/OpenSymphony.git"),
            Some("OpenSymphony".to_string())
        );
    }

    #[test]
    fn github_repo_slug_parser_supports_https_and_ssh_remotes() {
        assert_eq!(
            github_repo_slug_from_remote("https://github.com/kumanday/OpenSymphony.git"),
            Some("kumanday/OpenSymphony".to_string())
        );
        assert_eq!(
            github_repo_slug_from_remote("git@github.com:kumanday/OpenSymphony.git"),
            Some("kumanday/OpenSymphony".to_string())
        );
        assert_eq!(
            github_repo_slug_from_remote("https://gitlab.com/kumanday/OpenSymphony.git"),
            None
        );
    }

    #[test]
    fn normalize_github_repo_slug_rejects_invalid_paths() {
        assert_eq!(
            normalize_github_repo_slug("/kumanday/OpenSymphony.git"),
            Some("kumanday/OpenSymphony".to_string())
        );
        assert_eq!(normalize_github_repo_slug("/kumanday"), None);
        assert_eq!(
            normalize_github_repo_slug("/kumanday/OpenSymphony/extra"),
            None
        );
    }

    #[test]
    fn shell_single_quote_escapes_embedded_single_quotes() {
        assert_eq!(shell_single_quote("abc'def"), "'abc'\\''def'");
    }

    #[test]
    fn llm_defaults_match_fireworks_examples() {
        assert_eq!(
            DEFAULT_LLM_MODEL,
            "openai/accounts/fireworks/models/glm-5p1"
        );
        assert_eq!(
            DEFAULT_LLM_BASE_URL,
            "https://api.fireworks.ai/inference/v1"
        );
    }

    #[test]
    fn missing_llm_env_prompts_render_fireworks_defaults() {
        let env = StubEnvironment {
            values: BTreeMap::new(),
        };
        let mut output = Vec::new();
        let mut ui = PromptUi::new(&b"\n\n\n"[..], &mut output);

        prompt_for_missing_llm_env(&env, &mut ui, &InitArgs::default())
            .expect("prompt should succeed");

        let rendered = String::from_utf8(output).expect("prompt output should be utf-8");
        assert!(rendered.contains("LLM_MODEL is not set."));
        assert!(rendered.contains("openai/accounts/fireworks/models/glm-5p1"));
        assert!(rendered.contains("https://api.fireworks.ai/inference/v1"));
        assert!(rendered.contains("export LLM_API_KEY='<your-llm-api-key>'"));
    }

    #[test]
    fn non_interactive_llm_export_hints_ignore_empty_flag_overrides() {
        let env = StubEnvironment {
            values: BTreeMap::new(),
        };
        let args = InitArgs {
            non_interactive: true,
            llm_model: Some("   ".to_string()),
            llm_api_key_placeholder: Some(String::new()),
            llm_base_url: Some("\t".to_string()),
            ..InitArgs::default()
        };
        let mut output = Vec::new();
        let mut ui = PromptUi::new(&b""[..], &mut output);

        prompt_for_missing_llm_env(&env, &mut ui, &args).expect("prompt should succeed");

        let rendered = String::from_utf8(output).expect("prompt output should be utf-8");
        assert!(rendered.contains(&format!("export LLM_MODEL='{}'", DEFAULT_LLM_MODEL)));
        assert!(rendered.contains("export LLM_API_KEY='<your-llm-api-key>'"));
        assert!(rendered.contains(&format!("export LLM_BASE_URL='{}'", DEFAULT_LLM_BASE_URL)));
        assert!(!rendered.contains("export LLM_MODEL=''"));
        assert!(!rendered.contains("export LLM_API_KEY=''"));
        assert!(!rendered.contains("export LLM_BASE_URL=''"));
    }

    #[test]
    fn prompt_yes_no_accepts_blank_as_default() {
        let mut output = Vec::new();
        let mut ui = PromptUi::new(&b"\n"[..], &mut output);

        let accepted =
            prompt_yes_no(&mut ui, "Enable? [y/N]: ", false).expect("prompt should succeed");

        assert!(!accepted);
    }

    #[test]
    fn template_fetch_timeout_uses_default_and_override() {
        assert_eq!(
            template_fetch_timeout_from_env(None),
            Duration::from_millis(DEFAULT_TEMPLATE_FETCH_TIMEOUT_MS)
        );
        assert_eq!(
            template_fetch_timeout_from_env(Some("250")),
            Duration::from_millis(250)
        );
        assert_eq!(
            template_fetch_timeout_from_env(Some("not-a-number")),
            Duration::from_millis(DEFAULT_TEMPLATE_FETCH_TIMEOUT_MS)
        );
    }

    #[test]
    fn prompt_ai_review_config_supports_non_fireworks_provider_defaults() {
        let input = b"litellm-native\nopenai/gpt-5.4\ncustom\nn\n";
        let mut output = Vec::new();
        let mut ui = PromptUi::new(&input[..], &mut output);

        let config = prompt_ai_review_config(&mut ui).expect("prompt should succeed");

        assert_eq!(config.provider_kind, "litellm-native");
        assert_eq!(config.model_id, "openai/gpt-5.4");
        assert_eq!(config.base_url, None);
        assert_eq!(config.style, "custom");
        assert!(!config.require_evidence);
    }

    #[test]
    fn init_args_reject_litellm_native_base_url_override() {
        let args = InitArgs {
            ai_review_provider_kind: Some(AiReviewProviderKindArg::LitellmNative),
            ai_review_base_url: Some("https://example.com/unused".to_string()),
            ..InitArgs::default()
        };

        let error = args.validate().expect_err("base URL should be rejected");

        assert!(matches!(
            error,
            InitCommandError::InvalidArgument(message)
                if message.contains("--ai-review-base-url can only be used")
        ));
    }

    #[test]
    fn ai_review_config_from_flags_ignores_empty_string_overrides() {
        let args = InitArgs {
            ai_review_provider_kind: Some(AiReviewProviderKindArg::OpenaiCompatible),
            ai_review_model_id: Some(" ".to_string()),
            ai_review_base_url: Some(String::new()),
            ai_review_style: Some("\t".to_string()),
            ..InitArgs::default()
        };

        args.validate()
            .expect("empty string overrides should fall back to defaults");
        assert_eq!(
            args.ai_review_config_from_flags(),
            AiReviewConfig::default()
        );
    }

    #[test]
    fn ai_review_request_detection_comes_from_config_plan() {
        let args = InitArgs {
            ai_review_model_id: Some(DEFAULT_AI_REVIEW_MODEL_ID.to_string()),
            ..InitArgs::default()
        };

        assert!(
            args.ai_pr_review_requested_by_flags(),
            "an explicit AI review config flag should request scaffolding even when it resolves to the default config"
        );
        assert_eq!(
            args.ai_review_config_from_flags(),
            AiReviewConfig::default()
        );
    }

    #[test]
    fn ai_review_config_from_flags_trims_non_empty_overrides() {
        let args = InitArgs {
            ai_review_provider_kind: Some(AiReviewProviderKindArg::OpenaiCompatible),
            ai_review_model_id: Some(" custom-model ".to_string()),
            ai_review_base_url: Some(" https://example.com/v1 ".to_string()),
            ai_review_style: Some(" concise ".to_string()),
            ai_review_require_evidence: Some(false),
            ..InitArgs::default()
        };

        let config = args.ai_review_config_from_flags();

        assert_eq!(config.provider_kind, "openai-compatible");
        assert_eq!(config.model_id, "custom-model");
        assert_eq!(config.base_url.as_deref(), Some("https://example.com/v1"));
        assert_eq!(config.style, "concise");
        assert!(!config.require_evidence);
    }

    #[test]
    fn ai_review_secret_env_rejects_empty_values() {
        let env = StubEnvironment {
            values: BTreeMap::from([("AI_SECRET".to_string(), "   ".to_string())]),
        };
        let mut output = Vec::new();
        let mut ui = PromptUi::new(&b""[..], &mut output);
        let args = InitArgs {
            non_interactive: true,
            ai_review_secret_env: Some("AI_SECRET".to_string()),
            ..InitArgs::default()
        };

        let error = resolve_ai_review_secret(&mut ui, &env, &args)
            .expect_err("empty secret env should fail");

        assert!(matches!(
            error,
            InitCommandError::MissingEnvVar { name, flag }
                if name == "AI_SECRET" && flag == "--ai-review-secret-env"
        ));
    }

    #[test]
    fn ai_review_secret_reuse_rejects_empty_llm_api_key() {
        let env = StubEnvironment {
            values: BTreeMap::from([("LLM_API_KEY".to_string(), String::new())]),
        };
        let mut output = Vec::new();
        let mut ui = PromptUi::new(&b""[..], &mut output);
        let args = InitArgs {
            non_interactive: true,
            reuse_llm_api_key_for_ai_review_secret: true,
            ..InitArgs::default()
        };

        let error = resolve_ai_review_secret(&mut ui, &env, &args)
            .expect_err("empty reused LLM_API_KEY should fail");

        assert!(matches!(
            error,
            InitCommandError::MissingEnvVar { name, flag }
                if name == "LLM_API_KEY"
                    && flag == "--reuse-llm-api-key-for-ai-review-secret"
        ));
    }

    #[test]
    fn custom_codereview_guide_contains_starter_skill_metadata() {
        let guide = custom_codereview_guide_contents();

        assert!(guide.contains("name: custom-codereview-guide"));
        assert!(guide.contains("Default Priorities"));
        assert!(guide.contains("Evidence Expectations"));
    }
}
