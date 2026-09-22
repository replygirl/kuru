//! Execution-time permission decisions and private-grant contracts.
//!
//! This module deliberately knows nothing about a terminal UI. A foreground
//! caller may lend one operation a bounded reply channel; unattended callers
//! receive a typed `PermissionRequired` outcome instead of waiting for a
//! process-wide prompt handler.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use kuru_core::{
    Config, ManifestDigest, NativeTool, PermissionAction, PermissionSelector, ProjectRelativeTarget,
};
use kuru_platform::fs::Directory;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

const MAX_GRANTS: usize = 128;
const MAX_DISPLAY_BYTES: usize = 512;

/// Immutable facts used to authorize one dispatch. Its arguments are private
/// and borrowed: the exact value remains held through the request-specific
/// approval reply and is then passed to the tool transport.
pub struct PermissionInvocation<'a> {
    selector: PermissionSelector,
    target: Option<ProjectRelativeTarget>,
    arguments: &'a Value,
}

impl<'a> PermissionInvocation<'a> {
    pub fn new(
        selector: PermissionSelector,
        target: Option<ProjectRelativeTarget>,
        arguments: &'a Value,
    ) -> Result<Self> {
        selector.validate()?;
        match (selector.is_file(), target.as_ref()) {
            (true, Some(_)) | (false, None) => {}
            (true, None) => anyhow::bail!("file permission invocation lacks a validated target"),
            (false, Some(_)) => anyhow::bail!("non-file permission invocation has a file target"),
        }
        ensure!(arguments.is_object(), "tool arguments must be an object");
        Ok(Self {
            selector,
            target,
            arguments,
        })
    }

    pub fn selector(&self) -> &PermissionSelector {
        &self.selector
    }

    pub fn target(&self) -> Option<&ProjectRelativeTarget> {
        self.target.as_ref()
    }

    fn display(&self) -> PermissionDisplay {
        let label = bounded_redacted(&selector_label(&self.selector));
        let exact_scope = match self.target.as_ref() {
            Some(target) => format!("project file {}", target.as_str()),
            None => selector_label(&self.selector),
        };
        let exact_display = lossless_display(&exact_scope);
        let scope = exact_display
            .clone()
            .unwrap_or_else(|| bounded_redacted(&exact_scope));
        let preview = bounded_redacted(&serde_json::to_string(self.arguments).unwrap_or_default());
        PermissionDisplay {
            label,
            scope,
            preview,
            rememberable: exact_display.is_some(),
            remember_disabled_reason: exact_display
                .is_none()
                .then_some("The exact grant scope cannot be shown safely and completely.".into()),
        }
    }
}

/// Binding shared by every persistent grant for one reviewed workspace view.
/// Digest strings are intentionally serialization-friendly for the app-owned
/// private store, but `validate` rejects malformed externally read records.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionBinding {
    root_path_digest: String,
    root_identity: String,
    manifest_digest: String,
    context_digest: String,
}

impl PermissionBinding {
    /// Bind grants to the held root identity, its exact canonical spelling, the
    /// complete reviewed manifest, and every effective permission route/fallback.
    pub fn checked(root: &Directory, manifest: ManifestDigest, config: &Config) -> Result<Self> {
        root.revalidate()
            .context("workspace changed before permission binding")?;
        let context = permission_context_digest(config)?;
        let binding = Self {
            root_path_digest: digest(
                "kuru.permission.root-path",
                root.path().as_os_str().as_encoded_bytes(),
            ),
            root_identity: hex(&root.identity().to_bytes()),
            manifest_digest: manifest.to_string(),
            context_digest: context,
        };
        binding.validate()?;
        Ok(binding)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            is_lower_hex(&self.root_path_digest, 64)
                && is_lower_hex(&self.root_identity, 48)
                && is_lower_hex(&self.manifest_digest, 64)
                && is_lower_hex(&self.context_digest, 64),
            "permission grant binding is malformed"
        );
        Ok(())
    }

    /// Check both the retained native identity and canonical root spelling.
    pub fn validate_root(&self, root: &Directory) -> Result<()> {
        self.validate()?;
        root.revalidate()
            .context("workspace changed while permission binding was checked")?;
        ensure!(
            self.root_path_digest
                == digest(
                    "kuru.permission.root-path",
                    root.path().as_os_str().as_encoded_bytes(),
                )
                && self.root_identity == hex(&root.identity().to_bytes()),
            "permission binding belongs to a different workspace root"
        );
        Ok(())
    }

    fn unattended(root: &Directory, config: &Config) -> Result<Self> {
        root.revalidate()
            .context("workspace changed before unattended permission binding")?;
        let binding = Self {
            root_path_digest: digest(
                "kuru.permission.root-path",
                root.path().as_os_str().as_encoded_bytes(),
            ),
            root_identity: hex(&root.identity().to_bytes()),
            manifest_digest: digest("kuru.permission.unreviewed-manifest", b""),
            context_digest: permission_context_digest(config)?,
        };
        binding.validate()?;
        Ok(binding)
    }
}

/// A remembered grant is either one exact checked file or a named non-file
/// tool. It never contains a command, JSON arguments, or an A2A message.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum GrantScope {
    ExactFile {
        selector: PermissionSelector,
        target: ProjectRelativeTarget,
    },
    WholeTool {
        selector: PermissionSelector,
    },
}

impl Serialize for GrantScope {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        GrantScopeRecord::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GrantScope {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let record = GrantScopeRecord::deserialize(deserializer)?;
        Self::try_from(record).map_err(serde::de::Error::custom)
    }
}

impl GrantScope {
    pub fn for_invocation(invocation: &PermissionInvocation<'_>) -> Result<Self> {
        let scope = if let Some(target) = invocation.target() {
            Self::ExactFile {
                selector: invocation.selector().clone(),
                target: target.clone(),
            }
        } else {
            Self::WholeTool {
                selector: invocation.selector().clone(),
            }
        };
        scope.validate()?;
        Ok(scope)
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::ExactFile { selector, target } => {
                selector.validate()?;
                ensure!(
                    selector.is_file(),
                    "exact-file grant requires a native file tool"
                );
                ProjectRelativeTarget::parse(target.as_str().to_owned())?;
            }
            Self::WholeTool { selector } => {
                selector.validate()?;
                ensure!(
                    !selector.is_file(),
                    "whole-tool grant cannot widen a native file tool"
                );
            }
        }
        Ok(())
    }

    pub fn selector(&self) -> &PermissionSelector {
        match self {
            Self::ExactFile { selector, .. } | Self::WholeTool { selector } => selector,
        }
    }

    pub fn target(&self) -> Option<&ProjectRelativeTarget> {
        match self {
            Self::ExactFile { target, .. } => Some(target),
            Self::WholeTool { .. } => None,
        }
    }
}

/// Serialization form used by the app-owned store. Conversion re-parses the
/// target so a checked store never revives a malformed scope from disk.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GrantScopeRecord {
    ExactFile {
        selector: PermissionSelector,
        target: String,
    },
    WholeTool {
        selector: PermissionSelector,
    },
}

impl From<&GrantScope> for GrantScopeRecord {
    fn from(scope: &GrantScope) -> Self {
        match scope {
            GrantScope::ExactFile { selector, target } => Self::ExactFile {
                selector: selector.clone(),
                target: target.as_str().into(),
            },
            GrantScope::WholeTool { selector } => Self::WholeTool {
                selector: selector.clone(),
            },
        }
    }
}

impl TryFrom<GrantScopeRecord> for GrantScope {
    type Error = anyhow::Error;

    fn try_from(record: GrantScopeRecord) -> Result<Self> {
        let scope = match record {
            GrantScopeRecord::ExactFile { selector, target } => Self::ExactFile {
                selector,
                target: ProjectRelativeTarget::parse(target)?,
            },
            GrantScopeRecord::WholeTool { selector } => Self::WholeTool { selector },
        };
        scope.validate()?;
        Ok(scope)
    }
}

/// A private-store record. The binding is repeated deliberately so a checked
/// reader can reject a record copied from a different workspace or authority
/// context before it ever contributes a grant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistentGrant {
    pub binding: PermissionBinding,
    pub scope: GrantScope,
}

impl PersistentGrant {
    pub fn validate(&self) -> Result<()> {
        self.binding.validate()?;
        self.scope.validate()
    }
}

/// App-owned checked private storage. Implementations must bound their record
/// counts and bytes, and lock every operation; `add` and `revoke` independently
/// read, modify, and publish to avoid dropping concurrent TUI changes.
pub trait PermissionGrantStore: Send + Sync {
    fn load(&self, binding: &PermissionBinding) -> Result<Vec<PersistentGrant>>;
    fn add(&self, grant: &PersistentGrant) -> Result<()>;
    fn revoke(&self, grant: &PersistentGrant) -> Result<bool>;
}

/// A foreground-only bounded approval endpoint. The service never stores one:
/// a caller supplies it for the individual foreground operation it controls.
#[derive(Clone)]
pub struct ApprovalSender {
    sender: mpsc::Sender<ApprovalRequest>,
}

impl ApprovalSender {
    pub fn new(sender: mpsc::Sender<ApprovalRequest>) -> Self {
        Self { sender }
    }

    async fn ask(&self, display: PermissionDisplay, scope: GrantScope) -> PermissionOutcome {
        let (reply, receive) = oneshot::channel();
        if self
            .sender
            .try_send(ApprovalRequest {
                display,
                scope,
                reply,
            })
            .is_err()
        {
            return PermissionOutcome::PermissionRequired;
        }
        match receive.await {
            Ok(ApprovalAnswer::Once) => PermissionOutcome::OnceAuthorized,
            Ok(ApprovalAnswer::Session) => PermissionOutcome::SessionAuthorized,
            Ok(ApprovalAnswer::Always) => PermissionOutcome::PersistentAuthorized,
            Ok(ApprovalAnswer::Deny) => PermissionOutcome::Denied,
            Err(_) => PermissionOutcome::PermissionRequired,
        }
    }
}

pub struct ApprovalRequest {
    pub display: PermissionDisplay,
    pub scope: GrantScope,
    pub reply: oneshot::Sender<ApprovalAnswer>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalAnswer {
    Once,
    Session,
    Always,
    Deny,
}

/// This is deliberately a display-only projection. It is bounded and redacted;
/// policy and grants use `PermissionInvocation`, never this text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionDisplay {
    pub label: String,
    pub scope: String,
    pub preview: String,
    /// Session/persistent choices are valid only when `scope` is complete and
    /// unredacted, exactly matching the scope the service would remember.
    pub rememberable: bool,
    pub remember_disabled_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionOutcome {
    Authorized,
    /// The caller must immediately dispatch the still-bound invocation. There
    /// is no transferable token and a retry requires a fresh approval.
    OnceAuthorized,
    SessionAuthorized,
    PersistentAuthorized,
    Denied,
    PermissionRequired,
}

impl PermissionOutcome {
    pub fn is_authorized(self) -> bool {
        matches!(
            self,
            Self::Authorized
                | Self::OnceAuthorized
                | Self::SessionAuthorized
                | Self::PersistentAuthorized
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantInspection {
    pub session: Vec<GrantScope>,
    pub persistent: Vec<GrantScope>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrantRevocation {
    pub session_removed: bool,
    pub persistent_removed: bool,
}

/// Connector-owned evaluator. The app supplies the only persistent backend;
/// runtime receives this service but no alternative store or global prompt path.
pub struct PermissionService {
    config: Config,
    binding: PermissionBinding,
    store: Arc<dyn PermissionGrantStore>,
    session: Mutex<BTreeSet<GrantScope>>,
}

impl PermissionService {
    pub fn new(
        config: Config,
        binding: PermissionBinding,
        store: Arc<dyn PermissionGrantStore>,
    ) -> Result<Self> {
        binding.validate()?;
        ensure!(
            binding.context_digest == permission_context_digest(&config)?,
            "permission binding does not match the effective tool-route context"
        );
        Ok(Self {
            config,
            binding,
            store,
            session: Mutex::new(BTreeSet::new()),
        })
    }

    /// Explicit no-persistence compatibility service. It owns no sender and
    /// cannot read or write a grant record; foreground apps use `Self::new`.
    pub fn unattended(root: &Directory, config: Config) -> Result<Self> {
        let binding = PermissionBinding::unattended(root, &config)?;
        Self::new(config, binding, Arc::new(UnattendedGrantStore))
    }

    pub fn binding(&self) -> &PermissionBinding {
        &self.binding
    }

    /// Verify the immutable service against the retained root and effective
    /// configuration before a caller installs it in a ToolHost.
    pub fn validate_context(&self, root: &Directory, config: &Config) -> Result<()> {
        self.binding.validate_root(root)?;
        ensure!(
            self.binding.context_digest == permission_context_digest(config)?,
            "permission service does not match the effective tool-route context"
        );
        Ok(())
    }

    /// Confirm that the endpoint about to receive an outbound A2A request is
    /// the exact endpoint included in this immutable service's route context.
    pub fn validate_a2a_route(&self, alias: &str, endpoint: &str) -> Result<()> {
        let configured = self
            .config
            .external_agents
            .get(alias)
            .context("external agent alias is not configured in the permission service")?;
        ensure!(
            configured == endpoint,
            "outbound A2A endpoint does not match the reviewed permission route"
        );
        Ok(())
    }

    /// A path-scoped rule cannot hide a whole file tool because another checked
    /// target may still be allowed or requestable. Whole-tool deny does hide it.
    pub fn advertises(&self, selector: &PermissionSelector) -> bool {
        if selector.validate().is_err() {
            return false;
        }
        if selector.is_file() {
            return !self.config.permissions.iter().any(|rule| {
                rule.selector == *selector
                    && rule.path.is_none()
                    && rule.action == PermissionAction::Deny
            });
        }
        self.config.permission_decision(selector, None) != PermissionAction::Deny
    }

    pub async fn authorize(
        &self,
        invocation: &PermissionInvocation<'_>,
        approval: Option<&ApprovalSender>,
    ) -> Result<PermissionOutcome> {
        let scope = GrantScope::for_invocation(invocation)?;
        match self
            .config
            .permission_decision(invocation.selector(), invocation.target())
        {
            PermissionAction::Deny => return Ok(PermissionOutcome::Denied),
            PermissionAction::Allow => return Ok(PermissionOutcome::Authorized),
            PermissionAction::Ask => {}
        }

        if self.has_session_grant(&scope)? || self.has_persistent_grant(&scope)? {
            return Ok(PermissionOutcome::Authorized);
        }
        let Some(approval) = approval else {
            return Ok(PermissionOutcome::PermissionRequired);
        };
        let display = invocation.display();
        let outcome = approval.ask(display.clone(), scope.clone()).await;
        match outcome {
            // The request-bound oneshot corresponds to this immutable
            // invocation; there is no transferable approval token or retry.
            PermissionOutcome::OnceAuthorized => Ok(outcome),
            PermissionOutcome::SessionAuthorized => {
                if !display.rememberable {
                    return Ok(PermissionOutcome::PermissionRequired);
                }
                let mut session = self
                    .session
                    .lock()
                    .map_err(|_| anyhow::anyhow!("permission session state is unavailable"))?;
                ensure!(
                    session.len() < MAX_GRANTS || session.contains(&scope),
                    "permission session grant limit reached"
                );
                session.insert(scope);
                Ok(outcome)
            }
            PermissionOutcome::PersistentAuthorized => {
                if !display.rememberable {
                    return Ok(PermissionOutcome::PermissionRequired);
                }
                self.store.add(&PersistentGrant {
                    binding: self.binding.clone(),
                    scope,
                })?;
                Ok(outcome)
            }
            PermissionOutcome::Denied | PermissionOutcome::PermissionRequired => Ok(outcome),
            _ => unreachable!("approval channel only yields approval outcomes"),
        }
    }

    pub fn reset_session(&self) -> Result<()> {
        self.session
            .lock()
            .map_err(|_| anyhow::anyhow!("permission session state is unavailable"))?
            .clear();
        Ok(())
    }

    pub fn inspect(&self) -> Result<GrantInspection> {
        let session = self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("permission session state is unavailable"))?
            .iter()
            .cloned()
            .collect();
        let persistent = self.checked_persistent_grants()?;
        Ok(GrantInspection {
            session,
            persistent,
        })
    }

    pub fn revoke(&self, scope: &GrantScope) -> Result<GrantRevocation> {
        scope.validate()?;
        let session_removed = self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("permission session state is unavailable"))?
            .remove(scope);
        let persistent_removed = self.store.revoke(&PersistentGrant {
            binding: self.binding.clone(),
            scope: scope.clone(),
        })?;
        Ok(GrantRevocation {
            session_removed,
            persistent_removed,
        })
    }

    fn has_session_grant(&self, scope: &GrantScope) -> Result<bool> {
        Ok(self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("permission session state is unavailable"))?
            .contains(scope))
    }

    fn has_persistent_grant(&self, scope: &GrantScope) -> Result<bool> {
        Ok(self.checked_persistent_grants()?.contains(scope))
    }

    fn checked_persistent_grants(&self) -> Result<Vec<GrantScope>> {
        self.binding.validate()?;
        let grants = self.store.load(&self.binding)?;
        ensure!(
            grants.len() <= MAX_GRANTS,
            "permission grant storage exceeds its record limit"
        );
        for grant in &grants {
            grant.validate()?;
            ensure!(
                grant.binding == self.binding,
                "permission grant storage returned a mismatched binding"
            );
        }
        Ok(grants.into_iter().map(|grant| grant.scope).collect())
    }
}

struct UnattendedGrantStore;

impl PermissionGrantStore for UnattendedGrantStore {
    fn load(&self, _binding: &PermissionBinding) -> Result<Vec<PersistentGrant>> {
        Ok(Vec::new())
    }

    fn add(&self, _grant: &PersistentGrant) -> Result<()> {
        anyhow::bail!("persistent permission grants require a foreground app store")
    }

    fn revoke(&self, _grant: &PersistentGrant) -> Result<bool> {
        Ok(false)
    }
}

fn selector_label(selector: &PermissionSelector) -> String {
    match selector {
        PermissionSelector::Native { name } => format!("native {}", native_label(*name)),
        PermissionSelector::Mcp { alias, tool } => format!("MCP {alias}/{tool}"),
        PermissionSelector::A2a { alias } => format!("external agent {alias}"),
    }
}

const fn native_label(name: NativeTool) -> &'static str {
    match name {
        NativeTool::FileRead => "file read",
        NativeTool::FileList => "file list",
        NativeTool::Grep => "grep",
        NativeTool::Glob => "glob",
        NativeTool::FileWrite => "file write",
        NativeTool::FileDelete => "file delete",
        NativeTool::Shell => "shell",
    }
}

fn bounded_redacted(value: &str) -> String {
    let redacted = crate::redaction::text(value).unwrap_or_else(|_| "[withheld]".into());
    if redacted.len() <= MAX_DISPLAY_BYTES {
        return redacted;
    }
    let mut end = MAX_DISPLAY_BYTES;
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &redacted[..end])
}

fn lossless_display(value: &str) -> Option<String> {
    (value.len() <= MAX_DISPLAY_BYTES)
        .then(|| crate::redaction::text(value).ok())
        .flatten()
        .filter(|display| display == value)
}

fn digest(domain: &str, bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(domain.as_bytes());
    hash.update([0]);
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    hex(&hash.finalize())
}

fn permission_context_digest(config: &Config) -> Result<String> {
    let encoded = serde_json::to_vec(&(
        &config.permissions,
        &config.mcp,
        &config.external_agents,
        config.allow_write,
        config.allow_shell,
    ))
    .context("permission context cannot be encoded")?;
    Ok(digest("kuru.permission.context", &encoded))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;
    use kuru_core::{ConfigSnapshot, InvocationOverrides, PermissionRule};
    use kuru_platform::fs::{NameRetention, Privacy};
    use tempfile::TempDir;

    #[derive(Default)]
    struct MemoryStore(StdMutex<Vec<PersistentGrant>>);

    impl PermissionGrantStore for MemoryStore {
        fn load(&self, binding: &PermissionBinding) -> Result<Vec<PersistentGrant>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|grant| &grant.binding == binding)
                .cloned()
                .collect())
        }

        fn add(&self, grant: &PersistentGrant) -> Result<()> {
            grant.validate()?;
            let mut grants = self.0.lock().unwrap();
            if !grants.contains(grant) {
                grants.push(grant.clone());
            }
            Ok(())
        }

        fn revoke(&self, grant: &PersistentGrant) -> Result<bool> {
            let mut grants = self.0.lock().unwrap();
            let before = grants.len();
            grants.retain(|stored| stored != grant);
            Ok(before != grants.len())
        }
    }

    fn fixture() -> (TempDir, Directory, Config, PermissionBinding) {
        let directory = tempfile::tempdir().unwrap();
        let root =
            Directory::open(directory.path(), Privacy::Inherited, NameRetention::Pinned).unwrap();
        let config = Config::default();
        let snapshot =
            ConfigSnapshot::parse(None, directory.path(), None, InvocationOverrides::default())
                .unwrap();
        let binding =
            PermissionBinding::checked(&root, snapshot.manifest().full_digest(), &config).unwrap();
        (directory, root, config, binding)
    }

    fn file_write() -> PermissionSelector {
        PermissionSelector::native(NativeTool::FileWrite)
    }

    #[test]
    fn scope_records_reparse_checked_exact_file_targets() {
        let scope = GrantScope::ExactFile {
            selector: file_write(),
            target: ProjectRelativeTarget::parse("notes/plan.md").unwrap(),
        };
        let encoded = serde_json::to_string(&scope).unwrap();
        assert_eq!(serde_json::from_str::<GrantScope>(&encoded).unwrap(), scope);
        assert!(serde_json::from_str::<GrantScope>(r#"{"kind":"exact_file","selector":{"kind":"native","name":"file_write"},"target":"../outside"}"#).is_err());
        assert!(
            GrantScope::WholeTool {
                selector: file_write()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn binding_rejects_an_effective_fallback_change() {
        let (_directory, _root, mut config, binding) = fixture();
        config.allow_shell = true;
        assert!(PermissionService::new(config, binding, Arc::new(MemoryStore::default())).is_err());
    }

    #[tokio::test]
    async fn explicit_deny_wins_even_when_a_persistent_grant_exists() {
        let (_directory, _root, mut config, _binding) = fixture();
        config.permissions = vec![PermissionRule {
            action: PermissionAction::Deny,
            selector: file_write(),
            path: None,
        }];
        let binding = PermissionBinding::checked(
            &_root,
            ConfigSnapshot::parse(
                None,
                _directory.path(),
                None,
                InvocationOverrides::default(),
            )
            .unwrap()
            .manifest()
            .full_digest(),
            &config,
        )
        .unwrap();
        let store = Arc::new(MemoryStore::default());
        let scope = GrantScope::ExactFile {
            selector: file_write(),
            target: ProjectRelativeTarget::parse("notes/plan.md").unwrap(),
        };
        store
            .add(&PersistentGrant {
                binding: binding.clone(),
                scope: scope.clone(),
            })
            .unwrap();
        let service = PermissionService::new(config, binding, store).unwrap();
        let arguments = serde_json::json!({"path":"notes/plan.md","content":"changed"});
        let invocation =
            PermissionInvocation::new(file_write(), scope.target().cloned(), &arguments).unwrap();
        assert_eq!(
            service.authorize(&invocation, None).await.unwrap(),
            PermissionOutcome::Denied
        );
    }

    #[tokio::test]
    async fn session_answer_is_bound_to_the_exact_file_scope() {
        let (_directory, _root, mut config, _binding) = fixture();
        config.permissions = vec![PermissionRule {
            action: PermissionAction::Ask,
            selector: file_write(),
            path: None,
        }];
        let binding = PermissionBinding::checked(
            &_root,
            ConfigSnapshot::parse(
                None,
                _directory.path(),
                None,
                InvocationOverrides::default(),
            )
            .unwrap()
            .manifest()
            .full_digest(),
            &config,
        )
        .unwrap();
        let service =
            PermissionService::new(config, binding, Arc::new(MemoryStore::default())).unwrap();
        let (sender, mut receiver) = mpsc::channel::<ApprovalRequest>(1);
        let response = tokio::spawn(async move {
            let request = receiver.recv().await.unwrap();
            assert_eq!(request.scope.target().unwrap().as_str(), "notes/one.md");
            request.reply.send(ApprovalAnswer::Session).unwrap();
        });
        let first_args = serde_json::json!({"path":"notes/one.md","content":"one"});
        let first = PermissionInvocation::new(
            file_write(),
            Some(ProjectRelativeTarget::parse("notes/one.md").unwrap()),
            &first_args,
        )
        .unwrap();
        assert_eq!(
            service
                .authorize(&first, Some(&ApprovalSender::new(sender)))
                .await
                .unwrap(),
            PermissionOutcome::SessionAuthorized
        );
        response.await.unwrap();
        let second_args = serde_json::json!({"path":"notes/two.md","content":"two"});
        let second = PermissionInvocation::new(
            file_write(),
            Some(ProjectRelativeTarget::parse("notes/two.md").unwrap()),
            &second_args,
        )
        .unwrap();
        assert_eq!(
            service.authorize(&second, None).await.unwrap(),
            PermissionOutcome::PermissionRequired
        );
        service.reset_session().unwrap();
        assert!(service.inspect().unwrap().session.is_empty());
    }

    #[tokio::test]
    async fn hidden_scope_cannot_be_remembered_but_can_be_denied() {
        let (_directory, _root, mut config, _binding) = fixture();
        config.permissions = vec![PermissionRule {
            action: PermissionAction::Ask,
            selector: file_write(),
            path: None,
        }];
        let binding = PermissionBinding::checked(
            &_root,
            ConfigSnapshot::parse(
                None,
                _directory.path(),
                None,
                InvocationOverrides::default(),
            )
            .unwrap()
            .manifest()
            .full_digest(),
            &config,
        )
        .unwrap();
        let service =
            PermissionService::new(config, binding, Arc::new(MemoryStore::default())).unwrap();
        let target = format!("notes/{}.md", "a".repeat(MAX_DISPLAY_BYTES));
        let arguments = serde_json::json!({"path":target,"content":"changed"});
        let invocation = PermissionInvocation::new(
            file_write(),
            Some(ProjectRelativeTarget::parse(arguments["path"].as_str().unwrap()).unwrap()),
            &arguments,
        )
        .unwrap();
        let (sender, mut receiver) = mpsc::channel::<ApprovalRequest>(1);
        let response = tokio::spawn(async move {
            let request = receiver.recv().await.unwrap();
            assert!(!request.display.rememberable);
            assert!(request.display.remember_disabled_reason.is_some());
            request.reply.send(ApprovalAnswer::Session).unwrap();
        });
        assert_eq!(
            service
                .authorize(&invocation, Some(&ApprovalSender::new(sender)))
                .await
                .unwrap(),
            PermissionOutcome::PermissionRequired
        );
        response.await.unwrap();
        assert!(service.inspect().unwrap().session.is_empty());

        let (sender, mut receiver) = mpsc::channel::<ApprovalRequest>(1);
        let response = tokio::spawn(async move {
            let request = receiver.recv().await.unwrap();
            request.reply.send(ApprovalAnswer::Deny).unwrap();
        });
        assert_eq!(
            service
                .authorize(&invocation, Some(&ApprovalSender::new(sender)))
                .await
                .unwrap(),
            PermissionOutcome::Denied
        );
        response.await.unwrap();
    }
}
