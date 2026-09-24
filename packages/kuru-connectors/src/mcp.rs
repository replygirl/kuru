use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use futures::future::join_all;
use kuru_core::{McpConfig, McpOAuthConfig, PermissionSelector, ToolSpec};
use kuru_platform::fs::Directory;
#[cfg(test)]
use kuru_platform::fs::{NameRetention, Privacy};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, WWW_AUTHENTICATE};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify, RwLock};

use crate::{
    IO_TIMEOUT, MAX_BYTES, http,
    mcp_cache::{CachedMcpTool, McpCatalogStore},
    mcp_credentials::{
        McpCredentialGeneration, McpCredentialLease, McpCredentialStore, McpOAuthCredential,
        McpRegistrationKind,
    },
    mcp_oauth::{
        AuthorizationChallenge, AuthorizationCodeLogin, AuthorizationCodePreparation,
        AuthorizationServerMetadata, CheckedUrl, DeviceAuthorizationResponse, RevocationOutcome,
        ScopeSet, SecretText, TokenResponse, UrlPolicy, begin_device_authorization,
        discover_authorization_server, discover_protected_resource, exchange_authorization_code,
        finish_device_authorization, finish_device_authorization_with_cancellation,
        refresh_access_token, register_dynamic_client, revoke_remote, select_initial_scopes,
        select_step_up_scopes,
    },
    rpc::Rpc,
    tool_output::ToolFailureKind,
};

const VERSION: &str = "2025-11-25";
const SUPPORTED: &[&str] = &[VERSION, "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_STATIC_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_STATIC_HEADERS_BYTES: usize = 64 * 1024;

pub(crate) struct McpHosts {
    clients: BTreeMap<String, Arc<McpClient>>,
    disabled: BTreeSet<String>,
    routes: Arc<RwLock<BTreeMap<String, (String, String)>>>,
    admission: Arc<Admission>,
    cache: OnceLock<Arc<McpCatalogStore>>,
    credentials: OnceLock<Arc<McpCredentialStore>>,
}

pub(crate) struct Admission {
    closed: AtomicBool,
    #[cfg(test)]
    pause_launch: AtomicBool,
    #[cfg(test)]
    launched: Notify,
    #[cfg(test)]
    resume_launch: Notify,
    #[cfg(test)]
    fail_setup: AtomicBool,
}

impl Admission {
    pub(crate) fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            #[cfg(test)]
            pause_launch: AtomicBool::new(false),
            #[cfg(test)]
            launched: Notify::new(),
            #[cfg(test)]
            resume_launch: Notify::new(),
            #[cfg(test)]
            fail_setup: AtomicBool::new(false),
        }
    }

    pub(crate) fn enter(&self) -> Result<()> {
        ensure!(!self.is_closed(), "MCP host is closed");
        Ok(())
    }

    fn close_admission(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) async fn after_launch(&self) {
        if self.pause_launch.swap(false, Ordering::AcqRel) {
            let resume = self.resume_launch.notified();
            tokio::pin!(resume);
            self.launched.notify_waiters();
            resume.await;
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_setup(&self) -> bool {
        self.fail_setup.swap(false, Ordering::AcqRel)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAvailability {
    Disabled,
    Live,
    Stale,
    Degraded,
}

#[derive(Debug, Serialize)]
pub struct McpStatus {
    alias: String,
    availability: McpAvailability,
    diagnostic: Option<String>,
}

impl McpStatus {
    pub fn alias(&self) -> &str {
        &self.alias
    }

    pub fn available(&self) -> bool {
        self.availability == McpAvailability::Live
    }

    pub const fn availability(&self) -> McpAvailability {
        self.availability
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

struct PreparedOAuthPublication {
    alias: String,
    resource: CheckedUrl,
    metadata: AuthorizationServerMetadata,
    client_id: String,
    request_secret: Option<SecretText>,
    stored_secret: Option<String>,
    registration: McpRegistrationKind,
    requested_scopes: Option<ScopeSet>,
    configured_ceiling: Option<ScopeSet>,
    credential_store: Arc<McpCredentialStore>,
    lease: McpCredentialLease,
    prior_generation: Option<McpCredentialGeneration>,
}

struct OAuthAuthority {
    alias: String,
    config: McpOAuthConfig,
    resource: CheckedUrl,
    metadata: AuthorizationServerMetadata,
    scopes: Option<ScopeSet>,
    configured_ceiling: Option<ScopeSet>,
}

struct OAuthRegistration {
    client_id: String,
    request_secret: Option<SecretText>,
    stored_secret: Option<String>,
    kind: McpRegistrationKind,
}

impl OAuthAuthority {
    fn publication(
        self,
        lease: McpCredentialLease,
        prior_generation: Option<McpCredentialGeneration>,
        credential_store: Arc<McpCredentialStore>,
        registration: OAuthRegistration,
    ) -> PreparedOAuthPublication {
        PreparedOAuthPublication {
            alias: self.alias,
            resource: self.resource,
            metadata: self.metadata,
            client_id: registration.client_id,
            request_secret: registration.request_secret,
            stored_secret: registration.stored_secret,
            registration: registration.kind,
            requested_scopes: self.scopes,
            configured_ceiling: self.configured_ceiling,
            credential_store,
            lease,
            prior_generation,
        }
    }
}

/// One owned browser login for a configured MCP alias. Dropping it releases
/// the callback listener and, after any in-flight native operation finishes,
/// the alias publication lock.
pub struct McpBrowserLogin {
    login: AuthorizationCodeLogin,
    publication: PreparedOAuthPublication,
}

impl McpBrowserLogin {
    pub fn authorization_url(&self) -> &str {
        self.login.authorization_url()
    }

    pub fn callback_guidance(&self) -> &'static str {
        "The loopback callback must reach this host; use same-host browsing or forward the printed loopback port."
    }

    pub async fn finish(self) -> Result<()> {
        let Self { login, publication } = self;
        let authorization = login.finish().await?;
        publication.publish_authorization(authorization).await
    }

    /// Cancel only while waiting for the browser callback. Once a valid code
    /// is accepted, the token exchange and native publication settle before
    /// this method returns.
    pub async fn finish_with_cancellation<C>(self, cancellation: C) -> Result<()>
    where
        C: Future<Output = Result<()>>,
    {
        let Self { login, publication } = self;
        tokio::pin!(cancellation);
        let authorization = tokio::select! {
            result = login.finish() => result?,
            cancelled = &mut cancellation => {
                cancelled?;
                bail!("MCP OAuth login cancelled");
            }
        };
        publication.publish_authorization(authorization).await
    }
}

impl PreparedOAuthPublication {
    async fn publish_authorization(
        self,
        authorization: crate::mcp_oauth::AuthorizationCode,
    ) -> Result<()> {
        let token = exchange_authorization_code(
            &http::client()?,
            &self.metadata,
            &self.client_id,
            self.request_secret.as_ref(),
            &self.resource,
            &authorization,
        )
        .await?;
        self.publish(token).await
    }
}

/// One server-paced device login. The user code is public protocol data; the
/// device code stays redacted inside the connector.
pub struct McpDeviceLogin {
    device: DeviceAuthorizationResponse,
    publication: PreparedOAuthPublication,
}

#[derive(Debug, Serialize)]
pub struct McpOAuthAliasStatus {
    alias: String,
    state: &'static str,
    availability: McpAvailability,
    expires_at: Option<u64>,
    scopes: Vec<String>,
    diagnostic: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct McpOAuthLogout {
    alias: String,
    local_deleted: bool,
    remote: &'static str,
}

impl McpDeviceLogin {
    pub fn verification_url(&self) -> &str {
        self.device
            .verification_uri_complete
            .as_ref()
            .unwrap_or(&self.device.verification_uri)
            .as_str()
    }

    pub fn user_code(&self) -> &str {
        self.device.user_code.expose()
    }

    pub async fn finish(self) -> Result<()> {
        let token = finish_device_authorization(
            &http::client()?,
            &self.publication.metadata,
            &self.publication.client_id,
            self.publication.request_secret.as_ref(),
            &self.publication.resource,
            &self.device,
        )
        .await?;
        self.publication.publish(token).await
    }

    /// Cancel between device polls. A dispatched token request always drains;
    /// an accepted token is published under the retained alias lease.
    pub async fn finish_with_cancellation<C>(self, cancellation: C) -> Result<()>
    where
        C: Future<Output = Result<()>>,
    {
        let token = finish_device_authorization_with_cancellation(
            &http::client()?,
            &self.publication.metadata,
            &self.publication.client_id,
            self.publication.request_secret.as_ref(),
            &self.publication.resource,
            &self.device,
            cancellation,
        )
        .await?;
        self.publication.publish(token).await
    }
}

impl PreparedOAuthPublication {
    async fn publish(self, token: TokenResponse) -> Result<()> {
        let response_scopes = token.scopes.clone();
        if let (Some(returned), Some(requested)) = (&response_scopes, &self.requested_scopes) {
            ensure!(
                returned.is_subset(requested),
                "OAuth token response granted an unrequested scope"
            );
        }
        if let (Some(returned), Some(ceiling)) = (&response_scopes, &self.configured_ceiling) {
            ensure!(
                returned.is_subset(ceiling),
                "OAuth token response exceeded the configured scope ceiling"
            );
        }
        let scopes = response_scopes
            .or(self.requested_scopes)
            .unwrap_or_default()
            .into_values();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("system clock precedes the Unix epoch")?
            .as_secs();
        let expires_at = token
            .expires_in
            .map(|seconds| {
                now.checked_add(seconds)
                    .context("OAuth token expiry overflowed")
            })
            .transpose()?
            .unwrap_or(0);
        let credential = McpOAuthCredential {
            authority: self.credential_store.binding_authority(),
            project: self.credential_store.project_identity(),
            alias: oauth_binding(b"alias", &self.alias),
            resource: oauth_binding(b"resource", self.resource.as_str()),
            issuer: oauth_binding(b"issuer", self.metadata.issuer.as_str()),
            registration: self.registration,
            expires_at,
            client_id: self.client_id,
            scopes,
            client_secret: self.stored_secret,
            access_token: token.access_token.expose().to_owned(),
            refresh_token: token.refresh_token.map(|value| value.expose().to_owned()),
        };
        let bytes = credential.encode()?;
        match self.prior_generation {
            Some(generation) => {
                self.lease.replace(generation, bytes).await?;
            }
            None => {
                self.lease.create(bytes).await?;
            }
        }
        Ok(())
    }
}

pub(crate) struct McpCatalog {
    pub(crate) tools: Vec<ToolSpec>,
    pub(crate) selectors: BTreeMap<String, PermissionSelector>,
    pub(crate) statuses: Vec<McpStatus>,
}

#[derive(Debug)]
pub(crate) enum McpExecution {
    Success(Value),
    ApplicationError(Value),
}

#[derive(Debug)]
pub(crate) struct McpCallFailure {
    pub(crate) kind: ToolFailureKind,
    pub(crate) error: anyhow::Error,
}

impl McpCallFailure {
    fn route(error: anyhow::Error) -> Self {
        Self {
            kind: ToolFailureKind::McpRoute,
            error,
        }
    }

    fn call(alias: &str) -> Self {
        Self {
            kind: ToolFailureKind::McpCall,
            error: anyhow::anyhow!("configured MCP server {alias} is unavailable"),
        }
    }
}

impl McpHosts {
    #[cfg(test)]
    pub fn new(root: &Path, configs: &BTreeMap<String, McpConfig>) -> Result<Self> {
        let root = root.canonicalize().context("MCP root does not exist")?;
        let root = Arc::new(Directory::open(
            &root,
            Privacy::Inherited,
            NameRetention::Pinned,
        )?);
        Self::with_retained_root(root, configs)
    }

    pub(crate) fn with_retained_root(
        root_guard: Arc<Directory>,
        configs: &BTreeMap<String, McpConfig>,
    ) -> Result<Self> {
        root_guard.revalidate()?;
        let root = root_guard.path().to_path_buf();
        let mut clients = BTreeMap::new();
        let mut disabled = BTreeSet::new();
        let admission = Arc::new(Admission::new());
        for (name, config) in configs {
            ensure!(
                config.command.is_some() != config.url.is_some(),
                "MCP {name} needs exactly one command or URL"
            );
            ensure!(
                config.command.is_none() || config.header_env.is_empty(),
                "stdio MCP {name} cannot specify HTTP headers"
            );
            if let Some(url) = &config.url {
                http::endpoint(url)?;
            }
            if !config.enabled {
                disabled.insert(name.clone());
                continue;
            }
            clients.insert(
                name.clone(),
                Arc::new(McpClient {
                    config: config.clone(),
                    root: root.clone(),
                    root_guard: root_guard.clone(),
                    state: Mutex::new(ClientState::default()),
                    available: AtomicBool::new(false),
                    admission: admission.clone(),
                    stop: Notify::new(),
                }),
            );
        }
        Ok(Self {
            clients,
            disabled,
            routes: Arc::new(RwLock::new(BTreeMap::new())),
            admission,
            cache: OnceLock::new(),
            credentials: OnceLock::new(),
        })
    }

    pub(crate) fn install_cache(&self, cache: Arc<McpCatalogStore>) -> Result<()> {
        self.cache
            .set(cache)
            .map_err(|_| anyhow::anyhow!("MCP catalog cache is already installed"))
    }

    pub(crate) fn install_credentials(&self, store: Arc<McpCredentialStore>) -> Result<()> {
        self.credentials
            .set(store)
            .map_err(|_| anyhow::anyhow!("MCP credential store is already installed"))
    }

    pub(crate) async fn begin_oauth_browser(&self, alias: &str) -> Result<McpBrowserLogin> {
        ensure_open(&self.admission)?;
        let (authority, lease, prior_generation, store) = self.oauth_authority(alias).await?;
        let callback = AuthorizationCodePreparation::bind(
            &authority.metadata,
            &authority.resource,
            authority.scopes.as_ref(),
            std::time::Duration::from_secs(10 * 60),
            oauth_url_policy(),
        )
        .await?;
        let redirect = CheckedUrl::parse(callback.redirect_uri(), UrlPolicy::LoopbackRedirect)?;
        let registration = resolve_oauth_registration(
            &http::client()?,
            &authority.metadata,
            &authority.config,
            &redirect,
        )
        .await?;
        let login = callback.authorize(&registration.client_id)?;
        Ok(McpBrowserLogin {
            login,
            publication: authority.publication(lease, prior_generation, store, registration),
        })
    }

    pub(crate) async fn begin_oauth_device(&self, alias: &str) -> Result<McpDeviceLogin> {
        ensure_open(&self.admission)?;
        let (authority, lease, prior_generation, store) = self.oauth_authority(alias).await?;
        let redirect = CheckedUrl::parse(
            "http://127.0.0.1/oauth/callback",
            UrlPolicy::LoopbackRedirect,
        )?;
        let registration = resolve_oauth_registration(
            &http::client()?,
            &authority.metadata,
            &authority.config,
            &redirect,
        )
        .await?;
        let device = begin_device_authorization(
            &http::client()?,
            &authority.metadata,
            &registration.client_id,
            registration.request_secret.as_ref(),
            &authority.resource,
            authority.scopes.as_ref(),
            oauth_url_policy(),
        )
        .await?;
        Ok(McpDeviceLogin {
            device,
            publication: authority.publication(lease, prior_generation, store, registration),
        })
    }

    pub(crate) async fn oauth_status(&self, alias: &str) -> Result<McpOAuthAliasStatus> {
        if self.disabled.contains(alias) {
            return Ok(McpOAuthAliasStatus {
                alias: alias.to_owned(),
                state: "disabled",
                availability: McpAvailability::Disabled,
                expires_at: None,
                scopes: Vec::new(),
                diagnostic: None,
            });
        }
        let (client, store) = self.oauth_client_and_store(alias)?;
        let catalog = self.catalog_selected(Some(alias)).await?;
        let catalog_status = catalog
            .statuses
            .into_iter()
            .find(|status| status.alias == alias)
            .with_context(|| format!("configured MCP alias {alias} is unavailable"))?;
        let record = match store.acquire(alias).await {
            Ok(lease) => match lease.get().await {
                Ok(record) => record,
                Err(error) => {
                    return Ok(McpOAuthAliasStatus {
                        alias: alias.to_owned(),
                        state: "native_store_unavailable",
                        availability: catalog_status.availability,
                        expires_at: None,
                        scopes: Vec::new(),
                        diagnostic: Some(error.to_string()),
                    });
                }
            },
            Err(error) => {
                return Ok(McpOAuthAliasStatus {
                    alias: alias.to_owned(),
                    state: "native_store_unavailable",
                    availability: catalog_status.availability,
                    expires_at: None,
                    scopes: Vec::new(),
                    diagnostic: Some(error.to_string()),
                });
            }
        };
        let Some(record) = record else {
            return Ok(McpOAuthAliasStatus {
                alias: alias.to_owned(),
                state: "login_required",
                availability: catalog_status.availability,
                expires_at: None,
                scopes: Vec::new(),
                diagnostic: catalog_status.diagnostic,
            });
        };
        let credential = match McpOAuthCredential::decode(record.secret()).and_then(|credential| {
            validate_local_credential(alias, &client.config, &store, &credential)?;
            Ok(credential)
        }) {
            Ok(credential) => credential,
            Err(error) => {
                return Ok(McpOAuthAliasStatus {
                    alias: alias.to_owned(),
                    state: "authorization_failure",
                    availability: catalog_status.availability,
                    expires_at: None,
                    scopes: Vec::new(),
                    diagnostic: Some(error.to_string()),
                });
            }
        };
        let now = unix_time()?;
        Ok(McpOAuthAliasStatus {
            alias: alias.to_owned(),
            state: if credential.expires_at != 0 && credential.expires_at <= now {
                "refresh_required"
            } else {
                "authorized"
            },
            availability: catalog_status.availability,
            expires_at: (credential.expires_at != 0).then_some(credential.expires_at),
            scopes: credential.scopes,
            diagnostic: catalog_status.diagnostic,
        })
    }

    pub(crate) async fn oauth_logout(&self, alias: &str) -> Result<McpOAuthLogout> {
        let (client, store) = self.oauth_client_and_store(alias)?;
        let alias = alias.to_owned();
        let routes = Arc::clone(&self.routes);
        // Once admitted, logout owns one detached settlement task. Dropping a
        // CLI/TUI caller cannot interrupt an in-flight remote attempt between
        // dispatch and the authoritative generation-checked local deletion.
        tokio::spawn(async move {
            // Catalog, execution and logout use the same client-state gate.
            // Retire the admitted route before touching the credential so a
            // dropped caller cannot leave its prior authorized tool callable.
            let mut state = client.state.lock().await;
            client.available.store(false, Ordering::Release);
            routes.write().await.retain(|_, (owner, _)| owner != &alias);
            let result = async {
                let lease = store.acquire(&alias).await?;
                let (lease, Some(record)) = lease.read_locked().await? else {
                    return Ok(McpOAuthLogout {
                        alias,
                        local_deleted: false,
                        remote: "no_local_credential",
                    });
                };
                let generation = record.generation();
                let remote = match McpOAuthCredential::decode(record.secret()) {
                    Ok(credential) => {
                        let attempted =
                            Self::remote_revocation(&alias, &client, &store, &credential).await;
                        match attempted {
                            Ok(outcome) => revocation_name(outcome),
                            Err(_) => "remote_outcome_uncertain",
                        }
                    }
                    Err(_) => "local_credential_invalid",
                };
                // Local deletion is unconditional after the bounded remote
                // attempt, including malformed records and uncertain network.
                lease.delete(generation).await?;
                Ok(McpOAuthLogout {
                    alias,
                    local_deleted: true,
                    remote,
                })
            }
            .await;
            // A failed native operation still leaves the old route retired.
            // Shutdown will retry transport cleanup if this attempt fails.
            let _ = close_transport(&mut state).await;
            result
        })
        .await
        .context("MCP OAuth logout settlement task stopped")?
    }

    fn oauth_client_and_store(
        &self,
        alias: &str,
    ) -> Result<(Arc<McpClient>, Arc<McpCredentialStore>)> {
        ensure_open(&self.admission)?;
        let client = self
            .clients
            .get(alias)
            .cloned()
            .with_context(|| format!("configured MCP alias {alias} is unavailable"))?;
        ensure!(
            client
                .config
                .oauth
                .as_ref()
                .is_some_and(|oauth| oauth.enabled),
            "selected MCP alias does not enable OAuth"
        );
        let store = self
            .credentials
            .get()
            .cloned()
            .context("MCP OAuth native credential store is unavailable for this command")?;
        Ok((client, store))
    }

    async fn remote_revocation(
        alias: &str,
        client: &McpClient,
        store: &McpCredentialStore,
        credential: &McpOAuthCredential,
    ) -> Result<RevocationOutcome> {
        validate_local_credential(alias, &client.config, store, credential)?;
        let resource = checked_resource_url(
            client
                .config
                .url
                .as_deref()
                .context("OAuth MCP alias lacks its HTTP resource")?,
        )?;
        let http = http::client()?;
        let protected = discover_protected_resource(&http, &resource, None, oauth_url_policy())
            .await
            .context("discover MCP OAuth protected resource for logout")?;
        let issuer = protected
            .authorization_servers
            .first()
            .context("MCP OAuth protected resource lacks an issuer")?;
        let metadata = discover_authorization_server(&http, issuer, oauth_url_policy())
            .await
            .context("discover MCP OAuth authorization server for logout")?;
        ensure!(
            credential.issuer == oauth_binding(b"issuer", metadata.issuer.as_str()),
            "stored MCP OAuth issuer no longer matches this alias"
        );
        let configured_secret = client
            .config
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.client_secret_env.as_deref())
            .map(|environment| {
                SecretText::new(
                    "client secret",
                    std::env::var(environment).with_context(|| {
                        format!("configured MCP OAuth client secret {environment} is unavailable")
                    })?,
                )
            })
            .transpose()?;
        let stored_secret = credential
            .client_secret
            .as_ref()
            .map(|value| SecretText::new("client secret", value.clone()))
            .transpose()?;
        let token = credential
            .refresh_token
            .as_ref()
            .unwrap_or(&credential.access_token);
        let token = SecretText::new("revocation token", token.clone())?;
        Ok(revoke_remote(
            &http,
            &metadata,
            &credential.client_id,
            configured_secret.as_ref().or(stored_secret.as_ref()),
            &token,
        )
        .await)
    }

    async fn oauth_authority(
        &self,
        alias: &str,
    ) -> Result<(
        OAuthAuthority,
        McpCredentialLease,
        Option<McpCredentialGeneration>,
        Arc<McpCredentialStore>,
    )> {
        let client = self
            .clients
            .get(alias)
            .with_context(|| format!("configured MCP alias {alias} is unavailable"))?;
        let config = client
            .config
            .oauth
            .as_ref()
            .filter(|oauth| oauth.enabled)
            .context("selected MCP alias does not enable OAuth")?
            .clone();
        let store = self
            .credentials
            .get()
            .cloned()
            .context("MCP OAuth native credential store is unavailable for this command")?;
        let lease = store.acquire(alias).await?;
        let (lease, current) = lease.read_locked().await?;
        let prior_generation = current.as_ref().map(|record| record.generation());
        let resource = checked_resource_url(
            client
                .config
                .url
                .as_deref()
                .context("OAuth MCP alias lacks its HTTP resource")?,
        )?;
        let http = http::client()?;
        let resource_headers = client.resolved_http_headers()?;
        let challenge = probe_authorization_challenge(&http, &resource, &resource_headers).await?;
        let protected = discover_protected_resource(
            &http,
            &resource,
            challenge
                .as_ref()
                .map(|challenge| &challenge.resource_metadata),
            oauth_url_policy(),
        )
        .await
        .context("discover MCP OAuth protected resource")?;
        let issuer = protected
            .authorization_servers
            .first()
            .context("MCP OAuth protected resource lacks an issuer")?;
        let metadata = discover_authorization_server(&http, issuer, oauth_url_policy())
            .await
            .context("discover MCP OAuth authorization server")?;
        let configured_ceiling = (!config.scopes.is_empty())
            .then(|| ScopeSet::from_values(config.scopes.iter().map(String::as_str)))
            .transpose()?;
        let scopes = if challenge
            .as_ref()
            .and_then(|challenge| challenge.error.as_deref())
            == Some("insufficient_scope")
        {
            let challenge_scopes = challenge
                .as_ref()
                .and_then(|challenge| challenge.scopes.as_ref())
                .context("insufficient-scope challenge lacks an authoritative scope")?;
            let current = current
                .as_ref()
                .context("scope step-up requires an existing MCP OAuth credential")?;
            let credential = McpOAuthCredential::decode(current.secret())?;
            validate_local_credential(alias, &client.config, &store, &credential)?;
            ensure!(
                credential.issuer == oauth_binding(b"issuer", metadata.issuer.as_str()),
                "stored MCP OAuth issuer no longer matches this alias"
            );
            let prior = ScopeSet::from_values(credential.scopes.iter().map(String::as_str))?;
            Some(select_step_up_scopes(
                &prior,
                challenge_scopes,
                configured_ceiling.as_ref(),
            )?)
        } else {
            select_initial_scopes(
                challenge
                    .as_ref()
                    .and_then(|challenge| challenge.scopes.as_ref()),
                &protected.scopes_supported,
                configured_ceiling.as_ref(),
            )?
        };
        Ok((
            OAuthAuthority {
                alias: alias.to_owned(),
                config,
                resource,
                metadata,
                scopes,
                configured_ceiling,
            },
            lease,
            prior_generation,
            store,
        ))
    }

    #[cfg(test)]
    pub async fn specs(&self) -> Result<Vec<ToolSpec>> {
        Ok(self.catalog().await?.tools)
    }

    pub(crate) async fn catalog(&self) -> Result<McpCatalog> {
        self.catalog_selected(None).await
    }

    async fn catalog_selected(&self, selected: Option<&str>) -> Result<McpCatalog> {
        ensure_open(&self.admission)?;
        let discoveries =
            self.clients
                .iter()
                .filter(|(alias, _)| selected.is_none_or(|selected| alias.as_str() == selected))
                .map(|(alias, client)| async move {
                    let mut state = client.state.lock().await;
                    ensure_open(&self.admission)?;
                    let mut disable = DisableOnDrop::new(&client.available);
                    let static_headers = match client.resolved_http_headers() {
                        Ok(headers) => headers,
                        Err(_) => {
                            self.routes
                                .write()
                                .await
                                .retain(|_, (owner, _)| owner != alias);
                            return Ok::<_, anyhow::Error>((
                                Vec::new(),
                                BTreeMap::new(),
                                McpStatus {
                                    alias: alias.clone(),
                                    availability: McpAvailability::Degraded,
                                    diagnostic: Some(
                                        "configured MCP static header is unavailable".into(),
                                    ),
                                },
                            ));
                        }
                    };
                    let (mut headers, mut credential_generation) = match client
                        .oauth_headers(alias, self.credentials.get(), static_headers.clone(), false)
                        .await
                    {
                        Ok(headers) => headers,
                        Err(error) => {
                            let _ = close_transport(&mut state).await;
                            self.routes
                                .write()
                                .await
                                .retain(|_, (owner, _)| owner != alias);
                            state.diagnostic = Some(error.to_string());
                            return Ok::<_, anyhow::Error>((
                                Vec::new(),
                                BTreeMap::new(),
                                McpStatus {
                                    alias: alias.clone(),
                                    availability: McpAvailability::Degraded,
                                    diagnostic: state.diagnostic.clone(),
                                },
                            ));
                        }
                    };
                    let mut context =
                        catalog_context(alias, &client.config, &headers, credential_generation)?;
                    let cached = self.load_cache(alias, context).await;
                    let mut cache_invalid = cached.is_err();
                    let mut cached = cached.ok().flatten();
                    let mut listed = client.list(&mut state, headers.clone()).await;
                    if listed.as_ref().is_err_and(|error| {
                        error
                            .downcast_ref::<McpHttpAuthorizationFailure>()
                            .is_some_and(McpHttpAuthorizationFailure::invalid_token)
                    }) {
                        match client
                            .oauth_headers(alias, self.credentials.get(), static_headers, true)
                            .await
                        {
                            Ok((refreshed_headers, refreshed_generation)) => {
                                headers = refreshed_headers;
                                credential_generation = refreshed_generation;
                                context = catalog_context(
                                    alias,
                                    &client.config,
                                    &headers,
                                    credential_generation,
                                )?;
                                let refreshed_cache = self.load_cache(alias, context).await;
                                cache_invalid = refreshed_cache.is_err();
                                cached = refreshed_cache.ok().flatten();
                                listed = client.list(&mut state, headers).await;
                            }
                            Err(error) => listed = Err(error),
                        }
                    }
                    let tools = match listed {
                        Ok(tools) => tools,
                        Err(error) => {
                            if !state.close_attempted {
                                let _ = close_transport(&mut state).await;
                            }
                            state.diagnostic.get_or_insert_with(|| error.to_string());
                            self.routes
                                .write()
                                .await
                                .retain(|_, (owner, _)| owner != alias);
                            let (tools, selectors, availability) = match cached {
                                Some(cached) => {
                                    let (tools, selectors) = project_cached(alias, cached)?;
                                    (tools, selectors, McpAvailability::Stale)
                                }
                                None => (Vec::new(), BTreeMap::new(), McpAvailability::Degraded),
                            };
                            return Ok::<_, anyhow::Error>((
                                tools,
                                selectors,
                                McpStatus {
                                    alias: alias.clone(),
                                    availability,
                                    diagnostic: state.diagnostic.clone().or_else(|| {
                                        cache_invalid.then(|| {
                                    "configured MCP server and its cached catalog are unavailable"
                                        .into()
                                })
                                    }),
                                },
                            ));
                        }
                    };
                    let candidate = project_live(alias, &client.config, tools);
                    match candidate {
                        Ok((candidate_routes, candidate_specs, selectors, cached_tools)) => {
                            let mut routes = self.routes.write().await;
                            if candidate_routes.keys().any(|name| {
                                routes.get(name).is_some_and(|(owner, _)| owner != alias)
                            }) {
                                routes.retain(|_, (owner, _)| owner != alias);
                                drop(routes);
                                let _ = close_transport(&mut state).await;
                                return Ok::<_, anyhow::Error>((
                                    Vec::new(),
                                    BTreeMap::new(),
                                    McpStatus {
                                        alias: alias.clone(),
                                        availability: McpAvailability::Degraded,
                                        diagnostic: state.diagnostic.clone(),
                                    },
                                ));
                            }
                            routes.retain(|_, (owner, _)| owner != alias);
                            for (name, route) in candidate_routes {
                                routes.insert(name, route);
                            }
                            client.available.store(true, Ordering::Release);
                            disable.disarm();
                            let cache_failure =
                                self.save_cache(alias, context, cached_tools).await.is_err();
                            Ok::<_, anyhow::Error>((
                                candidate_specs,
                                selectors,
                                McpStatus {
                                    alias: alias.clone(),
                                    availability: McpAvailability::Live,
                                    diagnostic: cache_failure
                                        .then(|| "MCP catalog cache could not be updated".into()),
                                },
                            ))
                        }
                        Err(_) => {
                            let _ = close_transport(&mut state).await;
                            self.routes
                                .write()
                                .await
                                .retain(|_, (owner, _)| owner != alias);
                            let (tools, selectors, availability) = match cached {
                                Some(cached) => {
                                    let (tools, selectors) = project_cached(alias, cached)?;
                                    (tools, selectors, McpAvailability::Stale)
                                }
                                None => (Vec::new(), BTreeMap::new(), McpAvailability::Degraded),
                            };
                            Ok::<_, anyhow::Error>((
                                tools,
                                selectors,
                                McpStatus {
                                    alias: alias.clone(),
                                    availability,
                                    diagnostic: state.diagnostic.clone(),
                                },
                            ))
                        }
                    }
                });
        let mut specs = Vec::new();
        let mut selectors = BTreeMap::new();
        let mut statuses = self
            .disabled
            .iter()
            .filter(|alias| selected.is_none_or(|selected| alias.as_str() == selected))
            .map(|alias| McpStatus {
                alias: alias.clone(),
                availability: McpAvailability::Disabled,
                diagnostic: None,
            })
            .collect::<Vec<_>>();
        for result in join_all(discoveries).await {
            let (candidate_specs, candidate_selectors, status) = result?;
            specs.extend(candidate_specs);
            for (name, selector) in candidate_selectors {
                ensure!(
                    selectors.insert(name, selector).is_none(),
                    "duplicate projected MCP tool name"
                );
            }
            statuses.push(status);
        }
        statuses.sort_by(|left, right| left.alias.cmp(&right.alias));
        Ok(McpCatalog {
            tools: specs,
            selectors,
            statuses,
        })
    }

    async fn load_cache(
        &self,
        alias: &str,
        context: [u8; 32],
    ) -> Result<Option<Vec<CachedMcpTool>>> {
        let Some(cache) = self.cache.get().cloned() else {
            return Ok(None);
        };
        let alias = alias.to_owned();
        tokio::task::spawn_blocking(move || cache.load(&alias, context))
            .await
            .context("MCP catalog cache reader stopped")?
    }

    async fn save_cache(
        &self,
        alias: &str,
        context: [u8; 32],
        tools: Vec<CachedMcpTool>,
    ) -> Result<()> {
        let Some(cache) = self.cache.get().cloned() else {
            return Ok(());
        };
        let alias = alias.to_owned();
        tokio::task::spawn_blocking(move || cache.save(&alias, context, tools))
            .await
            .context("MCP catalog cache writer stopped")?
    }

    pub(crate) async fn execute(
        &self,
        name: &str,
        arguments: Value,
    ) -> std::result::Result<McpExecution, McpCallFailure> {
        let route = self.routes.read().await.get(name).cloned();
        let (alias, original) = route.ok_or_else(|| {
            McpCallFailure::route(anyhow::anyhow!(
                "unknown tool: {name}; discover configured MCP tools first"
            ))
        })?;
        let client = self
            .clients
            .get(&alias)
            .ok_or_else(|| McpCallFailure::route(anyhow::anyhow!("MCP route no longer exists")))?;
        ensure_open(&self.admission).map_err(McpCallFailure::route)?;
        let mut state = client.state.lock().await;
        ensure_open(&self.admission).map_err(McpCallFailure::route)?;
        if !client.available.load(Ordering::Acquire) {
            return Err(McpCallFailure::call(&alias));
        }
        let mut disable = DisableOnDrop::new(&client.available);
        let result = client.call(&mut state, &original, arguments).await;
        let result = match result {
            Ok(result) => {
                disable.disarm();
                result
            }
            Err(_) => return Err(McpCallFailure::call(&alias)),
        };
        if result["isError"] == true {
            Ok(McpExecution::ApplicationError(result["content"].clone()))
        } else {
            Ok(McpExecution::Success(result))
        }
    }

    /// Resolve the provider-facing hash to its stable configured identity
    /// before permission matching. The hash itself is never an authority key.
    pub(crate) async fn selector(
        &self,
        name: &str,
    ) -> std::result::Result<PermissionSelector, McpCallFailure> {
        let route = self.routes.read().await.get(name).cloned();
        let (alias, original) = route.ok_or_else(|| {
            McpCallFailure::route(anyhow::anyhow!(
                "unknown tool: {name}; discover configured MCP tools first"
            ))
        })?;
        PermissionSelector::mcp(alias, original).map_err(McpCallFailure::route)
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.admission.close_admission();
        let mut clients = tokio::task::JoinSet::new();
        for client in self.clients.values() {
            client.stop.notify_waiters();
            let client = Arc::clone(client);
            clients.spawn(async move { client.close().await });
        }
        let mut first_error = None;
        let joined = tokio::time::timeout(IO_TIMEOUT, async {
            while let Some(result) = clients.join_next().await {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        first_error.get_or_insert(error);
                    }
                    Err(_) => {
                        first_error
                            .get_or_insert_with(|| anyhow::anyhow!("MCP cleanup task failed"));
                    }
                }
            }
        })
        .await;
        if joined.is_err() {
            clients.abort_all();
            return Err(anyhow::anyhow!("MCP shutdown remains unconfirmed"));
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}

struct McpClient {
    config: McpConfig,
    root: PathBuf,
    root_guard: Arc<Directory>,
    state: Mutex<ClientState>,
    available: AtomicBool,
    admission: Arc<Admission>,
    stop: Notify,
}

type ProjectedCatalog = (
    BTreeMap<String, (String, String)>,
    Vec<ToolSpec>,
    BTreeMap<String, PermissionSelector>,
    Vec<CachedMcpTool>,
);

fn project_live(alias: &str, config: &McpConfig, tools: Vec<Value>) -> Result<ProjectedCatalog> {
    let mut routes = BTreeMap::new();
    let mut specs = Vec::new();
    let mut selectors = BTreeMap::new();
    let mut cached = Vec::new();
    for tool in tools {
        let original = tool["name"].as_str().context("MCP tool lacks name")?;
        if !config.admits_tool(original) {
            continue;
        }
        let selector = PermissionSelector::mcp(alias, original)?;
        ensure!(
            tool["inputSchema"].is_object(),
            "MCP tool lacks inputSchema object"
        );
        let name = projected_name(alias, original);
        ensure!(
            routes
                .insert(name.clone(), (alias.to_owned(), original.to_owned()))
                .is_none(),
            "duplicate MCP tool name for {alias}"
        );
        ensure!(
            selectors.insert(name.clone(), selector).is_none(),
            "duplicate MCP tool selector for {alias}"
        );
        let description = tool["description"]
            .as_str()
            .unwrap_or("Configured external tool")
            .to_owned();
        let parameters = tool["inputSchema"].clone();
        specs.push(ToolSpec {
            name,
            description: format!("MCP {alias}/{original}: {description}"),
            parameters: parameters.clone(),
        });
        cached.push(CachedMcpTool {
            original_name: original.to_owned(),
            description,
            parameters,
        });
    }
    Ok((routes, specs, selectors, cached))
}

fn project_cached(
    alias: &str,
    tools: Vec<CachedMcpTool>,
) -> Result<(Vec<ToolSpec>, BTreeMap<String, PermissionSelector>)> {
    let mut specs = Vec::with_capacity(tools.len());
    let mut selectors = BTreeMap::new();
    for tool in tools {
        let selector = PermissionSelector::mcp(alias, &tool.original_name)?;
        let name = projected_name(alias, &tool.original_name);
        ensure!(
            selectors.insert(name.clone(), selector).is_none(),
            "duplicate cached MCP tool name for {alias}"
        );
        specs.push(ToolSpec {
            name,
            description: format!(
                "MCP {alias}/{} (stale; unavailable until rediscovered): {}",
                tool.original_name, tool.description
            ),
            parameters: tool.parameters,
        });
    }
    Ok((specs, selectors))
}

fn projected_name(alias: &str, original: &str) -> String {
    format!(
        "mcp_{}",
        uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("{alias}\0{original}").as_bytes()
        )
        .simple()
    )
}

fn catalog_context(
    alias: &str,
    config: &McpConfig,
    headers: &HeaderMap,
    credential_generation: Option<McpCredentialGeneration>,
) -> Result<[u8; 32]> {
    let mut digest = Sha256::new();
    digest.update(b"kuru.mcp.catalog-context.v1\0");
    hash_field(&mut digest, alias.as_bytes());
    hash_field(&mut digest, &serde_json::to_vec(config)?);
    for (name, environment) in &config.header_env {
        hash_field(&mut digest, name.as_bytes());
        hash_field(&mut digest, environment.as_bytes());
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| anyhow::anyhow!("configured MCP static header name is invalid"))?;
        let value = headers
            .get(name)
            .context("configured MCP static header value is unavailable")?;
        hash_field(&mut digest, value.as_bytes());
    }
    if let Some(generation) = credential_generation {
        hash_field(&mut digest, &generation.bytes());
    }
    Ok(digest.finalize().into())
}

fn hash_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn is_reserved_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "accept"
            | "connection"
            | "content-length"
            | "content-type"
            | "host"
            | "mcp-protocol-version"
            | "mcp-session-id"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

#[derive(Default)]
struct ClientState {
    transport: Option<Transport>,
    diagnostic: Option<String>,
    close_attempted: bool,
}

struct DisableOnDrop<'a> {
    available: &'a AtomicBool,
    armed: bool,
}

impl<'a> DisableOnDrop<'a> {
    fn new(available: &'a AtomicBool) -> Self {
        Self {
            available,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DisableOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.available.store(false, Ordering::Release);
        }
    }
}

impl McpClient {
    fn resolved_http_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let mut total_bytes = 0_usize;
        for (name, environment) in &self.config.header_env {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| anyhow::anyhow!("configured MCP static header name is invalid"))?;
            ensure!(
                !is_reserved_header(&name),
                "configured MCP static header is reserved for protocol ownership"
            );
            let value = static_header_value(
                environment,
                std::env::var_os(environment).with_context(|| {
                    format!("configured MCP static header environment {environment} is missing")
                })?,
            )?;
            total_bytes = total_bytes
                .checked_add(name.as_str().len())
                .and_then(|bytes| bytes.checked_add(value.as_bytes().len()))
                .context("configured MCP static headers exceed their aggregate byte limit")?;
            ensure!(
                total_bytes <= MAX_STATIC_HEADERS_BYTES,
                "configured MCP static headers exceed their aggregate byte limit"
            );
            ensure!(
                headers.insert(name, value).is_none(),
                "duplicate configured MCP static header"
            );
        }
        Ok(headers)
    }

    async fn oauth_headers(
        &self,
        alias: &str,
        credential_store: Option<&Arc<McpCredentialStore>>,
        mut headers: HeaderMap,
        force_refresh: bool,
    ) -> Result<(HeaderMap, Option<McpCredentialGeneration>)> {
        let Some(oauth) = self.config.oauth.as_ref().filter(|oauth| oauth.enabled) else {
            return Ok((headers, None));
        };
        let resource = checked_resource_url(
            self.config
                .url
                .as_deref()
                .context("OAuth MCP alias lacks its HTTP resource")?,
        )?;
        let client = http::client()?;
        let protected = discover_protected_resource(&client, &resource, None, oauth_url_policy())
            .await
            .context("discover MCP OAuth protected resource")?;
        let issuer = protected
            .authorization_servers
            .first()
            .context("MCP OAuth protected resource lacks an issuer")?;
        let metadata = discover_authorization_server(&client, issuer, oauth_url_policy())
            .await
            .context("discover MCP OAuth authorization server")?;
        let credential_store = credential_store
            .context("MCP OAuth native credential store is unavailable for this command")?;
        let lease = credential_store.acquire(alias).await?;
        let (lease, record) = lease.read_locked().await?;
        let record = record.context("MCP OAuth login is required for this alias")?;
        let mut credential = McpOAuthCredential::decode(record.secret())?;
        validate_local_credential(alias, &self.config, credential_store, &credential)?;
        ensure!(
            credential.issuer == oauth_binding(b"issuer", metadata.issuer.as_str()),
            "stored MCP OAuth issuer no longer matches this alias"
        );
        let now = unix_time()?;
        let generation = if force_refresh
            || (credential.expires_at != 0 && credential.expires_at <= now)
        {
            let refresh = credential
                .refresh_token
                .as_deref()
                .context("MCP OAuth credential requires relogin")?;
            let refresh = SecretText::new("refresh token", refresh.to_owned())?;
            let prior_scopes = ScopeSet::from_values(credential.scopes.iter().map(String::as_str))?;
            let configured_ceiling = (!oauth.scopes.is_empty())
                .then(|| ScopeSet::from_values(oauth.scopes.iter().map(String::as_str)))
                .transpose()?;
            ensure!(
                configured_ceiling
                    .as_ref()
                    .is_none_or(|ceiling| prior_scopes.is_subset(ceiling)),
                "stored MCP OAuth scope is outside the configured ceiling"
            );
            let configured_secret = configured_client_secret(oauth)?;
            let stored_secret = credential
                .client_secret
                .as_ref()
                .map(|value| SecretText::new("client secret", value.clone()))
                .transpose()?;
            let token = refresh_access_token(
                &client,
                &metadata,
                &credential.client_id,
                configured_secret.as_ref().or(stored_secret.as_ref()),
                &resource,
                &refresh,
                (!prior_scopes.is_empty()).then_some(&prior_scopes),
            )
            .await?;
            if let Some(scopes) = &token.scopes {
                ensure!(
                    scopes.is_subset(&prior_scopes),
                    "OAuth refresh response expanded the granted scope"
                );
            }
            credential.access_token = token.access_token.expose().to_owned();
            if let Some(refresh) = token.refresh_token {
                credential.refresh_token = Some(refresh.expose().to_owned());
            }
            if let Some(scopes) = token.scopes {
                credential.scopes = scopes.into_values();
            }
            credential.expires_at = token
                .expires_in
                .map(|seconds| {
                    now.checked_add(seconds)
                        .context("OAuth token expiry overflowed")
                })
                .transpose()?
                .unwrap_or(0);
            let bytes = credential.encode()?;
            lease.replace(record.generation(), bytes).await?
        } else {
            record.generation()
        };
        let mut value = HeaderValue::from_str(&format!("Bearer {}", credential.access_token))
            .context("stored MCP OAuth access token is not a valid header value")?;
        value.set_sensitive(true);
        ensure!(
            headers.insert(AUTHORIZATION, value).is_none(),
            "OAuth MCP alias has another authorization header"
        );
        Ok((headers, Some(generation)))
    }

    async fn while_open<T>(
        &self,
        operation: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let stopped = self.stop.notified();
        tokio::pin!(stopped);
        ensure_open(&self.admission)?;
        tokio::pin!(operation);
        tokio::select! {
            biased;
            _ = &mut stopped => Err(anyhow::anyhow!("MCP host is closed")),
            result = &mut operation => result,
        }
    }

    async fn initialize(&self, state: &mut ClientState, headers: HeaderMap) -> Result<()> {
        state.transport = Some(if let Some(url) = &self.config.url {
            Transport::Http(HttpRpc {
                client: http::client()?,
                url: url.clone(),
                headers,
                session: None,
                version: None,
                next_id: 0,
            })
        } else {
            Transport::Stdio(Box::new(Rpc::spawn(
                self.config
                    .command
                    .as_deref()
                    .context("missing MCP command")?,
                &self.config.args,
                &self.config.env,
                &self.root,
                self.root_guard.clone(),
                self.admission.clone(),
            )?))
        });
        state.close_attempted = false;
        let initialization = async {
            let transport = state
                .transport
                .as_mut()
                .context("MCP initialization failed")?;
            self.while_open(transport.ready()).await?;
            let result = self.while_open(transport.request("initialize", json!({"protocolVersion":VERSION,"capabilities":{},"clientInfo":{"name":"kuru","version":env!("CARGO_PKG_VERSION")}}))).await?;
            let version = result["protocolVersion"].as_str().context("MCP initialize lacks protocolVersion")?;
            ensure!(SUPPORTED.contains(&version), "unsupported MCP protocol version: {version}");
            ensure!(result["capabilities"]["tools"].is_object(), "MCP server did not advertise tools capability");
            if let Transport::Http(http) = &mut *transport { http.version = Some(version.into()); }
            self.while_open(transport.notify("notifications/initialized", json!({}))).await
        }.await;
        if let Err(error) = initialization {
            let _ = close_transport(state).await;
            return Err(error);
        }
        Ok(())
    }

    async fn request(&self, state: &mut ClientState, method: &str, params: Value) -> Result<Value> {
        ensure!(
            state.transport.is_some(),
            "MCP route is not initialized; rediscover its catalog"
        );
        let result = self
            .while_open(
                state
                    .transport
                    .as_mut()
                    .context("MCP initialization failed")?
                    .request(method, params),
            )
            .await;
        if result.is_err() {
            // Never automatically retry a tool mutation. A subsequent explicit
            // call may establish a fresh session after a failed transport.
            let _ = close_transport(state).await;
        }
        result
    }

    async fn list(&self, state: &mut ClientState, headers: HeaderMap) -> Result<Vec<Value>> {
        if !self.available.load(Ordering::Acquire) {
            close_transport(state).await?;
        }
        state.diagnostic = None;
        if state
            .transport
            .as_ref()
            .is_some_and(|transport| !transport.matches_http_headers(&headers))
        {
            close_transport(state).await?;
        }
        if state.transport.is_none() {
            self.initialize(state, headers).await?;
        }
        let mut cursor = Value::Null;
        let mut seen = BTreeSet::new();
        let mut tools = Vec::new();
        loop {
            let params = if cursor.is_null() {
                json!({})
            } else {
                json!({"cursor":cursor})
            };
            let result = self.request(state, "tools/list", params).await?;
            tools.extend(
                result["tools"]
                    .as_array()
                    .context("MCP tools/list lacks tools array")?
                    .clone(),
            );
            ensure!(
                tools.len() <= 10_000,
                "MCP tool catalog exceeds 10000 tools"
            );
            cursor = result["nextCursor"].clone();
            if cursor.is_null() || cursor == "" {
                break;
            }
            ensure!(
                seen.len() < 100 && seen.insert(cursor.to_string()),
                "MCP pagination did not advance"
            );
        }
        Ok(tools)
    }

    async fn call(&self, state: &mut ClientState, name: &str, arguments: Value) -> Result<Value> {
        self.request(
            state,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
        )
        .await
    }

    async fn close(&self) -> Result<()> {
        self.available.store(false, Ordering::Release);
        let mut state = self.state.lock().await;
        close_transport(&mut state).await
    }
}

fn oauth_binding(kind: &[u8], value: &str) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"kuru.mcp.oauth.binding.v1\0");
    hash_field(&mut digest, kind);
    hash_field(&mut digest, value.as_bytes());
    digest.finalize().into()
}

fn validate_local_credential(
    alias: &str,
    config: &McpConfig,
    store: &McpCredentialStore,
    credential: &McpOAuthCredential,
) -> Result<()> {
    let resource = checked_resource_url(
        config
            .url
            .as_deref()
            .context("OAuth MCP alias lacks its HTTP resource")?,
    )?;
    ensure!(
        credential.authority == store.binding_authority()
            && credential.project == store.project_identity()
            && credential.alias == oauth_binding(b"alias", alias)
            && credential.resource == oauth_binding(b"resource", resource.as_str()),
        "stored MCP OAuth credential does not match this alias authority"
    );
    let oauth = config
        .oauth
        .as_ref()
        .filter(|oauth| oauth.enabled)
        .context("selected MCP alias does not enable OAuth")?;
    match (
        oauth.client_id.as_deref(),
        oauth.client_metadata_url.as_deref(),
        credential.registration,
    ) {
        (Some(client_id), None, McpRegistrationKind::Configured) => ensure!(
            credential.client_id == client_id,
            "stored MCP OAuth client identity no longer matches this alias"
        ),
        (None, Some(client_id), McpRegistrationKind::ClientMetadata) => {
            let client_id = CheckedUrl::parse(client_id, oauth_url_policy())?;
            ensure!(
                credential.client_id == client_id.as_str(),
                "stored MCP OAuth client identity no longer matches this alias"
            );
        }
        (None, None, McpRegistrationKind::Dynamic) => {}
        _ => bail!("stored MCP OAuth registration no longer matches this alias"),
    }
    Ok(())
}

fn configured_client_secret(config: &McpOAuthConfig) -> Result<Option<SecretText>> {
    config
        .client_secret_env
        .as_deref()
        .map(|environment| {
            SecretText::new(
                "client secret",
                std::env::var(environment).with_context(|| {
                    format!("configured MCP OAuth client secret {environment} is unavailable")
                })?,
            )
        })
        .transpose()
}

fn unix_time() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock precedes the Unix epoch")?
        .as_secs())
}

fn revocation_name(outcome: RevocationOutcome) -> &'static str {
    match outcome {
        RevocationOutcome::Revoked => "revoked",
        RevocationOutcome::NoAdvertisedEndpoint => "no_advertised_endpoint",
        RevocationOutcome::RemoteRefused => "remote_refused",
        RevocationOutcome::RemoteOutcomeUncertain => "remote_outcome_uncertain",
    }
}

async fn resolve_oauth_registration(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    config: &McpOAuthConfig,
    redirect: &CheckedUrl,
) -> Result<OAuthRegistration> {
    if let Some(client_id) = &config.client_id {
        let request_secret = config
            .client_secret_env
            .as_deref()
            .map(|environment| {
                let value = std::env::var(environment).with_context(|| {
                    format!("configured MCP OAuth client secret {environment} is unavailable")
                })?;
                SecretText::new("client secret", value)
            })
            .transpose()?;
        return Ok(OAuthRegistration {
            client_id: client_id.clone(),
            request_secret,
            // Configured secrets remain environment-owned and are never copied
            // into Kuru's native credential record.
            stored_secret: None,
            kind: McpRegistrationKind::Configured,
        });
    }
    if let Some(client_id) = config.client_metadata_url.as_ref() {
        ensure!(
            metadata.client_id_metadata_document_supported,
            "authorization server does not advertise Client ID Metadata Documents"
        );
        return Ok(OAuthRegistration {
            client_id: client_id.clone(),
            request_secret: None,
            stored_secret: None,
            kind: McpRegistrationKind::ClientMetadata,
        });
    }
    let endpoint = metadata
        .registration_endpoint
        .as_ref()
        .context("authorization server advertises no usable client registration mechanism")?;
    let registered = register_dynamic_client(client, endpoint, redirect).await?;
    let stored_secret = registered
        .client_secret
        .as_ref()
        .map(|secret| secret.expose().to_owned());
    Ok(OAuthRegistration {
        client_id: registered.client_id,
        request_secret: registered.client_secret,
        stored_secret,
        kind: McpRegistrationKind::Dynamic,
    })
}

fn checked_resource_url(value: &str) -> Result<CheckedUrl> {
    CheckedUrl::parse(value, oauth_url_policy())
}

async fn probe_authorization_challenge(
    client: &reqwest::Client,
    resource: &CheckedUrl,
    headers: &HeaderMap,
) -> Result<Option<AuthorizationChallenge>> {
    let mut request = client.get(resource.as_str());
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .await
        .context("probe MCP OAuth protected resource")?;
    if !matches!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    ) {
        return Ok(None);
    }
    let challenges = response
        .headers()
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .collect::<Vec<_>>();
    ensure!(
        challenges.len() == 1,
        "MCP protected resource must return one authorization challenge"
    );
    let challenge = challenges[0]
        .to_str()
        .context("MCP authorization challenge is not ASCII")?;
    Ok(Some(AuthorizationChallenge::parse(
        challenge,
        oauth_url_policy(),
    )?))
}

fn oauth_url_policy() -> UrlPolicy {
    #[cfg(test)]
    {
        UrlPolicy::LoopbackFixture
    }
    #[cfg(not(test))]
    {
        UrlPolicy::Https
    }
}

fn static_header_value(environment: &str, value: std::ffi::OsString) -> Result<HeaderValue> {
    let value = value.into_string().map_err(|_| {
        anyhow::anyhow!("configured MCP static header environment {environment} is not Unicode")
    })?;
    ensure!(
        value.len() <= MAX_STATIC_HEADER_VALUE_BYTES,
        "configured MCP static header environment {environment} exceeds its byte limit"
    );
    HeaderValue::from_str(&value).map_err(|_| {
        anyhow::anyhow!(
            "configured MCP static header environment {environment} is not a valid header value"
        )
    })
}

async fn close_transport(state: &mut ClientState) -> Result<()> {
    let Some(transport) = state.transport.as_mut() else {
        state.close_attempted = false;
        return Ok(());
    };
    state.close_attempted = true;
    let result = transport.close().await;
    state.diagnostic = transport.stderr_diagnostic();
    result?;
    state.transport.take();
    state.close_attempted = false;
    Ok(())
}

fn ensure_open(admission: &Admission) -> Result<()> {
    ensure!(!admission.is_closed(), "MCP host is closed");
    Ok(())
}

enum Transport {
    Stdio(Box<Rpc>),
    Http(HttpRpc),
}

impl Transport {
    fn matches_http_headers(&self, headers: &HeaderMap) -> bool {
        match self {
            Self::Stdio(_) => headers.is_empty(),
            Self::Http(rpc) => rpc.headers == *headers,
        }
    }

    async fn ready(&mut self) -> Result<()> {
        match self {
            Self::Stdio(rpc) => rpc.ready().await,
            Self::Http(_) => Ok(()),
        }
    }

    fn stderr_diagnostic(&self) -> Option<String> {
        match self {
            Self::Stdio(rpc) => rpc.stderr_diagnostic(),
            Self::Http(_) => None,
        }
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        match self {
            Self::Stdio(rpc) => rpc.request(method, params, IO_TIMEOUT).await,
            Self::Http(rpc) => tokio::time::timeout(IO_TIMEOUT, rpc.request(method, params))
                .await
                .context("MCP HTTP request timed out")?,
        }
    }
    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        match self {
            Self::Stdio(rpc) => {
                rpc.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
                    .await
            }
            Self::Http(rpc) => tokio::time::timeout(IO_TIMEOUT, async {
                http::body(
                    rpc.post()
                        .json(&json!({"jsonrpc":"2.0","method":method,"params":params}))
                        .send()
                        .await?,
                )
                .await?;
                Ok(())
            })
            .await
            .context("MCP HTTP notification timed out")?,
        }
    }
    async fn close(&mut self) -> Result<()> {
        match self {
            Self::Stdio(rpc) => rpc.close().await,
            Self::Http(rpc) => {
                if let Some(session) = rpc.session.clone() {
                    let mut request = rpc
                        .client
                        .delete(&rpc.url)
                        .header("Mcp-Session-Id", session);
                    for (name, value) in &rpc.headers {
                        request = request.header(name, value);
                    }
                    if let Some(version) = &rpc.version {
                        request = request.header("MCP-Protocol-Version", version);
                    }
                    let result = request.send().await?;
                    ensure!(
                        result.status().is_success()
                            || matches!(result.status().as_u16(), 404 | 405),
                        "MCP session DELETE failed: {}",
                        result.status()
                    );
                    rpc.session = None;
                }
                Ok(())
            }
        }
    }
}

struct HttpRpc {
    client: reqwest::Client,
    url: String,
    headers: HeaderMap,
    session: Option<String>,
    version: Option<String>,
    next_id: u64,
}

#[derive(Debug)]
struct McpHttpAuthorizationFailure {
    status: reqwest::StatusCode,
    challenge: Option<AuthorizationChallenge>,
}

impl McpHttpAuthorizationFailure {
    fn invalid_token(&self) -> bool {
        self.status == reqwest::StatusCode::UNAUTHORIZED
            && self
                .challenge
                .as_ref()
                .and_then(|challenge| challenge.error.as_deref())
                == Some("invalid_token")
    }

    fn insufficient_scope(&self) -> bool {
        self.status == reqwest::StatusCode::FORBIDDEN
            && self
                .challenge
                .as_ref()
                .and_then(|challenge| challenge.error.as_deref())
                == Some("insufficient_scope")
    }
}

impl std::fmt::Display for McpHttpAuthorizationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.insufficient_scope() {
            formatter.write_str("MCP OAuth scope step-up is required")
        } else if self.invalid_token() {
            formatter.write_str("MCP OAuth access token was rejected")
        } else {
            write!(formatter, "MCP HTTP authorization failed: {}", self.status)
        }
    }
}

impl std::error::Error for McpHttpAuthorizationFailure {}

fn http_authorization_failure(response: &reqwest::Response) -> Result<anyhow::Error> {
    let mut bearer = Vec::new();
    for value in response.headers().get_all(WWW_AUTHENTICATE) {
        let value = value
            .to_str()
            .context("MCP authorization challenge is not ASCII")?;
        if value
            .split_whitespace()
            .next()
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("Bearer"))
        {
            bearer.push(value);
        }
    }
    ensure!(
        bearer.len() <= 1,
        "MCP protected resource returned multiple Bearer challenges"
    );
    let challenge = bearer
        .first()
        .map(|value| AuthorizationChallenge::parse(value, oauth_url_policy()))
        .transpose()?;
    Ok(anyhow::Error::new(McpHttpAuthorizationFailure {
        status: response.status(),
        challenge,
    }))
}

impl HttpRpc {
    fn post(&self) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream");
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(session) = &self.session {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(version) = &self.version {
            request = request.header("MCP-Protocol-Version", version);
        }
        request
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = json!(self.next_id);
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        ensure!(
            message.to_string().len() <= MAX_BYTES,
            "MCP request exceeds size limit"
        );
        let mut response = self.post().json(&message).send().await?;
        if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            return Err(http_authorization_failure(&response)?);
        }
        ensure!(
            response.status().is_success(),
            "MCP HTTP request failed: {}",
            response.status()
        );
        if let Some(session) = response.headers().get("Mcp-Session-Id") {
            self.session = Some(
                session
                    .to_str()
                    .context("invalid MCP session header")?
                    .into(),
            );
        }
        if response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
        {
            let mut parser = Sse::default();
            while let Some(chunk) = response.chunk().await? {
                for event in parser.push(&chunk)? {
                    if event.get("id") == Some(&id) && event.get("method").is_none() {
                        return http::rpc_result(event, &id);
                    }
                    if event.get("method").is_some() && event.get("id").is_some() {
                        let response = if event["method"] == "ping" {
                            json!({"jsonrpc":"2.0","id":event["id"],"result":{}})
                        } else {
                            json!({"jsonrpc":"2.0","id":event["id"],"error":{"code":-32601,"message":"Client capability not supported"}})
                        };
                        http::body(self.post().json(&response).send().await?).await?;
                    }
                }
            }
            bail!("MCP SSE stream ended before response");
        }
        http::rpc_result(http::json(response).await?, &id)
    }
}

#[derive(Default)]
struct Sse {
    pending: Vec<u8>,
    data: Vec<String>,
    total: usize,
}

impl Sse {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        self.total += bytes.len();
        ensure!(
            self.total <= MAX_BYTES,
            "MCP SSE response exceeds size limit"
        );
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            let line = std::str::from_utf8(&line)
                .context("invalid SSE UTF-8")?
                .trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(
                        serde_json::from_str(&self.data.join("\n"))
                            .context("invalid MCP SSE JSON")?,
                    );
                    self.data.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::test_support::drain_bounded;
    use crate::test_support::{HttpFixture, Reply, StdioFixture, Step};
    use axum::{
        Router,
        body::{Body, Bytes},
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use kuru_core::{ConfigSnapshot, InvocationOverrides};
    #[cfg(unix)]
    use tokio::time::{Duration, timeout};

    fn http_config(url: &str) -> BTreeMap<String, McpConfig> {
        [(
            "test".into(),
            McpConfig {
                command: None,
                args: vec![],
                url: Some(url.into()),
                env: BTreeMap::new(),
                ..Default::default()
            },
        )]
        .into()
    }
    fn initialized() -> Reply {
        let mut reply = Reply::rpc(json!({"protocolVersion":VERSION,"capabilities":{"tools":{}}}));
        reply.session = true;
        reply
    }

    #[tokio::test]
    async fn protected_resource_probe_preserves_nonauth_headers_and_authoritative_scope() {
        let observed = Arc::new(tokio::sync::Mutex::new(None));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let challenge = format!(
            "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource\", scope=\"mcp.read mcp.write\""
        );
        let task = tokio::spawn({
            let observed = observed.clone();
            async move {
                let app = Router::new().route(
                    "/mcp",
                    get(move |headers: HeaderMap| {
                        let observed = observed.clone();
                        let challenge = challenge.clone();
                        async move {
                            *observed.lock().await = headers.get("x-fixture").cloned();
                            let mut response = StatusCode::UNAUTHORIZED.into_response();
                            response
                                .headers_mut()
                                .insert(WWW_AUTHENTICATE, challenge.parse().unwrap());
                            response
                        }
                    }),
                );
                axum::serve(socket, app).await.unwrap();
            }
        });
        let resource =
            CheckedUrl::parse(&format!("{base}/mcp"), UrlPolicy::LoopbackFixture).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-fixture", "retained".parse().unwrap());
        let challenge =
            probe_authorization_challenge(&http::client().unwrap(), &resource, &headers)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            challenge.scopes.unwrap().into_values(),
            ["mcp.read".to_owned(), "mcp.write".to_owned()]
        );
        assert_eq!(
            observed.lock().await.as_ref().unwrap(),
            &HeaderValue::from_static("retained")
        );
        task.abort();
    }

    #[tokio::test]
    async fn http_authorization_failures_keep_typed_invalid_and_step_up_boundaries() {
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let metadata = format!("{base}/.well-known/oauth-protected-resource");
        let task = tokio::spawn({
            let requests = requests.clone();
            async move {
                let app = Router::new().fallback(move |headers: HeaderMap| {
                    let requests = requests.clone();
                    let metadata = metadata.clone();
                    async move {
                        requests.fetch_add(1, Ordering::Relaxed);
                        let (status, error, scope) = if headers["x-case"] == "step-up" {
                            (StatusCode::FORBIDDEN, "insufficient_scope", "mcp.write")
                        } else {
                            (StatusCode::UNAUTHORIZED, "invalid_token", "mcp.read")
                        };
                        let challenge = format!(
                            "Bearer resource_metadata=\"{metadata}\", error=\"{error}\", scope=\"{scope}\""
                        );
                        let mut response = status.into_response();
                        response
                            .headers_mut()
                            .insert(WWW_AUTHENTICATE, challenge.parse().unwrap());
                        response
                    }
                });
                axum::serve(socket, app).await.unwrap();
            }
        });
        for (case, invalid, step_up) in [("invalid", true, false), ("step-up", false, true)] {
            let mut headers = HeaderMap::new();
            headers.insert("x-case", case.parse().unwrap());
            let mut rpc = HttpRpc {
                client: http::client().unwrap(),
                url: format!("{base}/mcp"),
                headers,
                session: None,
                version: None,
                next_id: 0,
            };
            let error = rpc.request("tools/list", json!({})).await.unwrap_err();
            let failure = error.downcast_ref::<McpHttpAuthorizationFailure>().unwrap();
            assert_eq!(failure.invalid_token(), invalid);
            assert_eq!(failure.insufficient_scope(), step_up);
        }
        assert_eq!(requests.load(Ordering::Relaxed), 2);
        task.abort();
    }

    #[tokio::test]
    async fn oauth_commands_refuse_unknown_disabled_and_stdio_aliases_before_activation() {
        let disabled = HttpFixture::new(Vec::new()).await;
        let stdio = StdioFixture::new([Step::Eof]);
        let project = tempfile::tempdir().unwrap();
        let config = BTreeMap::from([
            (
                "disabled".into(),
                McpConfig {
                    enabled: false,
                    url: Some(disabled.url.clone()),
                    oauth: Some(McpOAuthConfig {
                        enabled: true,
                        client_id: Some("native-client".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            (
                "stdio".into(),
                McpConfig {
                    command: Some(stdio.command().into()),
                    ..Default::default()
                },
            ),
        ]);
        let hosts = McpHosts::new(project.path(), &config).unwrap();
        for alias in ["unknown", "disabled"] {
            let error = match hosts.begin_oauth_browser(alias).await {
                Ok(_) => panic!("{alias} unexpectedly started OAuth"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("unavailable"), "{error:#}");
        }
        let error = match hosts.begin_oauth_browser("stdio").await {
            Ok(_) => panic!("stdio alias unexpectedly started OAuth"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("does not enable OAuth"),
            "{error:#}"
        );
        assert!(disabled.requests.lock().await.is_empty());
        assert!(stdio.conversations().is_empty());
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn browser_login_settles_native_publication_then_refreshes_and_logs_out() {
        let token_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let revocations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let revoked_body = Arc::new(tokio::sync::Mutex::new(None));
        let accepted_token = Arc::new(Notify::new());
        let require_step_up = Arc::new(AtomicBool::new(false));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let issuer = format!("{base}/");
        let metadata_url = format!("{base}/.well-known/oauth-protected-resource/mcp");
        let task = tokio::spawn({
            let base = base.clone();
            let issuer = issuer.clone();
            let token_requests = token_requests.clone();
            let revocations = revocations.clone();
            let revoked_body = revoked_body.clone();
            let accepted_token = accepted_token.clone();
            let require_step_up = require_step_up.clone();
            async move {
                let resource_metadata = json!({
                    "resource": format!("{base}/mcp"),
                    "authorization_servers": [issuer.clone()],
                    "scopes_supported": ["mcp.read", "mcp.write"]
                });
                let server_metadata = json!({
                    "issuer": issuer,
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                    "revocation_endpoint": format!("{base}/revoke"),
                    "code_challenge_methods_supported": ["S256"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                    "authorization_response_iss_parameter_supported": true
                });
                let app = Router::new()
                    .route(
                        "/mcp",
                        get(move || {
                            let metadata_url = metadata_url.clone();
                            let require_step_up = require_step_up.clone();
                            async move {
                                let (status, challenge) =
                                    if require_step_up.load(Ordering::Acquire) {
                                        (
                                            StatusCode::FORBIDDEN,
                                            format!(
                                                "Bearer resource_metadata=\"{metadata_url}\", error=\"insufficient_scope\", scope=\"mcp.write\""
                                            ),
                                        )
                                    } else {
                                        (
                                            StatusCode::UNAUTHORIZED,
                                            format!(
                                                "Bearer resource_metadata=\"{metadata_url}\", scope=\"mcp.read\""
                                            ),
                                        )
                                    };
                                let mut response = status.into_response();
                                response
                                    .headers_mut()
                                    .insert(WWW_AUTHENTICATE, challenge.parse().unwrap());
                                response
                            }
                        }),
                    )
                    .route(
                        "/.well-known/oauth-protected-resource/mcp",
                        get({
                            let resource_metadata = resource_metadata.clone();
                            move || {
                                let resource_metadata = resource_metadata.clone();
                                async move { axum::Json(resource_metadata) }
                            }
                        }),
                    )
                    .route(
                        "/.well-known/oauth-protected-resource",
                        get(move || {
                            let resource_metadata = resource_metadata.clone();
                            async move { axum::Json(resource_metadata) }
                        }),
                    )
                    .route(
                        "/.well-known/oauth-authorization-server",
                        get(move || {
                            let server_metadata = server_metadata.clone();
                            async move { axum::Json(server_metadata) }
                        }),
                    )
                    .route(
                        "/token",
                        post(move |body: String| {
                            let token_requests = token_requests.clone();
                            let accepted_token = accepted_token.clone();
                            async move {
                                let request = token_requests.fetch_add(1, Ordering::Relaxed);
                                if body.contains("grant_type=authorization_code") {
                                    accepted_token.notify_waiters();
                                    axum::Json(json!({
                                        "access_token": "synthetic-access-one",
                                        "token_type": "Bearer",
                                        "expires_in": 60,
                                        "refresh_token": "synthetic-refresh-one",
                                        "scope": "mcp.read"
                                    }))
                                } else {
                                    assert!(body.contains("grant_type=refresh_token"));
                                    assert!(body.contains("refresh_token=synthetic-refresh-one"));
                                    assert_eq!(request, 1);
                                    axum::Json(json!({
                                        "access_token": "synthetic-access-two",
                                        "token_type": "Bearer",
                                        "expires_in": 120,
                                        "refresh_token": "synthetic-refresh-two",
                                        "scope": "mcp.read"
                                    }))
                                }
                            }
                        }),
                    )
                    .route(
                        "/revoke",
                        post(move |body: String| {
                            let revocations = revocations.clone();
                            let revoked_body = revoked_body.clone();
                            async move {
                                *revoked_body.lock().await = Some(body);
                                revocations.fetch_add(1, Ordering::Relaxed);
                                StatusCode::OK
                            }
                        }),
                    );
                axum::serve(socket, app).await.unwrap();
            }
        });

        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let private_parent =
            Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        let data = private_parent
            .create_private_directory(std::ffi::OsStr::new("data"))
            .unwrap();
        let root = Arc::new(
            Directory::open(project.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        let snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project.path(),
            None,
            None,
            None,
            &["/mcp"],
            InvocationOverrides::default(),
        )
        .unwrap();
        let store = Arc::new(
            McpCredentialStore::new(data.path(), root, snapshot.manifest().full_digest()).unwrap(),
        );
        let config = [(
            "test".into(),
            McpConfig {
                url: Some(format!("{base}/mcp")),
                oauth: Some(McpOAuthConfig {
                    enabled: true,
                    client_id: Some("native-client".into()),
                    scopes: vec!["mcp.read".into(), "mcp.write".into()],
                    ..Default::default()
                }),
                ..Default::default()
            },
        )]
        .into();
        let hosts = McpHosts::new(project.path(), &config).unwrap();
        hosts.install_credentials(store.clone()).unwrap();

        let result: Result<()> = async {
            let login = hosts.begin_oauth_browser("test").await?;
            let authorization = url::Url::parse(login.authorization_url())?;
            let parameters = authorization
                .query_pairs()
                .into_owned()
                .collect::<BTreeMap<_, _>>();
            let redirect = parameters
                .get("redirect_uri")
                .context("authorization URL omitted redirect_uri")?;
            let state = parameters
                .get("state")
                .context("authorization URL omitted state")?;
            let mut callback = url::Url::parse(redirect)?;
            callback
                .query_pairs_mut()
                .append_pair("code", "synthetic-code")
                .append_pair("state", state)
                .append_pair("iss", &issuer);
            let callback_task = tokio::spawn(async move {
                http::client()?
                    .get(callback)
                    .send()
                    .await?
                    .error_for_status()?;
                Ok::<_, anyhow::Error>(())
            });
            login
                .finish_with_cancellation(async {
                    accepted_token.notified().await;
                    Ok(())
                })
                .await?;
            callback_task.await??;

            let before = store
                .acquire("test")
                .await?
                .get()
                .await?
                .context("browser login did not publish a credential")?;
            let client = hosts.clients.get("test").context("missing MCP client")?;
            let (headers, refreshed_generation) = client
                .oauth_headers("test", Some(&store), HeaderMap::new(), true)
                .await?;
            ensure!(
                headers
                    .get(AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer synthetic-access-two"),
                "refresh did not return the rotated access token"
            );
            ensure!(
                refreshed_generation.is_some_and(|generation| generation != before.generation()),
                "refresh did not publish a new credential generation"
            );
            require_step_up.store(true, Ordering::Release);
            let step_up = hosts.begin_oauth_browser("test").await?;
            let step_up_url = url::Url::parse(step_up.authorization_url())?;
            let step_up_scope = step_up_url
                .query_pairs()
                .find(|(name, _)| name == "scope")
                .map(|(_, value)| value.into_owned());
            ensure!(
                step_up_scope.as_deref() == Some("mcp.read mcp.write"),
                "step-up did not retain the prior grant and authoritative challenge scope"
            );
            drop(step_up);
            let logout = hosts.oauth_logout("test").await?;
            ensure!(
                logout.local_deleted && logout.remote == "revoked",
                "unexpected logout outcome: {logout:?}"
            );
            let projected = serde_json::to_string(&logout)?;
            for secret in [
                "synthetic-code",
                "synthetic-access-one",
                "synthetic-access-two",
                "synthetic-refresh-one",
                "synthetic-refresh-two",
            ] {
                ensure!(!projected.contains(secret), "logout disclosed {secret}");
            }
            ensure!(store.acquire("test").await?.get().await?.is_none());
            Ok(())
        }
        .await;

        // Always remove a test-owned native credential if the assertion path
        // failed after publication.
        let cleanup = async {
            if let Some(record) = store.acquire("test").await?.get().await? {
                store
                    .acquire("test")
                    .await?
                    .delete(record.generation())
                    .await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let shutdown = hosts.shutdown().await;
        task.abort();
        cleanup.unwrap();
        shutdown.unwrap();
        let token_count = token_requests.load(Ordering::Relaxed);
        let revocation_count = revocations.load(Ordering::Relaxed);
        let revoked_body = revoked_body.lock().await.clone();
        if let Err(error) = result {
            panic!(
                "OAuth flow failed after {token_count} token requests and {revocation_count} revocations (body captured: {}): {error:#}",
                revoked_body.is_some()
            );
        }
        assert_eq!(token_count, 2);
        assert_eq!(revocation_count, 1);
        assert!(
            revoked_body
                .as_deref()
                .is_some_and(|body| body.contains("token=synthetic-refresh-two"))
        );
    }

    #[tokio::test]
    async fn logout_settles_local_deletion_across_cancellation_and_generation_races() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let revocations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let revocation_mode = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let issuer = format!("{base}/");
        let resource = format!("{base}/mcp");
        let task = tokio::spawn({
            let base = base.clone();
            let issuer = issuer.clone();
            let resource = resource.clone();
            let entered = entered.clone();
            let release = release.clone();
            let revocations = revocations.clone();
            let revocation_mode = revocation_mode.clone();
            async move {
                let resource_metadata = json!({
                    "resource": resource,
                    "authorization_servers": [issuer.clone()]
                });
                let app = Router::new()
                    .route(
                        "/.well-known/oauth-protected-resource/mcp",
                        get({
                            let metadata = resource_metadata.clone();
                            move || {
                                let metadata = metadata.clone();
                                async move { axum::Json(metadata) }
                            }
                        }),
                    )
                    .route(
                        "/.well-known/oauth-protected-resource",
                        get(move || {
                            let metadata = resource_metadata.clone();
                            async move { axum::Json(metadata) }
                        }),
                    )
                    .route(
                        "/.well-known/oauth-authorization-server",
                        get({
                            let base = base.clone();
                            let issuer = issuer.clone();
                            let revocation_mode = revocation_mode.clone();
                            move || {
                                let mut metadata = json!({
                                    "issuer": issuer,
                                    "authorization_endpoint": format!("{base}/authorize"),
                                    "token_endpoint": format!("{base}/token"),
                                    "code_challenge_methods_supported": ["S256"]
                                });
                                if revocation_mode.load(Ordering::Acquire) != 1 {
                                    metadata["revocation_endpoint"] =
                                        json!(format!("{base}/revoke"));
                                }
                                async move { axum::Json(metadata) }
                            }
                        }),
                    )
                    .route(
                        "/revoke",
                        post(move || {
                            let entered = entered.clone();
                            let release = release.clone();
                            let revocations = revocations.clone();
                            let revocation_mode = revocation_mode.clone();
                            async move {
                                revocations.fetch_add(1, Ordering::Relaxed);
                                match revocation_mode.load(Ordering::Acquire) {
                                    0 => {
                                        entered.notify_one();
                                        release.notified().await;
                                        StatusCode::OK.into_response()
                                    }
                                    2 => StatusCode::BAD_REQUEST.into_response(),
                                    3 => {
                                        let failed = futures::stream::once(async {
                                            Err::<Bytes, std::io::Error>(std::io::Error::new(
                                                std::io::ErrorKind::ConnectionReset,
                                                "fixture dropped the accepted revocation response",
                                            ))
                                        });
                                        Response::new(Body::from_stream(failed))
                                    }
                                    4 => vec![b'x'; 256 * 1024 + 1].into_response(),
                                    mode => panic!("unexpected revocation fixture mode {mode}"),
                                }
                            }
                        }),
                    );
                axum::serve(socket, app).await.unwrap();
            }
        });

        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let store = credential_store(project.path(), private.path());
        let mut credential =
            fixture_credential(&store, "auth", &resource, unix_time().unwrap() + 300);
        credential.issuer = oauth_binding(b"issuer", &issuer);
        let generation = store
            .acquire("auth")
            .await
            .unwrap()
            .create(credential.encode().unwrap())
            .await
            .unwrap();
        let config = [(
            "auth".into(),
            McpConfig {
                url: Some(resource.clone()),
                oauth: Some(McpOAuthConfig {
                    enabled: true,
                    client_id: Some("native-client".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )]
        .into();
        let hosts = Arc::new(McpHosts::new(project.path(), &config).unwrap());
        hosts.install_credentials(store.clone()).unwrap();

        // Hold the alias before logout's first poll. That poll admits the
        // detached settlement task but cannot dispatch revocation yet. A
        // concurrent refresh that already owns the lease settles first; the
        // cancelled logout then observes, revokes and deletes that exact new
        // generation rather than restoring either credential.
        let (lease, record) = store
            .acquire("auth")
            .await
            .unwrap()
            .read_locked()
            .await
            .unwrap();
        assert_eq!(record.unwrap().generation(), generation);
        let mut before_dispatch = Box::pin(hosts.oauth_logout("auth"));
        assert!(futures::poll!(before_dispatch.as_mut()).is_pending());
        drop(before_dispatch);
        assert_eq!(revocations.load(Ordering::Relaxed), 0);
        let mut refreshed =
            fixture_credential(&store, "auth", &resource, unix_time().unwrap() + 600);
        refreshed.issuer = oauth_binding(b"issuer", &issuer);
        lease
            .replace(generation, refreshed.encode().unwrap())
            .await
            .unwrap();
        entered.notified().await;
        release.notify_one();
        let current = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            store.acquire("auth").await.unwrap().get(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(current.is_none());

        let mut credential =
            fixture_credential(&store, "auth", &resource, unix_time().unwrap() + 300);
        credential.issuer = oauth_binding(b"issuer", &issuer);
        let generation = store
            .acquire("auth")
            .await
            .unwrap()
            .create(credential.encode().unwrap())
            .await
            .unwrap();
        let caller = tokio::spawn({
            let hosts = hosts.clone();
            async move { hosts.oauth_logout("auth").await }
        });
        entered.notified().await;
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        release.notify_one();

        let current = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            store.acquire("auth").await.unwrap().get(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(current.is_none());
        let stale = fixture_credential(&store, "auth", &base, unix_time().unwrap() + 300)
            .encode()
            .unwrap();
        assert!(
            store
                .acquire("auth")
                .await
                .unwrap()
                .replace(generation, stale)
                .await
                .is_err()
        );
        assert_eq!(revocations.load(Ordering::Relaxed), 2);

        for (mode, expected) in [
            (1, "no_advertised_endpoint"),
            (2, "remote_refused"),
            (3, "remote_outcome_uncertain"),
            (4, "remote_outcome_uncertain"),
        ] {
            revocation_mode.store(mode, Ordering::Release);
            let mut credential =
                fixture_credential(&store, "auth", &resource, unix_time().unwrap() + 300);
            credential.issuer = oauth_binding(b"issuer", &issuer);
            store
                .acquire("auth")
                .await
                .unwrap()
                .create(credential.encode().unwrap())
                .await
                .unwrap();
            let outcome = hosts.oauth_logout("auth").await.unwrap();
            assert!(outcome.local_deleted, "mode {mode}: {outcome:?}");
            assert_eq!(outcome.remote, expected, "mode {mode}: {outcome:?}");
            assert!(
                store
                    .acquire("auth")
                    .await
                    .unwrap()
                    .get()
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(revocations.load(Ordering::Relaxed), 5);

        hosts.shutdown().await.unwrap();
        task.abort();
    }

    fn tool(name: &str) -> Value {
        json!({"name":name,"description":"A fixture tool","inputSchema":{"type":"object"}})
    }

    #[tokio::test]
    async fn oauth_cached_catalog_rejects_changed_config_authority_and_generation() {
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let data = private.path().join("data");
        let (root, cache) = cache_store(project.path(), &data);
        let credentials = credential_store(project.path(), private.path());
        let resource = "http://127.0.0.1:9/mcp";
        let config = McpConfig {
            url: Some(resource.into()),
            oauth: Some(McpOAuthConfig {
                enabled: true,
                client_id: Some("native-client".into()),
                scopes: vec!["mcp.read".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut credential =
            fixture_credential(&credentials, "auth", resource, unix_time().unwrap() + 300);
        let first_generation = credentials
            .acquire("auth")
            .await
            .unwrap()
            .create(credential.encode().unwrap())
            .await
            .unwrap();
        let context =
            catalog_context("auth", &config, &HeaderMap::new(), Some(first_generation)).unwrap();
        let cached = vec![CachedMcpTool {
            original_name: "authorized-read".into(),
            description: "synthetic authorized metadata".into(),
            parameters: json!({"type":"object"}),
        }];
        cache.save("auth", context, cached.clone()).unwrap();
        assert_eq!(cache.load("auth", context).unwrap(), Some(cached));

        let mut changed_endpoint = config.clone();
        changed_endpoint.url = Some("http://127.0.0.1:9/other".into());
        let mut changed_client = config.clone();
        changed_client.oauth.as_mut().unwrap().client_id = Some("another-client".into());
        let mut changed_scopes = config.clone();
        changed_scopes.oauth.as_mut().unwrap().scopes = vec!["mcp.write".into()];
        for (field, changed) in [
            ("endpoint", changed_endpoint),
            ("client identity", changed_client),
            ("scopes", changed_scopes),
        ] {
            let changed_context =
                catalog_context("auth", &changed, &HeaderMap::new(), Some(first_generation))
                    .unwrap();
            assert_ne!(
                changed_context, context,
                "{field} kept the OAuth cache identity"
            );
            assert!(
                cache.load("auth", changed_context).is_err(),
                "{field} admitted the prior OAuth catalog record"
            );
        }

        credential.access_token = "synthetic-rotated-access".into();
        let next_generation = credentials
            .acquire("auth")
            .await
            .unwrap()
            .replace(first_generation, credential.encode().unwrap())
            .await
            .unwrap();
        assert_ne!(first_generation, next_generation);
        let rotated_context =
            catalog_context("auth", &config, &HeaderMap::new(), Some(next_generation)).unwrap();
        assert_ne!(rotated_context, context);
        assert!(cache.load("auth", rotated_context).is_err());

        let original_snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project.path(),
            None,
            None,
            None,
            &["/help", "/tools"],
            InvocationOverrides::default(),
        )
        .unwrap();
        std::fs::create_dir_all(project.path().join(".kuru")).unwrap();
        std::fs::write(
            project.path().join(".kuru/config.toml"),
            "allow_shell = true\n",
        )
        .unwrap();
        let changed_snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project.path(),
            None,
            None,
            None,
            &["/help", "/tools"],
            InvocationOverrides::default(),
        )
        .unwrap();
        let changed_authority =
            McpCatalogStore::new(&data, root, changed_snapshot.manifest().full_digest()).unwrap();
        assert_ne!(
            changed_snapshot.manifest().full_digest(),
            original_snapshot.manifest().full_digest()
        );
        assert!(
            changed_authority.load("auth", context).is_err(),
            "a changed reviewed authority admitted the prior OAuth catalog record"
        );
        credentials
            .acquire("auth")
            .await
            .unwrap()
            .delete(next_generation)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn oauth_issuer_change_and_logout_revoke_cached_tool_routes_until_live_discovery() {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let resource = format!("{base}/mcp");
        let changed_issuer = Arc::new(AtomicBool::new(false));
        let live = Arc::new(AtomicBool::new(true));
        let tool_lists = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pause_revocation = Arc::new(AtomicBool::new(false));
        let (revocation_entered, mut revocation_observed) = tokio::sync::watch::channel(false);
        let revocation_release = Arc::new(tokio::sync::Notify::new());
        let app = Router::new().fallback(axum::routing::any({
            let base = base.clone();
            let changed_issuer = changed_issuer.clone();
            let live = live.clone();
            let tool_lists = tool_lists.clone();
            let tool_calls = tool_calls.clone();
            let pause_revocation = pause_revocation.clone();
            let revocation_entered = revocation_entered.clone();
            let revocation_release = revocation_release.clone();
            move |method: axum::http::Method,
                  uri: axum::http::Uri,
                  headers: HeaderMap,
                  body: Bytes| {
                let base = base.clone();
                let changed_issuer = changed_issuer.clone();
                let live = live.clone();
                let tool_lists = tool_lists.clone();
                let tool_calls = tool_calls.clone();
                let pause_revocation = pause_revocation.clone();
                let revocation_entered = revocation_entered.clone();
                let revocation_release = revocation_release.clone();
                async move {
                    let issuer = if changed_issuer.load(Ordering::Acquire) {
                        format!("{base}/changed/")
                    } else {
                        format!("{base}/")
                    };
                    if method == axum::http::Method::GET
                        && uri.path().contains("oauth-protected-resource")
                    {
                        return axum::Json(json!({
                            "resource": format!("{base}/mcp"),
                            "authorization_servers": [issuer],
                            "scopes_supported": ["mcp.read"]
                        }))
                        .into_response();
                    }
                    if method == axum::http::Method::GET
                        && uri.path().contains("oauth-authorization-server")
                    {
                        let mut metadata = json!({
                            "issuer": issuer,
                            "authorization_endpoint": format!("{base}/authorize"),
                            "token_endpoint": format!("{base}/token"),
                            "code_challenge_methods_supported": ["S256"],
                            "grant_types_supported": ["authorization_code", "refresh_token"]
                        });
                        if pause_revocation.load(Ordering::Acquire) {
                            metadata["revocation_endpoint"] = json!(format!("{base}/revoke"));
                        }
                        return axum::Json(metadata).into_response();
                    }
                    if method == axum::http::Method::POST && uri.path() == "/revoke" {
                        let _ = revocation_entered.send(true);
                        revocation_release.notified().await;
                        return StatusCode::OK.into_response();
                    }
                    if method != axum::http::Method::POST || uri.path() != "/mcp" {
                        return StatusCode::NOT_FOUND.into_response();
                    }
                    if headers
                        .get(AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        != Some("Bearer synthetic-cache-access")
                    {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    if !live.load(Ordering::Acquire) {
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let result = match request["method"].as_str() {
                        Some("initialize") => json!({
                            "protocolVersion": VERSION,
                            "capabilities": {"tools": {}}
                        }),
                        Some("tools/list") => {
                            tool_lists.fetch_add(1, Ordering::Relaxed);
                            json!({"tools":[tool("authorized-read")]})
                        }
                        Some("tools/call") => {
                            tool_calls.fetch_add(1, Ordering::Relaxed);
                            json!({"content":[{"type":"text","text":"synthetic result"}]})
                        }
                        Some("notifications/initialized") => {
                            return StatusCode::ACCEPTED.into_response();
                        }
                        _ => return StatusCode::NOT_FOUND.into_response(),
                    };
                    axum::Json(json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": result
                    }))
                    .into_response()
                }
            }
        }));
        let task = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });

        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let credentials = credential_store(project.path(), private.path());
        let config = McpConfig {
            url: Some(resource.clone()),
            oauth: Some(McpOAuthConfig {
                enabled: true,
                client_id: Some("native-client".into()),
                scopes: vec!["mcp.read".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut credential =
            fixture_credential(&credentials, "auth", &resource, unix_time().unwrap() + 300);
        credential.issuer = oauth_binding(b"issuer", &format!("{base}/"));
        credential.client_secret = None;
        credential.access_token = "synthetic-cache-access".into();
        let generation = credentials
            .acquire("auth")
            .await
            .unwrap()
            .create(credential.encode().unwrap())
            .await
            .unwrap();
        let hosts = Arc::new(
            McpHosts::with_retained_root(root, &BTreeMap::from([("auth".into(), config.clone())]))
                .unwrap(),
        );
        hosts.install_cache(cache.clone()).unwrap();
        hosts.install_credentials(credentials.clone()).unwrap();

        let route = projected_name("auth", "authorized-read");
        let current = hosts.catalog().await.unwrap();
        assert_eq!(current.statuses[0].availability, McpAvailability::Live);
        assert_eq!(current.tools.len(), 1);
        assert!(hosts.selector(&route).await.is_ok());
        assert!(hosts.execute(&route, json!({})).await.is_ok());
        assert_eq!(tool_calls.load(Ordering::Relaxed), 1);
        let old_context =
            catalog_context("auth", &config, &HeaderMap::new(), Some(generation)).unwrap();
        assert!(cache.load("auth", old_context).unwrap().is_some());
        assert_eq!(tool_lists.load(Ordering::Relaxed), 1);

        changed_issuer.store(true, Ordering::Release);
        let mismatched = hosts.catalog().await.unwrap();
        assert_eq!(
            mismatched.statuses[0].availability,
            McpAvailability::Degraded
        );
        assert!(mismatched.tools.is_empty());
        assert!(hosts.selector(&route).await.is_err());
        assert!(hosts.execute(&route, json!({})).await.is_err());
        assert_eq!(tool_calls.load(Ordering::Relaxed), 1);
        assert_eq!(tool_lists.load(Ordering::Relaxed), 1);
        changed_issuer.store(false, Ordering::Release);
        assert_eq!(
            hosts.catalog().await.unwrap().statuses[0].availability,
            McpAvailability::Live
        );
        assert!(hosts.selector(&route).await.is_ok());
        assert_eq!(tool_lists.load(Ordering::Relaxed), 2);

        let logout = hosts.oauth_logout("auth").await.unwrap();
        assert!(logout.local_deleted);
        assert!(
            credentials
                .acquire("auth")
                .await
                .unwrap()
                .get()
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            hosts.selector(&route).await.is_err(),
            "logout retained an executable route from the authorized session"
        );
        assert!(hosts.execute(&route, json!({})).await.is_err());
        assert_eq!(tool_calls.load(Ordering::Relaxed), 1);
        let logged_out = hosts.catalog().await.unwrap();
        assert!(logged_out.tools.is_empty());
        assert!(hosts.selector(&route).await.is_err());
        assert_eq!(tool_lists.load(Ordering::Relaxed), 2);

        let next_generation = credentials
            .acquire("auth")
            .await
            .unwrap()
            .create(credential.encode().unwrap())
            .await
            .unwrap();
        live.store(false, Ordering::Release);
        let unavailable = hosts.catalog().await.unwrap();
        assert!(unavailable.tools.is_empty());
        assert!(hosts.selector(&route).await.is_err());
        live.store(true, Ordering::Release);
        let recovered = hosts.catalog().await.unwrap();
        assert_eq!(recovered.statuses[0].availability, McpAvailability::Live);
        assert_eq!(recovered.tools.len(), 1);
        assert!(hosts.selector(&route).await.is_ok());
        assert_eq!(tool_lists.load(Ordering::Relaxed), 3);
        assert!(hosts.execute(&route, json!({})).await.is_ok());
        assert_eq!(tool_calls.load(Ordering::Relaxed), 2);

        // The detached logout must retire a previously live route even if its
        // caller disappears after the remote revocation request is accepted.
        pause_revocation.store(true, Ordering::Release);
        let caller = tokio::spawn({
            let hosts = hosts.clone();
            async move { hosts.oauth_logout("auth").await }
        });
        tokio::time::timeout(Duration::from_secs(5), revocation_observed.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(*revocation_observed.borrow());
        caller.abort();
        let _ = caller.await;
        assert!(hosts.selector(&route).await.is_err());
        assert!(hosts.execute(&route, json!({})).await.is_err());
        assert_eq!(tool_calls.load(Ordering::Relaxed), 2);
        revocation_release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if credentials.acquire("auth").await?.get().await?.is_none() {
                    break Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
        .unwrap();
        assert!(hosts.selector(&route).await.is_err());
        assert!(hosts.execute(&route, json!({})).await.is_err());
        assert_eq!(tool_calls.load(Ordering::Relaxed), 2);
        hosts.shutdown().await.unwrap();
        assert_ne!(generation, next_generation);
        task.abort();
    }

    fn cache_store(project: &Path, private: &Path) -> (Arc<Directory>, Arc<McpCatalogStore>) {
        let root =
            Arc::new(Directory::open(project, Privacy::Inherited, NameRetention::Pinned).unwrap());
        let snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project,
            None,
            None,
            None,
            &["/help", "/tools"],
            InvocationOverrides::default(),
        )
        .unwrap();
        let store = Arc::new(
            McpCatalogStore::new(private, root.clone(), snapshot.manifest().full_digest()).unwrap(),
        );
        (root, store)
    }

    fn credential_store(project: &Path, private: &Path) -> Arc<McpCredentialStore> {
        let root =
            Arc::new(Directory::open(project, Privacy::Inherited, NameRetention::Pinned).unwrap());
        let parent = Directory::open(private, Privacy::Inherited, NameRetention::Movable).unwrap();
        let data = parent
            .create_private_directory(std::ffi::OsStr::new("oauth-data"))
            .unwrap();
        let snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project,
            None,
            None,
            None,
            &["/mcp", "/tools"],
            InvocationOverrides::default(),
        )
        .unwrap();
        Arc::new(
            McpCredentialStore::new(data.path(), root, snapshot.manifest().full_digest()).unwrap(),
        )
    }

    fn fixture_credential(
        store: &McpCredentialStore,
        alias: &str,
        resource: &str,
        expires_at: u64,
    ) -> McpOAuthCredential {
        let resource = checked_resource_url(resource).unwrap();
        McpOAuthCredential {
            authority: store.binding_authority(),
            project: store.project_identity(),
            alias: oauth_binding(b"alias", alias),
            resource: oauth_binding(b"resource", resource.as_str()),
            issuer: oauth_binding(b"issuer", "https://issuer.invalid/"),
            registration: McpRegistrationKind::Configured,
            expires_at,
            client_id: "native-client".into(),
            scopes: vec!["mcp.read".into()],
            client_secret: Some("recognizable-client-secret".into()),
            access_token: "recognizable-access-secret".into(),
            refresh_token: Some("recognizable-refresh-secret".into()),
        }
    }

    #[tokio::test]
    async fn oauth_status_tracks_mixed_alias_state_without_projecting_secrets() {
        let live =
            HttpFixture::new((0..5).map(|_| Reply::json(json!({}))).collect::<Vec<_>>()).await;
        let disabled = HttpFixture::new(Vec::new()).await;
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let store = credential_store(project.path(), private.path());
        let config = BTreeMap::from([
            (
                "auth".into(),
                McpConfig {
                    url: Some(live.url.clone()),
                    oauth: Some(McpOAuthConfig {
                        enabled: true,
                        client_id: Some("native-client".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            (
                "disabled".into(),
                McpConfig {
                    enabled: false,
                    url: Some(disabled.url.clone()),
                    oauth: Some(McpOAuthConfig {
                        enabled: true,
                        client_id: Some("native-client".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
        ]);
        let future = fixture_credential(&store, "auth", &live.url, unix_time().unwrap() + 300);
        let generation = store
            .acquire("auth")
            .await
            .unwrap()
            .create(future.encode().unwrap())
            .await
            .unwrap();
        let hosts = McpHosts::new(project.path(), &config).unwrap();
        hosts.install_credentials(store.clone()).unwrap();

        let authorized = hosts.oauth_status("auth").await.unwrap();
        assert_eq!(authorized.state, "authorized");
        assert_eq!(
            authorized.availability,
            McpAvailability::Degraded,
            "{authorized:?}"
        );
        assert_eq!(authorized.scopes, ["mcp.read"]);
        let disabled_status = hosts.oauth_status("disabled").await.unwrap();
        assert_eq!(disabled_status.state, "disabled");
        assert!(disabled.requests.lock().await.is_empty());
        let projected = serde_json::to_string(&(authorized, disabled_status)).unwrap();
        const SECRETS: [&str; 3] = [
            "recognizable-client-secret",
            "recognizable-access-secret",
            "recognizable-refresh-secret",
        ];
        for secret in SECRETS {
            assert!(!projected.contains(secret), "status disclosed {secret}");
        }

        let expired = fixture_credential(&store, "auth", &live.url, 1);
        let expired_generation = store
            .acquire("auth")
            .await
            .unwrap()
            .replace(generation, expired.encode().unwrap())
            .await
            .unwrap();
        assert_eq!(
            hosts.oauth_status("auth").await.unwrap().state,
            "refresh_required"
        );
        store
            .acquire("auth")
            .await
            .unwrap()
            .delete(expired_generation)
            .await
            .unwrap();
        assert_eq!(
            hosts.oauth_status("auth").await.unwrap().state,
            "login_required"
        );
        let catalog = hosts.catalog().await.unwrap();
        let projected = serde_json::to_string(&(catalog.tools, catalog.statuses)).unwrap();
        for secret in SECRETS {
            assert!(!projected.contains(secret), "catalog disclosed {secret}");
        }

        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn disabled_and_filtered_aliases_never_publish_routes_or_omitted_metadata() {
        let disabled = StdioFixture::new([Step::Read, Step::Eof]);
        let disabled_http = HttpFixture::new(Vec::new()).await;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[
                tool("read_public"),
                tool("read_secret"),
                tool("write"),
                {"name":"ignored_without_schema"},
                tool(&"x".repeat(257))
            ]})),
            Reply::json(json!({})),
        ])
        .await;
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let config = BTreeMap::from([
            (
                "disabled".into(),
                McpConfig {
                    enabled: false,
                    command: Some(disabled.command().into()),
                    ..Default::default()
                },
            ),
            (
                "disabled-http".into(),
                McpConfig {
                    enabled: false,
                    url: Some(disabled_http.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "http".into(),
                McpConfig {
                    url: Some(peer.url.clone()),
                    allow_tools: vec!["read_*".into()],
                    deny_tools: vec!["*_secret".into()],
                    ..Default::default()
                },
            ),
        ]);
        let (root_guard, cache) = cache_store(root.path(), &private.path().join("data"));
        let hosts = McpHosts::with_retained_root(root_guard, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert_eq!(catalog.statuses.len(), 3);
        assert_eq!(
            catalog.statuses[0].availability(),
            McpAvailability::Disabled
        );
        assert_eq!(
            catalog.statuses[1].availability(),
            McpAvailability::Disabled
        );
        assert_eq!(catalog.statuses[2].availability(), McpAvailability::Live);
        assert_eq!(catalog.tools.len(), 1);
        assert!(catalog.tools[0].description.contains("read_public"));
        assert!(!catalog.tools[0].description.contains("secret"));
        assert!(disabled.conversations().is_empty());
        assert!(disabled_http.requests.lock().await.is_empty());
        assert!(
            hosts
                .selector(&projected_name("http", "read_secret"))
                .await
                .is_err()
        );
        assert!(
            hosts
                .selector(&projected_name("http", "write"))
                .await
                .is_err()
        );
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn offline_restart_projects_cache_as_stale_without_an_executable_route() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::json(json!({})),
        ])
        .await;
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let config = http_config(&peer.url);
        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let hosts = McpHosts::with_retained_root(root, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let live = hosts.catalog().await.unwrap();
        assert_eq!(live.statuses[0].availability(), McpAvailability::Live);
        assert_eq!(live.tools.len(), 1);
        hosts.shutdown().await.unwrap();
        drop(hosts);
        drop(peer);

        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let restarted = McpHosts::with_retained_root(root, &config).unwrap();
        restarted.install_cache(cache).unwrap();
        let stale = restarted.catalog().await.unwrap();
        assert_eq!(stale.statuses[0].availability(), McpAvailability::Stale);
        assert_eq!(stale.tools.len(), 1);
        let name = projected_name("test", "read");
        assert!(stale.tools[0].description.contains("stale"));
        assert!(restarted.selector(&name).await.is_err());
        assert!(restarted.execute(&name, json!({})).await.is_err());
        restarted.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn healthy_rediscovery_repairs_a_corrupt_cache_without_losing_its_route() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::json(json!({})),
        ])
        .await;
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let config = http_config(&peer.url);
        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let hosts = McpHosts::with_retained_root(root, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        assert_eq!(
            hosts.catalog().await.unwrap().statuses[0].availability(),
            McpAvailability::Live
        );
        let catalog_directory = std::fs::read_dir(private.path().join("data"))
            .unwrap()
            .find_map(|entry| {
                let path = entry.unwrap().path();
                path.is_dir().then_some(path)
            })
            .unwrap();
        let record = std::fs::read_dir(catalog_directory)
            .unwrap()
            .find_map(|entry| {
                let path = entry.unwrap().path();
                (path.extension().and_then(|value| value.to_str()) == Some("json")).then_some(path)
            })
            .unwrap();
        std::fs::write(&record, b"{").unwrap();

        let repaired = hosts.catalog().await.unwrap();
        assert_eq!(repaired.statuses[0].availability(), McpAvailability::Live);
        assert!(repaired.statuses[0].diagnostic().is_none());
        assert!(
            hosts
                .selector(&projected_name("test", "read"))
                .await
                .is_ok()
        );
        let repaired: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
        assert_eq!(repaired["schema"], 1);
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cache_write_failure_keeps_live_route_and_reports_safe_diagnostic() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"done"}]})),
            Reply::json(json!({})),
        ])
        .await;
        let project = tempfile::tempdir().unwrap();
        let private_parent = tempfile::tempdir().unwrap();
        let private = private_parent.path().join("not-a-directory");
        let config = http_config(&peer.url);
        let (root, cache) = cache_store(project.path(), &private);
        std::fs::write(&private, b"fixture").unwrap();
        let hosts = McpHosts::with_retained_root(root, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert_eq!(catalog.statuses[0].availability(), McpAvailability::Live);
        assert_eq!(
            catalog.statuses[0].diagnostic(),
            Some("MCP catalog cache could not be updated")
        );
        let name = projected_name("test", "read");
        assert!(matches!(
            hosts.execute(&name, json!({})).await.unwrap(),
            McpExecution::Success(_)
        ));
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn mixed_alias_failures_preserve_live_and_stale_catalogs_independently() {
        let live_peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("live-read")]})),
            Reply::json(json!({})),
        ])
        .await;
        let disabled_peer = HttpFixture::new(Vec::new()).await;
        let stale_peer = HttpFixture::new(vec![Reply::json(json!({}))]).await;
        let failed_peer = HttpFixture::new(vec![Reply::json(json!({}))]).await;
        let corrupt_peer = HttpFixture::new(vec![Reply::json(json!({}))]).await;

        let config = BTreeMap::from([
            (
                "corrupt".into(),
                McpConfig {
                    url: Some(corrupt_peer.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "disabled".into(),
                McpConfig {
                    enabled: false,
                    url: Some(disabled_peer.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "failed".into(),
                McpConfig {
                    url: Some(failed_peer.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "live".into(),
                McpConfig {
                    url: Some(live_peer.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "stale".into(),
                McpConfig {
                    url: Some(stale_peer.url.clone()),
                    ..Default::default()
                },
            ),
        ]);
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let stale_context =
            catalog_context("stale", &config["stale"], &HeaderMap::new(), None).unwrap();
        cache
            .save(
                "stale",
                stale_context,
                vec![CachedMcpTool {
                    original_name: "cached-read".into(),
                    description: "retained metadata".into(),
                    parameters: json!({"type":"object"}),
                }],
            )
            .unwrap();
        cache
            .save(
                "corrupt",
                [0xff; 32],
                vec![CachedMcpTool {
                    original_name: "must-not-project".into(),
                    description: "wrong context".into(),
                    parameters: json!({"type":"object"}),
                }],
            )
            .unwrap();

        let hosts = McpHosts::with_retained_root(root, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        let states = catalog
            .statuses
            .iter()
            .map(|status| (status.alias(), status.availability()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(states["corrupt"], McpAvailability::Degraded);
        assert_eq!(states["disabled"], McpAvailability::Disabled);
        assert_eq!(states["failed"], McpAvailability::Degraded);
        assert_eq!(states["live"], McpAvailability::Live);
        assert_eq!(states["stale"], McpAvailability::Stale);
        assert_eq!(catalog.tools.len(), 2);
        assert!(
            catalog
                .tools
                .iter()
                .any(|tool| tool.description.contains("live-read"))
        );
        assert!(
            catalog
                .tools
                .iter()
                .any(|tool| tool.description.contains("stale"))
        );
        assert!(
            hosts
                .selector(&projected_name("live", "live-read"))
                .await
                .is_ok()
        );
        assert!(
            hosts
                .selector(&projected_name("stale", "cached-read"))
                .await
                .is_err()
        );
        assert!(disabled_peer.requests.lock().await.is_empty());
        assert_eq!(stale_peer.requests.lock().await.len(), 1);
        assert_eq!(failed_peer.requests.lock().await.len(), 1);
        assert_eq!(corrupt_peer.requests.lock().await.len(), 1);
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn static_header_environment_reference_applies_to_the_whole_http_session() {
        let expected = std::env::var("PATH").expect("test runner PATH is required");
        HeaderValue::from_str(&expected).expect("test runner PATH must be a valid HTTP value");
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"done"}]})),
            Reply::json(json!({})),
        ])
        .await;
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let config = BTreeMap::from([(
            "http".into(),
            McpConfig {
                url: Some(peer.url.clone()),
                header_env: BTreeMap::from([("X-Kuru-Test".into(), "PATH".into())]),
                ..Default::default()
            },
        )]);
        let (root_guard, cache) = cache_store(root.path(), &private.path().join("data"));
        let hosts = McpHosts::with_retained_root(root_guard, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let name = hosts.catalog().await.unwrap().tools[0].name.clone();
        assert!(matches!(
            hosts.execute(&name, json!({})).await.unwrap(),
            McpExecution::Success(_)
        ));
        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 5);
        for request in requests.iter() {
            assert_eq!(
                request
                    .headers
                    .get("x-kuru-test")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                expected
            );
        }
        assert!(requests[0].headers.get("mcp-protocol-version").is_none());
        assert!(
            requests[1..]
                .iter()
                .all(|request| request.headers.get("mcp-protocol-version").is_some())
        );
        let cached = std::fs::read_dir(private.path().join("data"))
            .unwrap()
            .flat_map(|entry| std::fs::read_dir(entry.unwrap().path()).unwrap())
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .map(std::fs::read)
            .unwrap()
            .unwrap();
        assert!(
            !cached
                .windows(expected.len())
                .any(|window| window == expected.as_bytes()),
            "resolved header values must not enter catalog records"
        );
    }

    #[tokio::test]
    async fn changed_http_headers_close_the_captured_session_before_rediscovery() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("first")]})),
            Reply::json(json!({})),
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("second")]})),
            Reply::json(json!({})),
        ])
        .await;
        let workspace = tempfile::tempdir().unwrap();
        let root = Arc::new(
            Directory::open(workspace.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        let client = McpClient {
            config: http_config(&peer.url).remove("test").unwrap(),
            root: root.path().to_path_buf(),
            root_guard: root,
            state: Mutex::new(ClientState::default()),
            available: AtomicBool::new(true),
            admission: Arc::new(Admission::new()),
            stop: Notify::new(),
        };
        let mut first = HeaderMap::new();
        first.insert("x-kuru-test", HeaderValue::from_static("first"));
        let mut second = HeaderMap::new();
        second.insert("x-kuru-test", HeaderValue::from_static("second"));

        let mut state = client.state.lock().await;
        assert_eq!(
            client.list(&mut state, first).await.unwrap()[0]["name"],
            "first"
        );
        assert_eq!(
            client.list(&mut state, second).await.unwrap()[0]["name"],
            "second"
        );
        drop(state);
        client.close().await.unwrap();

        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 8);
        for request in &requests[..4] {
            assert_eq!(request.headers["x-kuru-test"], "first");
        }
        for request in &requests[4..] {
            assert_eq!(request.headers["x-kuru-test"], "second");
        }
    }

    #[tokio::test]
    async fn missing_static_header_reference_degrades_without_dispatch_or_value_diagnostic() {
        let peer = HttpFixture::new(Vec::new()).await;
        let root = tempfile::tempdir().unwrap();
        let environment = "KURU_TEST_MCP_HEADER_ENVIRONMENT_MUST_REMAIN_ABSENT_6F9320";
        assert!(std::env::var_os(environment).is_none());
        let config = BTreeMap::from([(
            "http".into(),
            McpConfig {
                url: Some(peer.url.clone()),
                header_env: BTreeMap::from([("Authorization".into(), environment.into())]),
                ..Default::default()
            },
        )]);
        let hosts = McpHosts::new(root.path(), &config).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert_eq!(
            catalog.statuses[0].availability(),
            McpAvailability::Degraded
        );
        assert_eq!(
            catalog.statuses[0].diagnostic(),
            Some("configured MCP static header is unavailable")
        );
        assert!(peer.requests.lock().await.is_empty());
        hosts.shutdown().await.unwrap();
    }

    #[test]
    fn static_header_values_are_bounded_and_value_free_on_rejection() {
        let secret = "recognizable-static-header-secret";
        let invalid = static_header_value("MCP_AUTH", format!("{secret}\n").into())
            .expect_err("a newline-bearing header value was accepted")
            .to_string();
        assert!(!invalid.contains(secret));

        let oversized_value = format!("{secret}{}", "x".repeat(MAX_STATIC_HEADER_VALUE_BYTES));
        let oversized = static_header_value("MCP_AUTH", oversized_value.into())
            .expect_err("an oversized header value was accepted")
            .to_string();
        assert!(!oversized.contains(secret));

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            let non_unicode = static_header_value(
                "MCP_AUTH",
                std::ffi::OsString::from_vec(vec![b's', b'e', b'c', b'r', b'e', b't', 0xff]),
            )
            .expect_err("a non-Unicode header value was accepted")
            .to_string();
            assert!(!non_unicode.contains("secret"));
        }
    }

    #[tokio::test]
    async fn http_session_paginated_discovery_call_and_shutdown() {
        let peer = HttpFixture::new(vec![initialized(),Reply::json(json!({})),Reply::rpc(json!({"tools":[tool("file_read")],"nextCursor":"p2"})),Reply::sse(json!({"tools":[tool("second")]})),Reply::sse(json!({"content":[{"type":"text","text":"called"}],"structuredContent":{"value":42}})),Reply::json(json!({}))]).await;
        let root = tempfile::tempdir().unwrap();
        let hosts = McpHosts::new(root.path(), &http_config(&peer.url)).unwrap();
        let specs = hosts.specs().await.unwrap();
        assert_eq!(specs.len(), 2);
        assert_ne!(specs[0].name, "file_read");
        assert!(specs[0].name.len() < 64);
        let McpExecution::Success(value) = hosts
            .execute(&specs[0].name, json!({"path":"a"}))
            .await
            .unwrap()
        else {
            panic!("expected an MCP success");
        };
        assert_eq!(value["structuredContent"]["value"], 42);
        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests[0].body["params"]["protocolVersion"], VERSION);
        assert_eq!(requests[1].body["method"], "notifications/initialized");
        assert_eq!(requests[1].headers["mcp-session-id"], "fixture-session");
        assert_eq!(requests[2].headers["mcp-protocol-version"], VERSION);
        assert_eq!(requests[3].body["params"]["cursor"], "p2");
        assert_eq!(requests[4].body["params"]["name"], "file_read");
        assert_eq!(requests[5].method, axum::http::Method::DELETE);
    }

    #[tokio::test]
    async fn stdio_discovery_preserves_server_state_and_sequential_concurrency() {
        let counter = json!({"tools":[{"name":"counter","inputSchema":{"type":"object"}}]});
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":counter})),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"1"}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"2"}]}}),
            ),
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":5,"result":counter})),
            Step::Eof,
        ]);
        let root = tempfile::tempdir().unwrap();
        let config = [(
            "stdio".into(),
            McpConfig {
                command: Some(script.command().into()),
                args: vec![],
                url: None,
                env: BTreeMap::new(),
                ..Default::default()
            },
        )]
        .into();
        let hosts = McpHosts::new(root.path(), &config).unwrap();
        let specs = hosts.specs().await.unwrap();
        let (one, two) = tokio::join!(
            hosts.execute(&specs[0].name, json!({})),
            hosts.execute(&specs[0].name, json!({}))
        );
        let McpExecution::Success(one) = one.unwrap() else {
            panic!("expected first MCP success");
        };
        let McpExecution::Success(two) = two.unwrap() else {
            panic!("expected second MCP success");
        };
        assert_eq!(one["content"][0]["text"], "1");
        assert_eq!(two["content"][0]["text"], "2");
        assert_eq!(hosts.specs().await.unwrap()[0].name, specs[0].name);
        hosts.shutdown().await.unwrap();
        script.assert_completed(1);
        let requests = script.conversations().remove(0);
        let methods: Vec<_> = requests
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/call",
                "tools/call",
                "tools/list"
            ]
        );
        assert_eq!(requests[3]["params"]["name"], "counter");
        assert_eq!(requests[4]["params"]["name"], "counter");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_stdio_mcp_keeps_inherited_environment_and_configured_override() {
        const CHILD: &str = "KURU_MCP_ENVIRONMENT_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let script = StdioFixture::new([
                Step::Read,
                Step::Write(
                    json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
                ),
                Step::Read,
                Step::Read,
                Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}})),
                Step::Read,
                Step::Eof,
            ]);
            let root = tempfile::tempdir().unwrap();
            let config = [(
                "stdio".into(),
                McpConfig {
                    command: Some(script.command().into()),
                    env: BTreeMap::from([(
                        "KURU_MCP_CONFIGURED_SENTINEL".into(),
                        "configured".into(),
                    )]),
                    ..Default::default()
                },
            )]
            .into();
            let hosts = McpHosts::new(root.path(), &config).unwrap();
            let specs = hosts.specs().await;
            let shutdown = hosts.shutdown().await;
            assert!(specs.unwrap().is_empty());
            shutdown.unwrap();
            assert_eq!(
                script.environment_observations(),
                vec!["inherited=true\noverride=true\n"]
            );
            return;
        }
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        let path = std::env::var_os("PATH").expect("test runner PATH is required for rustc");
        child
            .arg("--exact")
            .arg("mcp::tests::isolated_stdio_mcp_keeps_inherited_environment_and_configured_override")
            .arg("--nocapture")
            .env_clear()
            // The real stdio peer is compiled in the isolated child. Preserve
            // only its executable-discovery input; the observed MCP values are
            // controlled fake sentinels below, never dumped by the fixture.
            .env("PATH", path)
            .env("KURU_MCP_INHERITED_SENTINEL", "inherited")
            .env("KURU_MCP_CONFIGURED_SENTINEL", "parent-value")
            .env(CHILD, "1")
            .kill_on_drop(true);
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            child.env("LLVM_PROFILE_FILE", profile);
        }
        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
        let mut child = child.spawn().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let result = timeout(Duration::from_secs(15), async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let (stdout_truncated, stderr_truncated, status) = tokio::join!(
                drain_bounded(&mut stdout, &mut out),
                drain_bounded(&mut stderr, &mut err),
                child.wait(),
            );
            if stdout_truncated? || stderr_truncated? {
                return Err(std::io::Error::other(
                    "isolated MCP fixture output exceeds 2 MiB",
                ));
            }
            Ok::<_, std::io::Error>((status?, out, err))
        })
        .await;
        let (status, out, err) = match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                let requested = child.start_kill();
                let (stdout_drain, stderr_drain, reap) = tokio::join!(
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stdout, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stderr, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), child.wait()),
                );
                panic!(
                    "isolated MCP environment fixture failed: {error}; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}; reap={reap:?}"
                );
            }
            Err(_) => {
                let requested = child.start_kill();
                let (stdout_drain, stderr_drain, reap) = tokio::join!(
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stdout, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stderr, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), child.wait()),
                );
                panic!(
                    "isolated MCP environment fixture timed out; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}; reap={reap:?}"
                );
            }
        };
        assert!(
            status.success(),
            "isolated MCP environment fixture failed: stdout={} stderr={}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retained_root_replacement_blocks_stdio_discovery_before_child_spawn() {
        let script = StdioFixture::new([Step::Read, Step::Eof]);
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let retained =
            Arc::new(Directory::open(&root, Privacy::Inherited, NameRetention::Pinned).unwrap());
        let config = [(
            "stdio".into(),
            McpConfig {
                command: Some(script.command().into()),
                args: vec![],
                url: None,
                env: BTreeMap::new(),
                ..Default::default()
            },
        )]
        .into();
        let hosts = McpHosts::with_retained_root(retained, &config).unwrap();
        std::fs::rename(&root, parent.path().join("replaced")).unwrap();
        std::fs::create_dir(&root).unwrap();

        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        assert!(script.conversations().is_empty());
    }

    #[tokio::test]
    async fn discovery_rejects_incompatible_version_duplicates_and_stalled_pagination() {
        for (reply, expected) in [
            (
                json!({"protocolVersion":"2099-01-01","capabilities":{"tools":{}}}),
                "unsupported",
            ),
            (
                json!({"protocolVersion":VERSION,"capabilities":{}}),
                "advertise",
            ),
            (json!({}), "protocolVersion"),
        ] {
            let peer = HttpFixture::new(vec![Reply::rpc(reply)]).await;
            let workspace = tempfile::tempdir().unwrap();
            let root = Arc::new(
                Directory::open(workspace.path(), Privacy::Inherited, NameRetention::Pinned)
                    .unwrap(),
            );
            let client = McpClient {
                config: http_config(&peer.url).remove("test").unwrap(),
                root: root.path().to_path_buf(),
                root_guard: root,
                state: Mutex::new(ClientState::default()),
                available: AtomicBool::new(false),
                admission: Arc::new(Admission::new()),
                stop: Notify::new(),
            };
            let mut state = client.state.lock().await;
            assert!(
                client
                    .list(&mut state, HeaderMap::new())
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("a"),tool("a")]})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        hosts.shutdown().await.unwrap();
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[],"nextCursor":"same"})),
            Reply::rpc(json!({"tools":[],"nextCursor":"same"})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        hosts.shutdown().await.unwrap();

        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":(0..=10_000).map(|index| tool(&format!("tool-{index}"))).collect::<Vec<_>>()})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn tool_application_error_retains_the_session_while_protocol_failure_is_typed() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("a")]})),
            Reply::rpc(json!({"isError":true,"content":[{"type":"text","text":"denied"}]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"usable"}]})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":"server failed"}})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let name = &hosts.specs().await.unwrap()[0].name;
        let McpExecution::ApplicationError(content) = hosts.execute(name, json!({})).await.unwrap()
        else {
            panic!("expected MCP application error");
        };
        assert_eq!(content[0]["text"], "denied");
        let McpExecution::Success(success) = hosts.execute(name, json!({})).await.unwrap() else {
            panic!("expected MCP success after application error");
        };
        assert_eq!(success["content"][0]["text"], "usable");
        let failure = hosts.execute(name, json!({})).await.unwrap_err();
        assert_eq!(failure.kind, ToolFailureKind::McpCall);
        assert_eq!(
            failure.error.to_string(),
            "configured MCP server test is unavailable"
        );
        assert_eq!(peer.requests.lock().await.len(), 7);
        hosts.shutdown().await.unwrap();
        let failure = hosts.execute("unknown", json!({})).await.unwrap_err();
        assert_eq!(failure.kind, ToolFailureKind::McpRoute);
        let invalid = [(
            "bad".into(),
            McpConfig {
                command: None,
                args: vec![],
                url: None,
                env: BTreeMap::new(),
                ..Default::default()
            },
        )]
        .into();
        assert!(McpHosts::new(Path::new("."), &invalid).is_err());
    }

    #[tokio::test]
    async fn failed_call_retains_unconfirmed_http_close_without_replay() {
        let mut rejected = Reply::json(json!({}));
        rejected.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("mutate")]})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":"lost"}})),
            rejected,
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let name = hosts.specs().await.unwrap()[0].name.clone();
        let failure = hosts.execute(&name, json!({})).await.unwrap_err();
        assert_eq!(failure.kind, ToolFailureKind::McpCall);
        assert_eq!(peer.requests.lock().await.len(), 5);

        let disabled = hosts.execute(&name, json!({})).await.unwrap_err();
        assert_eq!(disabled.kind, ToolFailureKind::McpCall);
        assert_eq!(peer.requests.lock().await.len(), 5);
        assert!(
            hosts.clients["test"].state.lock().await.transport.is_some(),
            "failed session cleanup must remain observable"
        );

        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 6);
        assert_eq!(requests[3].body["method"], "tools/call");
        assert_eq!(requests[4].method, axum::http::Method::DELETE);
        assert_eq!(requests[5].method, axum::http::Method::DELETE);
    }

    #[tokio::test]
    async fn explicit_catalog_recovers_a_failed_alias_without_replaying_its_call() {
        let recovering = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("mutate")]})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":"lost"}})),
            Reply::json(json!({})),
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("mutate")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"recovered"}]})),
            Reply::json(json!({})),
        ])
        .await;
        let steady = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("steady")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"before"}]})),
            Reply::rpc(json!({"tools":[tool("steady")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"after"}]})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(
            Path::new("."),
            &BTreeMap::from([
                (
                    "recovering".into(),
                    McpConfig {
                        url: Some(recovering.url.clone()),
                        ..Default::default()
                    },
                ),
                (
                    "steady".into(),
                    McpConfig {
                        url: Some(steady.url.clone()),
                        ..Default::default()
                    },
                ),
            ]),
        )
        .unwrap();
        let initial = hosts.catalog().await.unwrap();
        let recovering_name = initial
            .tools
            .iter()
            .find(|tool| tool.description.starts_with("MCP recovering/"))
            .unwrap()
            .name
            .clone();
        let steady_name = initial
            .tools
            .iter()
            .find(|tool| tool.description.starts_with("MCP steady/"))
            .unwrap()
            .name
            .clone();
        let McpExecution::Success(before) = hosts.execute(&steady_name, json!({})).await.unwrap()
        else {
            panic!("expected independent alias success before recovery");
        };
        assert_eq!(before["content"][0]["text"], "before");
        assert!(hosts.execute(&recovering_name, json!({})).await.is_err());
        assert_eq!(recovering.requests.lock().await.len(), 5);
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.statuses.iter().all(McpStatus::available));
        assert!(
            catalog
                .tools
                .iter()
                .any(|tool| tool.name == recovering_name)
        );
        assert!(catalog.tools.iter().any(|tool| tool.name == steady_name));
        assert_eq!(recovering.requests.lock().await.len(), 8);
        let McpExecution::Success(result) =
            hosts.execute(&recovering_name, json!({})).await.unwrap()
        else {
            panic!("expected recovered call success");
        };
        assert_eq!(result["content"][0]["text"], "recovered");
        let McpExecution::Success(after) = hosts.execute(&steady_name, json!({})).await.unwrap()
        else {
            panic!("expected independent alias success after recovery");
        };
        assert_eq!(after["content"][0]["text"], "after");
        hosts.shutdown().await.unwrap();
        let requests = recovering.requests.lock().await;
        assert_eq!(requests.len(), 10);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.body["method"] == "tools/call")
                .count(),
            2
        );
        let steady_requests = steady.requests.lock().await;
        assert_eq!(
            steady_requests
                .iter()
                .filter(|request| request.body["method"] == "tools/call")
                .count(),
            2
        );
        assert_eq!(steady_requests[3].body["params"]["name"], "steady");
        assert_eq!(steady_requests[5].body["params"]["name"], "steady");
    }

    #[tokio::test]
    async fn received_stdio_mutation_disconnects_once_and_disables_later_calls() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("mutate")]}})),
            Step::Read,
        ]);
        let hosts = McpHosts::new(
            Path::new("."),
            &BTreeMap::from([(
                "stdio".into(),
                McpConfig {
                    command: Some(script.command().into()),
                    ..Default::default()
                },
            )]),
        )
        .unwrap();
        let name = hosts.specs().await.unwrap()[0].name.clone();

        let failure = hosts.execute(&name, json!({"write":"once"})).await;
        assert!(failure.is_err());
        script.assert_completed(1);
        let requests = script.conversations();
        assert_eq!(requests[0].len(), 4);
        assert_eq!(requests[0][3]["method"], "tools/call");
        assert_eq!(requests[0][3]["params"]["name"], "mutate");
        assert_eq!(requests[0][3]["params"]["arguments"]["write"], "once");

        for _ in 0..2 {
            let disabled = hosts.execute(&name, json!({"write":"again"})).await;
            assert_eq!(disabled.unwrap_err().kind, ToolFailureKind::McpCall);
        }
        assert_eq!(script.conversations()[0].len(), 4);
        hosts.shutdown().await.unwrap();
        assert_eq!(script.conversations()[0].len(), 4);
    }

    #[tokio::test]
    async fn repeated_catalog_does_not_replace_unconfirmed_stdio_cleanup() {
        let replacement = StdioFixture::new([Step::Read]);
        let hosts = McpHosts::new(
            Path::new("."),
            &BTreeMap::from([(
                "stdio".into(),
                McpConfig {
                    command: Some(replacement.command().into()),
                    ..Default::default()
                },
            )]),
        )
        .unwrap();
        hosts.clients["stdio"].state.lock().await.transport =
            Some(Transport::Stdio(Box::new(Rpc::unconfirmed_for_test())));

        for _ in 0..2 {
            let catalog = hosts.catalog().await.unwrap();
            assert!(catalog.tools.is_empty());
            assert!(!catalog.statuses[0].available());
            assert!(
                hosts.clients["stdio"]
                    .state
                    .lock()
                    .await
                    .transport
                    .is_some()
            );
            assert!(replacement.conversations().is_empty());
        }
        assert!(hosts.shutdown().await.is_err());
        assert!(replacement.conversations().is_empty());
    }

    #[tokio::test]
    async fn cancelled_stdio_call_is_never_dispatched_twice() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("mutate")]}})),
            Step::Read,
            Step::Sleep(5_000),
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        let name = hosts.specs().await.unwrap()[0].name.clone();

        let held = hosts.clients["stdio"].state.lock().await;
        let before_hosts = hosts.clone();
        let before_name = name.clone();
        let before =
            tokio::spawn(async move { before_hosts.execute(&before_name, json!({})).await });
        tokio::task::yield_now().await;
        before.abort();
        assert!(before.await.unwrap_err().is_cancelled());
        drop(held);
        assert_eq!(script.conversations()[0].len(), 3);

        let after_hosts = hosts.clone();
        let after_name = name.clone();
        let after = tokio::spawn(async move { after_hosts.execute(&after_name, json!({})).await });
        script.wait_for_requests(4).await;
        after.abort();
        assert!(after.await.unwrap_err().is_cancelled());
        let disabled = hosts.execute(&name, json!({})).await.unwrap_err();
        assert_eq!(disabled.kind, ToolFailureKind::McpCall);
        assert_eq!(script.conversations()[0].len(), 4);
        hosts.shutdown().await.unwrap();
        assert_eq!(script.conversations()[0].len(), 4);
    }

    #[tokio::test]
    async fn catalog_keeps_healthy_http_alias_when_stdio_alias_fails_in_either_order() {
        for failed_first in [true, false] {
            let healthy = HttpFixture::new(vec![
                initialized(),
                Reply::json(json!({})),
                Reply::rpc(json!({"tools":[tool("usable")]})),
                Reply::json(json!({})),
            ])
            .await;
            let failed = StdioFixture::new([Step::Read, Step::Raw("not JSON"), Step::Read]);
            let (failed_alias, healthy_alias) = if failed_first {
                ("a_failed", "z_healthy")
            } else {
                ("z_failed", "a_healthy")
            };
            let config = BTreeMap::from([
                (
                    failed_alias.into(),
                    McpConfig {
                        command: Some(failed.command().into()),
                        ..Default::default()
                    },
                ),
                (
                    healthy_alias.into(),
                    McpConfig {
                        url: Some(healthy.url.clone()),
                        ..Default::default()
                    },
                ),
            ]);
            let hosts = McpHosts::new(Path::new("."), &config).unwrap();
            let catalog = hosts.catalog().await.unwrap();
            assert_eq!(catalog.tools.len(), 1);
            assert_eq!(catalog.statuses.len(), 2);
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| status.alias() == healthy_alias && status.available())
            );
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| status.alias() == failed_alias && !status.available())
            );
            hosts.shutdown().await.unwrap();
            assert_eq!(
                failed.conversations(),
                vec![vec![json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": VERSION,
                        "capabilities": {},
                        "clientInfo": {
                            "name": "kuru",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    },
                })]],
            );
        }
    }

    #[tokio::test]
    async fn catalog_keeps_healthy_stdio_alias_when_http_alias_fails_in_either_order() {
        for failed_first in [true, false] {
            let healthy = StdioFixture::new([
                Step::Read,
                Step::Write(
                    json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
                ),
                Step::Read,
                Step::Read,
                Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("usable")]}})),
                Step::Read,
                Step::Write(
                    json!({"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"used"}]}}),
                ),
                Step::Eof,
            ]);
            let mut rejected = Reply::json(json!({}));
            rejected.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
            let failed = HttpFixture::new(vec![rejected]).await;
            let (failed_alias, healthy_alias) = if failed_first {
                ("a_failed", "z_healthy")
            } else {
                ("z_failed", "a_healthy")
            };
            let config = BTreeMap::from([
                (
                    failed_alias.into(),
                    McpConfig {
                        url: Some(failed.url.clone()),
                        ..Default::default()
                    },
                ),
                (
                    healthy_alias.into(),
                    McpConfig {
                        command: Some(healthy.command().into()),
                        ..Default::default()
                    },
                ),
            ]);
            let hosts = McpHosts::new(Path::new("."), &config).unwrap();
            let catalog = hosts.catalog().await.unwrap();
            assert_eq!(catalog.tools.len(), 1);
            assert_eq!(catalog.statuses.len(), 2);
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| status.alias() == healthy_alias && status.available())
            );
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| status.alias() == failed_alias && !status.available())
            );
            let McpExecution::Success(result) = hosts
                .execute(&catalog.tools[0].name, json!({}))
                .await
                .unwrap()
            else {
                panic!("expected healthy stdio alias to remain callable");
            };
            assert_eq!(result["content"][0]["text"], "used");
            hosts.shutdown().await.unwrap();
            healthy.assert_completed(1);
            assert_eq!(failed.requests.lock().await.len(), 1);
        }
    }

    #[tokio::test]
    async fn failed_later_catalog_page_never_publishes_partial_routes() {
        for failure in ["malformed", "repeated", "oversized"] {
            let second_page = match failure {
                "malformed" => Reply::rpc(json!({})),
                "repeated" => Reply::rpc(json!({"tools":[],"nextCursor":"p2"})),
                "oversized" => Reply::rpc(json!({
                    "tools": (0..10_000)
                        .map(|index| tool(&format!("extra-{index}")))
                        .collect::<Vec<_>>()
                })),
                _ => unreachable!(),
            };
            let failed = HttpFixture::new(vec![
                initialized(),
                Reply::json(json!({})),
                Reply::rpc(json!({"tools":[tool("partial")],"nextCursor":"p2"})),
                second_page,
                Reply::json(json!({})),
            ])
            .await;
            let healthy = HttpFixture::new(vec![
                initialized(),
                Reply::json(json!({})),
                Reply::rpc(json!({"tools":[tool("usable")]})),
                Reply::json(json!({})),
            ])
            .await;
            let hosts = McpHosts::new(
                Path::new("."),
                &BTreeMap::from([
                    (
                        "failed".into(),
                        McpConfig {
                            url: Some(failed.url.clone()),
                            ..Default::default()
                        },
                    ),
                    (
                        "healthy".into(),
                        McpConfig {
                            url: Some(healthy.url.clone()),
                            ..Default::default()
                        },
                    ),
                ]),
            )
            .unwrap();

            let catalog = hosts.catalog().await.unwrap();
            assert_eq!(catalog.tools.len(), 1, "{failure}");
            assert!(catalog.tools[0].description.contains("healthy/usable"));
            assert_eq!(hosts.routes.read().await.len(), 1, "{failure}");
            assert!(
                hosts
                    .routes
                    .read()
                    .await
                    .values()
                    .all(|route| route.0 == "healthy")
            );
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| { status.alias() == "failed" && !status.available() })
            );
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| { status.alias() == "healthy" && status.available() })
            );
            hosts.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn multipage_discovery_serializes_same_alias_while_other_alias_proceeds() {
        let slow = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("target")]}})),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":3,"result":{"tools":[tool("target")],"nextCursor":"p2"}}),
            ),
            Step::Read,
            Step::Sleep(400),
            Step::Write(json!({"jsonrpc":"2.0","id":4,"result":{"tools":[tool("extra")]}})),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"text","text":"slow done"}]}}),
            ),
            Step::Eof,
        ]);
        let independent = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("other")]}})),
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":3,"result":{"tools":[tool("other")]}})),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"other done"}]}}),
            ),
            Step::Eof,
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([
                    (
                        "independent".into(),
                        McpConfig {
                            command: Some(independent.command().into()),
                            ..Default::default()
                        },
                    ),
                    (
                        "slow".into(),
                        McpConfig {
                            command: Some(slow.command().into()),
                            ..Default::default()
                        },
                    ),
                ]),
            )
            .unwrap(),
        );
        let initial = hosts.catalog().await.unwrap();
        let slow_name = initial
            .tools
            .iter()
            .find(|tool| tool.description.contains("slow/target"))
            .unwrap()
            .name
            .clone();
        let independent_name = initial
            .tools
            .iter()
            .find(|tool| tool.description.contains("independent/other"))
            .unwrap()
            .name
            .clone();

        let catalog_hosts = hosts.clone();
        let catalog = tokio::spawn(async move { catalog_hosts.catalog().await });
        slow.wait_for_requests(4).await;
        let slow_hosts = hosts.clone();
        let same_alias =
            tokio::spawn(async move { slow_hosts.execute(&slow_name, json!({})).await });
        let other_hosts = hosts.clone();
        let other_alias =
            tokio::spawn(async move { other_hosts.execute(&independent_name, json!({})).await });

        let McpExecution::Success(other) =
            tokio::time::timeout(std::time::Duration::from_secs(1), other_alias)
                .await
                .expect("independent alias was blocked by another alias")
                .unwrap()
                .unwrap()
        else {
            panic!("expected independent MCP success");
        };
        assert_eq!(other["content"][0]["text"], "other done");
        assert!(!same_alias.is_finished());
        assert_eq!(catalog.await.unwrap().unwrap().tools.len(), 3);
        let McpExecution::Success(slow_result) = same_alias.await.unwrap().unwrap() else {
            panic!("expected serialized MCP success");
        };
        assert_eq!(slow_result["content"][0]["text"], "slow done");
        hosts.shutdown().await.unwrap();
        let slow_conversations = slow.conversations();
        let slow_methods: Vec<_> = slow_conversations[0]
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            slow_methods,
            [
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/list",
                "tools/list",
                "tools/call"
            ]
        );
    }

    #[tokio::test]
    async fn repeated_shutdown_waits_for_pending_client_cleanup() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}})),
            Step::Eof,
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        hosts.catalog().await.unwrap();

        let active = hosts.clients["stdio"].state.lock().await;
        let first_hosts = hosts.clone();
        let first = tokio::spawn(async move { first_hosts.shutdown().await });
        while !hosts.admission.is_closed() {
            tokio::task::yield_now().await;
        }
        let second_hosts = hosts.clone();
        let second = tokio::spawn(async move { second_hosts.shutdown().await });
        tokio::task::yield_now().await;
        assert!(!first.is_finished());
        assert!(!second.is_finished());
        drop(active);

        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert!(hosts.catalog().await.is_err());
        script.assert_completed(1);
    }

    #[tokio::test]
    async fn shutdown_interrupts_active_call_and_rejects_later_admission() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("slow")]}})),
            Step::Read,
            Step::Sleep(5_000),
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        let name = hosts.specs().await.unwrap()[0].name.clone();
        let call_hosts = hosts.clone();
        let call_name = name.clone();
        let call = tokio::spawn(async move { call_hosts.execute(&call_name, json!({})).await });
        script.wait_for_requests(4).await;

        tokio::time::timeout(std::time::Duration::from_secs(8), hosts.shutdown())
            .await
            .expect("active MCP shutdown exceeded the owned cleanup bound")
            .unwrap();
        let failure = call.await.unwrap().unwrap_err();
        assert_eq!(failure.kind, ToolFailureKind::McpCall);
        assert!(hosts.catalog().await.is_err());
        assert_eq!(
            hosts.execute(&name, json!({})).await.unwrap_err().kind,
            ToolFailureKind::McpRoute
        );
        assert_eq!(script.conversations()[0].len(), 4);
    }

    #[tokio::test]
    async fn shutdown_closes_admission_before_waiting_catalog_can_start() {
        let script = StdioFixture::new([Step::Read]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        let held = hosts.clients["stdio"].state.lock().await;
        let catalog_hosts = hosts.clone();
        let catalog = tokio::spawn(async move { catalog_hosts.catalog().await });
        tokio::task::yield_now().await;
        let shutdown_hosts = hosts.clone();
        let shutdown = tokio::spawn(async move { shutdown_hosts.shutdown().await });
        while !hosts.admission.is_closed() {
            tokio::task::yield_now().await;
        }
        drop(held);

        assert!(catalog.await.unwrap().is_err());
        shutdown.await.unwrap().unwrap();
        assert!(script.conversations().is_empty());
    }

    #[tokio::test]
    async fn cancelled_pending_startup_is_cleaned_before_explicit_recovery_spawns() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}})),
            Step::Eof,
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        hosts.admission.pause_launch.store(true, Ordering::Release);
        let launched = hosts.admission.launched.notified();
        tokio::pin!(launched);
        let catalog_hosts = hosts.clone();
        let pending = tokio::spawn(async move { catalog_hosts.catalog().await });
        launched.await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while script.conversations() != vec![Vec::<Value>::new()] {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled MCP peer did not create its empty transcript");
        pending.abort();
        match pending.await {
            Err(error) => assert!(error.is_cancelled()),
            Ok(_) => panic!("pending catalog was not cancelled"),
        }
        hosts.admission.resume_launch.notify_waiters();

        let recovered = hosts.catalog().await.unwrap();
        assert!(recovered.statuses[0].available());
        hosts.shutdown().await.unwrap();
        let conversations = script.conversations();
        assert_eq!(conversations.len(), 2);
        let mut lengths = conversations.iter().map(Vec::len).collect::<Vec<_>>();
        lengths.sort_unstable();
        assert_eq!(lengths, [0, 3]);
        assert_eq!(conversations.iter().map(Vec::len).sum::<usize>(), 3);
    }

    #[tokio::test]
    async fn shutdown_waits_for_admitted_startup_cleanup_before_success() {
        let script = StdioFixture::new([Step::Read]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        hosts.admission.pause_launch.store(true, Ordering::Release);
        let launched = hosts.admission.launched.notified();
        tokio::pin!(launched);
        let catalog_hosts = hosts.clone();
        let catalog = tokio::spawn(async move { catalog_hosts.catalog().await });
        launched.await;
        let shutdown_hosts = hosts.clone();
        let shutdown = tokio::spawn(async move { shutdown_hosts.shutdown().await });
        while !hosts.admission.is_closed() {
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;
        assert!(!shutdown.is_finished());
        hosts.admission.resume_launch.notify_waiters();

        let catalog = catalog.await.unwrap().unwrap();
        assert!(!catalog.statuses[0].available());
        shutdown.await.unwrap().unwrap();
        assert!(hosts.catalog().await.is_err());
        assert!(script.conversations().iter().all(Vec::is_empty));
    }

    #[tokio::test]
    async fn post_spawn_setup_failure_retains_owner_until_cleanup_is_confirmed() {
        let script = StdioFixture::new([Step::Read]);
        let hosts = McpHosts::new(
            Path::new("."),
            &BTreeMap::from([(
                "stdio".into(),
                McpConfig {
                    command: Some(script.command().into()),
                    ..Default::default()
                },
            )]),
        )
        .unwrap();
        hosts.admission.fail_setup.store(true, Ordering::Release);

        let catalog = hosts.catalog().await.unwrap();
        assert!(!catalog.statuses[0].available());
        assert!(
            hosts.clients["stdio"]
                .state
                .lock()
                .await
                .transport
                .is_none()
        );
        hosts.shutdown().await.unwrap();
        assert!(script.conversations().iter().all(Vec::is_empty));
    }

    #[test]
    fn sse_parses_fragmented_crlf_multiline_and_bounds_malformed_streams() {
        let mut parser = Sse::default();
        assert!(
            parser
                .push(b": hello\r\ndata: {\r\ndata: \"id\": 1,\r\ndata: \"result\":")
                .unwrap()
                .is_empty()
        );
        let values = parser.push(b" {}\r\ndata: }\r\n\r\n").unwrap();
        assert_eq!(
            values,
            json!([{ "id":1,"result":{} }]).as_array().unwrap().clone()
        );
        assert!(Sse::default().push(b"data: invalid\n\n").is_err());
        assert!(Sse::default().push(&[255, b'\n']).is_err());
        assert!(Sse::default().push(&vec![b'a'; MAX_BYTES + 1]).is_err());
    }

    #[tokio::test]
    async fn http_sse_answers_ping_rejects_unnegotiated_requests_and_ignores_notifications() {
        let mut stream = Reply::sse(json!({"tools":[]}));
        stream.body = format!(
            "data: {{\"method\":\"notice\"}}\n\ndata: {{\"id\":900,\"method\":\"ping\"}}\n\ndata: {{\"id\":901,\"method\":\"sampling/createMessage\"}}\n\n{}",
            stream.body
        );
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            stream,
            Reply::json(json!({})),
            Reply::json(json!({})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        assert!(hosts.specs().await.unwrap().is_empty());
        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(
            requests[3].body,
            json!({"jsonrpc":"2.0","id":900,"result":{}})
        );
        assert_eq!(requests[4].body["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn http_aborted_stream_and_rejected_delete_surface_failures() {
        let mut stream = Reply::sse(json!({}));
        stream.body = ": ended without response\n\n".into();
        let mut failed_delete = Reply::json(json!({}));
        failed_delete.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            stream,
            failed_delete,
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        hosts.shutdown().await.unwrap();
        let mut failed_delete = Reply::json(json!({}));
        failed_delete.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[]})),
            failed_delete,
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        hosts.specs().await.unwrap();
        assert!(
            hosts
                .shutdown()
                .await
                .unwrap_err()
                .to_string()
                .contains("DELETE failed")
        );
        hosts.shutdown().await.unwrap();
    }
}
