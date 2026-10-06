//! Fixed, bounded local observations for `kuru doctor`.

use std::path::Path;

use anyhow::Result;
use kuru_connectors::AuthManager;
use kuru_core::{AuthorityClaimCategory, ConfigSnapshot, ProjectPreferences};
use kuru_memory::{MemoryStore, OpenOptions, ProjectStructure};
use kuru_platform::fs::Directory;
use serde::Serialize;

use crate::{
    cli::{self, Cli},
    trust::{ApprovalState, ApprovalStore},
};

const SCHEMA_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckId {
    Invocation,
    ChatgptSubscription,
    ResponsesRoute,
    Configuration,
    WorkspaceTrust,
    Memory,
    EmbeddedEngine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Healthy,
    NotConfigured,
    Problem,
    Unverified,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Condition {
    CredentialsPresent,
    SignedOut,
    RouteVariablePresent,
    RouteKeyAbsent,
    RouteNotSelected,
    RouteUnverified,
    ConfigurationValid,
    ConfigurationInvalid,
    ConfigurationUnreadable,
    ApprovalNotRequired,
    Approved,
    Unapproved,
    ApprovalChanged,
    ApprovalInvalid,
    NotInitialized,
    ActivationInvalid,
    LiveOwnerChecked,
    ColdHealthUnchecked,
    InspectionUnavailable,
    PayloadConsistent,
    PayloadInvalid,
    InvocationInvalid,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    None,
    SignIn,
    ConfigureResponses,
    ReviewWorkspaceTrust,
    CheckConfiguration,
    OpenProjectNormally,
    ConsultTroubleshooting,
    RetryInspection,
    ReinstallKuru,
    CheckInvocation,
}

#[derive(Debug, Serialize)]
struct Finding {
    id: CheckId,
    state: State,
    condition: Condition,
    action: Action,
}

impl Finding {
    const fn new(id: CheckId, state: State, condition: Condition, action: Action) -> Self {
        Self {
            id,
            state,
            condition,
            action,
        }
    }
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u8,
    exit_code: i32,
    checks: Vec<Finding>,
}

impl Report {
    fn new(checks: Vec<Finding>) -> Self {
        let exit_code = if checks.iter().any(|check| check.state == State::Problem) {
            3
        } else if checks.iter().any(|check| check.state == State::Unverified) {
            2
        } else {
            0
        };
        Self {
            schema_version: SCHEMA_VERSION,
            exit_code,
            checks,
        }
    }

    fn print(&self, json: bool) -> Result<()> {
        if json {
            println!("{}", serde_json::to_string(self)?);
        } else {
            println!("Kuru doctor (schema {})", self.schema_version);
            for check in &self.checks {
                println!(
                    "{}: {} — {}. {}",
                    human_id(check.id),
                    human_state(check.state),
                    human_condition(check.condition),
                    human_action(check.action)
                );
            }
            println!(
                "Exit code {}: {}.",
                self.exit_code,
                match self.exit_code {
                    0 => "no actionable local issue was found",
                    2 => "one or more checks could not be completed locally",
                    3 => "one or more actionable local issues were found",
                    _ => "the diagnostic report could not be completed",
                }
            );
        }
        Ok(())
    }
}

fn human_id(value: CheckId) -> &'static str {
    match value {
        CheckId::ChatgptSubscription => "ChatGPT subscription",
        CheckId::ResponsesRoute => "Responses API route",
        CheckId::Configuration => "Configuration",
        CheckId::WorkspaceTrust => "Workspace trust",
        CheckId::Memory => "Project memory",
        CheckId::EmbeddedEngine => "Bundled Dolt engine",
        CheckId::Invocation => "Invocation",
    }
}

fn human_state(value: State) -> &'static str {
    match value {
        State::Healthy => "OK",
        State::NotConfigured => "Not configured or not selected",
        State::Problem => "Problem",
        State::Unverified => "Unverified",
    }
}

fn human_condition(value: Condition) -> &'static str {
    match value {
        Condition::CredentialsPresent => "Kuru sign-in credentials are present",
        Condition::SignedOut => "no Kuru sign-in is stored",
        Condition::RouteVariablePresent => {
            "the selected environment variable is non-empty; API access was not tested"
        }
        Condition::RouteKeyAbsent => "the selected environment variable is absent or empty",
        Condition::RouteNotSelected => "the Responses route is not selected",
        Condition::RouteUnverified => "route configuration awaits workspace review",
        Condition::ConfigurationValid => "configuration parsed and passed local validation",
        Condition::ConfigurationInvalid => "configuration is invalid",
        Condition::ConfigurationUnreadable => "configuration could not be inspected reliably",
        Condition::ApprovalNotRequired => "no automatic workspace authority requires approval",
        Condition::Approved => "the captured workspace manifest is approved",
        Condition::Unapproved => "the captured workspace manifest is not approved",
        Condition::ApprovalChanged => "stored approval does not match the captured manifest",
        Condition::ApprovalInvalid => "stored approval is invalid",
        Condition::NotInitialized => "project memory has not been activated",
        Condition::ActivationInvalid => "project activation metadata is invalid",
        Condition::LiveOwnerChecked => "an existing memory owner answered a read-only check",
        Condition::ColdHealthUnchecked => {
            "project metadata is valid; cold database health was not checked"
        }
        Condition::InspectionUnavailable => "the local check could not be completed",
        Condition::PayloadConsistent => "the embedded archive matches its target catalog",
        Condition::PayloadInvalid => "the embedded archive does not match its target catalog",
        Condition::InvocationInvalid => "the workspace or data directory could not be resolved",
    }
}

fn human_action(value: Action) -> &'static str {
    match value {
        Action::None => "No action is indicated.",
        Action::SignIn => "Run `kuru login` to sign in to the ChatGPT subscription route.",
        Action::ConfigureResponses => {
            "Configure the selected Responses API-key environment variable."
        }
        Action::ReviewWorkspaceTrust => {
            "Review the exact workspace manifest with `kuru trust status`."
        }
        Action::CheckConfiguration => "Correct the local configuration and run the doctor again.",
        Action::OpenProjectNormally => "Open the project normally to activate its memory store.",
        Action::ConsultTroubleshooting => {
            "See the troubleshooting guide before changing project memory."
        }
        Action::RetryInspection => {
            "Retry the local check after resolving the reported access issue."
        }
        Action::ReinstallKuru => "Reinstall Kuru from a verified release.",
        Action::CheckInvocation => {
            "Check `-C`, `--data-dir`, and the local data-directory environment."
        }
    }
}

#[derive(Debug)]
pub(crate) struct DoctorExit(pub i32);

impl std::fmt::Display for DoctorExit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "doctor report exit {}", self.0)
    }
}

impl std::error::Error for DoctorExit {}

pub(crate) async fn run_from_cli(cli: &Cli, json: bool) -> Result<()> {
    let (cwd, data, user) = match cli::paths(cli) {
        Ok(paths) => paths,
        Err(_) => return invocation_failure(json),
    };
    let root = match Directory::open(
        &cwd,
        kuru_platform::fs::Privacy::Inherited,
        kuru_platform::fs::NameRetention::Movable,
    ) {
        Ok(root) => root,
        Err(_) => return invocation_failure(json),
    };
    run(cli, &cwd, &data, user.as_deref(), &root, json).await
}

fn invocation_failure(json: bool) -> Result<()> {
    let report = Report {
        schema_version: SCHEMA_VERSION,
        exit_code: 1,
        checks: vec![Finding::new(
            CheckId::Invocation,
            State::Problem,
            Condition::InvocationInvalid,
            Action::CheckInvocation,
        )],
    };
    report.print(json)?;
    Err(DoctorExit(1).into())
}

pub(crate) async fn run(
    cli: &Cli,
    cwd: &Path,
    data: &Path,
    user: Option<&Path>,
    root: &Directory,
    json: bool,
) -> Result<()> {
    let mut checks = Vec::with_capacity(6);
    let captured = capture_configuration(cli, user, root).await;
    match captured {
        Ok((snapshot, chatgpt_selected)) => {
            checks.push(chatgpt_subscription(data, cwd, chatgpt_selected).await);
            if root.revalidate().is_err() {
                checks.push(Finding::new(
                    CheckId::Configuration,
                    State::Unverified,
                    Condition::ConfigurationUnreadable,
                    Action::RetryInspection,
                ));
                checks.push(Finding::new(
                    CheckId::WorkspaceTrust,
                    State::Unverified,
                    Condition::InspectionUnavailable,
                    Action::RetryInspection,
                ));
                checks.push(Finding::new(
                    CheckId::ResponsesRoute,
                    State::Unverified,
                    Condition::RouteUnverified,
                    Action::RetryInspection,
                ));
            } else {
                checks.push(Finding::new(
                    CheckId::Configuration,
                    State::Healthy,
                    Condition::ConfigurationValid,
                    Action::None,
                ));
                let trust = workspace_trust(data, root, &snapshot);
                let route_authorized = trust.route_authorized || cli.trust_workspace_once;
                checks.push(trust.finding);
                checks.push(responses_route(&snapshot, route_authorized));
            }
        }
        Err(ConfigurationFailure::Invalid) => {
            checks.push(chatgpt_subscription(data, cwd, false).await);
            checks.push(Finding::new(
                CheckId::Configuration,
                State::Problem,
                Condition::ConfigurationInvalid,
                Action::CheckConfiguration,
            ));
            checks.push(Finding::new(
                CheckId::WorkspaceTrust,
                State::Unverified,
                Condition::InspectionUnavailable,
                Action::RetryInspection,
            ));
            checks.push(Finding::new(
                CheckId::ResponsesRoute,
                State::Unverified,
                Condition::RouteUnverified,
                Action::RetryInspection,
            ));
        }
        Err(ConfigurationFailure::Unreadable) => {
            checks.push(chatgpt_subscription(data, cwd, false).await);
            checks.push(Finding::new(
                CheckId::Configuration,
                State::Unverified,
                Condition::ConfigurationUnreadable,
                Action::RetryInspection,
            ));
            checks.push(Finding::new(
                CheckId::WorkspaceTrust,
                State::Unverified,
                Condition::InspectionUnavailable,
                Action::RetryInspection,
            ));
            checks.push(Finding::new(
                CheckId::ResponsesRoute,
                State::Unverified,
                Condition::RouteUnverified,
                Action::RetryInspection,
            ));
        }
    }

    checks.push(memory(data, cwd).await);
    checks.push(if kuru_memory::provision::verify_embedded_asset().is_ok() {
        Finding::new(
            CheckId::EmbeddedEngine,
            State::Healthy,
            Condition::PayloadConsistent,
            Action::None,
        )
    } else {
        Finding::new(
            CheckId::EmbeddedEngine,
            State::Problem,
            Condition::PayloadInvalid,
            Action::ReinstallKuru,
        )
    });

    let report = Report::new(checks);
    report.print(json)?;
    match report.exit_code {
        0 => Ok(()),
        code => Err(DoctorExit(code).into()),
    }
}

enum ConfigurationFailure {
    Invalid,
    Unreadable,
}

async fn capture_configuration(
    cli: &Cli,
    user: Option<&Path>,
    root: &Directory,
) -> std::result::Result<(ConfigSnapshot, bool), ConfigurationFailure> {
    let local = cli::discovered_local(root)
        .await
        .map_err(|_| ConfigurationFailure::Unreadable)?;
    let managed = std::env::var_os("KURU_MANAGED_CONFIG").map(std::path::PathBuf::from);
    let user_prompt_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(cli::native_config_directory)
        .map(|path| path.join("kuru"));
    let snapshot = ConfigSnapshot::parse_with_sources(
        user,
        user_prompt_root.as_deref(),
        root.path(),
        local
            .as_ref()
            .map(|(path, content)| (path.as_path(), content.as_str())),
        cli.config.as_deref(),
        managed.as_deref(),
        &crate::commands::built_in_names(),
        cli::invocation_overrides(cli),
    )
    .map_err(classify_configuration_error)?;
    let config = snapshot
        .finalize(&ProjectPreferences::default())
        .map_err(classify_configuration_error)?;
    Ok((snapshot, config.provider == "codex"))
}

fn classify_configuration_error(error: anyhow::Error) -> ConfigurationFailure {
    let unreadable = error
        .chain()
        .any(|cause| cause.downcast_ref::<std::io::Error>().is_some())
        || error.to_string().contains("configuration read error");
    if unreadable {
        ConfigurationFailure::Unreadable
    } else {
        ConfigurationFailure::Invalid
    }
}

struct TrustObservation {
    finding: Finding,
    route_authorized: bool,
}

fn workspace_trust(data: &Path, root: &Directory, snapshot: &ConfigSnapshot) -> TrustObservation {
    let has_responses_claim = snapshot
        .manifest()
        .claims()
        .iter()
        .any(|claim| claim.category() == AuthorityClaimCategory::ResponsesRoute);
    if snapshot.manifest().claims().is_empty() {
        return TrustObservation {
            finding: Finding::new(
                CheckId::WorkspaceTrust,
                State::Healthy,
                Condition::ApprovalNotRequired,
                Action::None,
            ),
            route_authorized: !has_responses_claim,
        };
    }
    match ApprovalStore::new(data, root).inspect(snapshot.manifest()) {
        ApprovalState::Matching => TrustObservation {
            finding: Finding::new(
                CheckId::WorkspaceTrust,
                State::Healthy,
                Condition::Approved,
                Action::None,
            ),
            route_authorized: true,
        },
        ApprovalState::Absent => TrustObservation {
            finding: Finding::new(
                CheckId::WorkspaceTrust,
                State::NotConfigured,
                Condition::Unapproved,
                Action::ReviewWorkspaceTrust,
            ),
            route_authorized: !has_responses_claim,
        },
        ApprovalState::Stale => TrustObservation {
            finding: Finding::new(
                CheckId::WorkspaceTrust,
                State::NotConfigured,
                Condition::ApprovalChanged,
                Action::ReviewWorkspaceTrust,
            ),
            route_authorized: !has_responses_claim,
        },
        ApprovalState::Invalid => TrustObservation {
            finding: Finding::new(
                CheckId::WorkspaceTrust,
                State::Problem,
                Condition::ApprovalInvalid,
                Action::ReviewWorkspaceTrust,
            ),
            route_authorized: !has_responses_claim,
        },
        ApprovalState::Unreadable(_) => TrustObservation {
            finding: Finding::new(
                CheckId::WorkspaceTrust,
                State::Unverified,
                Condition::InspectionUnavailable,
                Action::RetryInspection,
            ),
            route_authorized: !has_responses_claim,
        },
    }
}

fn responses_route(snapshot: &ConfigSnapshot, authorized: bool) -> Finding {
    let route = match snapshot.responses_route() {
        Ok(Some(route)) => route,
        Ok(None) => {
            return Finding::new(
                CheckId::ResponsesRoute,
                State::NotConfigured,
                Condition::RouteNotSelected,
                Action::None,
            );
        }
        Err(_) => {
            return Finding::new(
                CheckId::ResponsesRoute,
                State::Unverified,
                Condition::RouteUnverified,
                Action::RetryInspection,
            );
        }
    };
    let requires_review = snapshot
        .manifest()
        .claims()
        .iter()
        .any(|claim| claim.category() == AuthorityClaimCategory::ResponsesRoute);
    if requires_review && !authorized {
        return Finding::new(
            CheckId::ResponsesRoute,
            State::Unverified,
            Condition::RouteUnverified,
            Action::ReviewWorkspaceTrust,
        );
    }
    if std::env::var_os(route.api_key_env()).is_some_and(|value| !value.is_empty()) {
        Finding::new(
            CheckId::ResponsesRoute,
            State::Healthy,
            Condition::RouteVariablePresent,
            Action::None,
        )
    } else {
        Finding::new(
            CheckId::ResponsesRoute,
            State::Problem,
            Condition::RouteKeyAbsent,
            Action::ConfigureResponses,
        )
    }
}

async fn chatgpt_subscription(data: &Path, cwd: &Path, selected: bool) -> Finding {
    let manager = match AuthManager::new(data.to_path_buf(), cwd.to_path_buf(), None) {
        Ok(manager) => manager,
        Err(_) => {
            return Finding::new(
                CheckId::ChatgptSubscription,
                State::Unverified,
                Condition::InspectionUnavailable,
                Action::RetryInspection,
            );
        }
    };
    match manager.status().await {
        Ok(status) if status.authenticated => Finding::new(
            CheckId::ChatgptSubscription,
            State::Healthy,
            Condition::CredentialsPresent,
            Action::None,
        ),
        Ok(_) if selected => Finding::new(
            CheckId::ChatgptSubscription,
            State::Problem,
            Condition::SignedOut,
            Action::SignIn,
        ),
        Ok(_) => Finding::new(
            CheckId::ChatgptSubscription,
            State::NotConfigured,
            Condition::SignedOut,
            Action::None,
        ),
        Err(_) => Finding::new(
            CheckId::ChatgptSubscription,
            State::Unverified,
            Condition::InspectionUnavailable,
            Action::RetryInspection,
        ),
    }
}

async fn memory(data: &Path, cwd: &Path) -> Finding {
    let id = CheckId::Memory;
    let Ok(scope) = kuru_runtime::project_scope(cwd) else {
        return Finding::new(
            id,
            State::Unverified,
            Condition::InspectionUnavailable,
            Action::RetryInspection,
        );
    };
    match MemoryStore::inspect_project_structure(data, &scope) {
        ProjectStructure::Absent => {
            return Finding::new(
                id,
                State::NotConfigured,
                Condition::NotInitialized,
                Action::OpenProjectNormally,
            );
        }
        ProjectStructure::InvalidActivation => {
            return Finding::new(
                id,
                State::Problem,
                Condition::ActivationInvalid,
                Action::ConsultTroubleshooting,
            );
        }
        ProjectStructure::Unverified => {
            return Finding::new(
                id,
                State::Unverified,
                Condition::InspectionUnavailable,
                Action::RetryInspection,
            );
        }
        ProjectStructure::Activated => {}
    }
    let mut options = OpenOptions::new(data.to_path_buf(), scope);
    options.read_only = true;
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(_) => {
            return Finding::new(
                id,
                State::Unverified,
                Condition::InspectionUnavailable,
                Action::RetryInspection,
            );
        }
    };
    match MemoryStore::attach_existing_for_inspection(options, cwd.to_path_buf(), executable).await
    {
        Ok(Some(store)) => {
            let observed = store.status().await;
            let closed = store.close().await;
            if observed.is_ok() && closed.is_ok() {
                Finding::new(
                    id,
                    State::Healthy,
                    Condition::LiveOwnerChecked,
                    Action::None,
                )
            } else {
                Finding::new(
                    id,
                    State::Unverified,
                    Condition::InspectionUnavailable,
                    Action::RetryInspection,
                )
            }
        }
        Ok(None) => Finding::new(
            id,
            State::Unverified,
            Condition::ColdHealthUnchecked,
            Action::OpenProjectNormally,
        ),
        Err(_) => Finding::new(
            id,
            State::Unverified,
            Condition::InspectionUnavailable,
            Action::RetryInspection,
        ),
    }
}
