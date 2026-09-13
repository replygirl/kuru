//! Kuru-owned OpenAI credentials. No other application's store is inspected.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::Instrument;

use crate::retry::{OperationBudget, RefreshAllowance};

mod http;
mod login;
mod store;
#[cfg(test)]
mod tests;

pub use login::{BrowserLogin, DeviceLogin};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ISSUER: &str = "https://auth.openai.com";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const REFRESH_SKEW: u64 = 300;

/// Deliberately contains only user-facing authentication metadata.
#[derive(Clone, Debug, Serialize)]
pub struct AuthStatus {
    pub authenticated: bool,
    pub account_id: Option<String>,
    pub expires_at: Option<u64>,
    pub api_key_available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthRoute {
    #[cfg(test)]
    ApiKey,
    Chatgpt,
}

/// Secret-bearing snapshots deliberately do not implement Debug or Serialize.
#[derive(Clone)]
pub(crate) struct RequestCredentials {
    route: AuthRoute,
    bearer: String,
    account_id: Option<String>,
    session_id: Option<String>,
    generation: Option<u64>,
}

impl RequestCredentials {
    pub(crate) fn route(&self) -> AuthRoute {
        self.route
    }
    pub(crate) fn bearer(&self) -> &str {
        &self.bearer
    }
    pub(crate) fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    pub(crate) fn generation(&self) -> Option<u64> {
        self.generation
    }
}

#[derive(Clone)]
pub struct AuthManager {
    inner: Arc<Settings>,
}

struct Settings {
    store: store::Store,
    api_key: Option<String>,
    issuer: String,
    http_timeout: Duration,
    login_timeout: Duration,
    callback_port: u16,
    #[cfg(test)]
    fail_refresh_preparation: bool,
    #[cfg(test)]
    refresh_attempts: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    refresh_proxy: Option<String>,
    #[cfg(test)]
    refresh_gate: Option<Arc<RefreshGate>>,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RefreshPhase {
    BeforeFirst,
    BeforeSecond,
}

#[cfg(test)]
struct RefreshGate {
    phase: RefreshPhase,
    reached: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl AuthManager {
    /// Validate configuration without opening or creating any credential files.
    pub fn new(data_dir: PathBuf, tool_root: PathBuf, api_key: Option<String>) -> Result<Self> {
        let api_key = api_key.filter(|key| !key.is_empty());
        if let Some(key) = &api_key {
            validate_secret(key)?;
        }
        Ok(Self {
            inner: Arc::new(Settings {
                store: store::Store::new(data_dir, tool_root)?,
                api_key,
                issuer: ISSUER.to_owned(),
                http_timeout: crate::IO_TIMEOUT,
                login_timeout: LOGIN_TIMEOUT,
                callback_port: 1455,
                #[cfg(test)]
                fail_refresh_preparation: false,
                #[cfg(test)]
                refresh_attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                #[cfg(test)]
                refresh_proxy: None,
                #[cfg(test)]
                refresh_gate: None,
            }),
        })
    }

    pub async fn status(&self) -> Result<AuthStatus> {
        let record = self.inner.store.read()?;
        Ok(self.status_from(record.as_ref().and_then(|record| record.session.as_ref())))
    }

    fn status_from(&self, session: Option<&store::Session>) -> AuthStatus {
        AuthStatus {
            authenticated: session.is_some_and(|session| !session.refresh_pending),
            account_id: session.map(|session| session.account_id.clone()),
            expires_at: session.map(|session| session.expires_at),
            api_key_available: self.inner.api_key.is_some(),
        }
    }

    pub async fn begin_browser(&self) -> Result<BrowserLogin> {
        login::browser(self.clone()).await
    }
    pub async fn begin_device(&self) -> Result<DeviceLogin> {
        login::device(self.clone()).await
    }

    /// Remove the Kuru grant, keeping a non-secret revision tombstone and lease.
    pub async fn logout(&self) -> Result<()> {
        let lease = self
            .inner
            .store
            .lease(true, self.inner.http_timeout)
            .await?;
        if let Some(lease) = lease {
            lease.write(&store::Record::new(None)?)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn credentials(&self, route: AuthRoute) -> Result<RequestCredentials> {
        if route == AuthRoute::ApiKey {
            return Ok(RequestCredentials {
                route,
                bearer: self.inner.api_key.clone().context(
                    "API key is missing; configure OPENAI_API_KEY for the responses provider",
                )?,
                account_id: None,
                session_id: None,
                generation: None,
            });
        }
        let session = self
            .inner
            .store
            .read()?
            .and_then(|record| record.session)
            .context("ChatGPT is not authenticated; run kuru login")?;
        let credentials = session.credentials();
        if session.refresh_pending || session.expires_at <= now()?.saturating_add(REFRESH_SKEW) {
            let budget = OperationBudget::new(self.inner.http_timeout);
            let allowance = budget.begin_rotation(self.inner.http_timeout)?;
            self.refresh_with_allowance(&credentials, allowance).await
        } else {
            Ok(credentials)
        }
    }

    pub(crate) async fn credentials_snapshot(&self) -> Result<RequestCredentials> {
        self.inner
            .store
            .read()?
            .and_then(|record| record.session)
            .context("ChatGPT is not authenticated; run kuru login")
            .map(|session| session.credentials())
    }

    pub(crate) async fn resolve_for_operation(
        &self,
        observed: &RequestCredentials,
        budget: &OperationBudget,
    ) -> Result<RequestCredentials> {
        let record = self
            .inner
            .store
            .read()?
            .and_then(|record| record.session)
            .context("ChatGPT is not authenticated; run kuru login")?;
        let current = record.credentials();
        ensure!(
            current.session_id() == observed.session_id()
                && current.account_id() == observed.account_id(),
            "ChatGPT account or login session changed; start a new Kuru session"
        );
        let newer_generation = current.generation() != observed.generation();
        if newer_generation
            || record.refresh_pending
            || record.expires_at <= now()?.saturating_add(REFRESH_SKEW)
        {
            let allowance = budget.begin_rotation(self.inner.http_timeout)?;
            self.refresh_with_allowance(
                if newer_generation { observed } else { &current },
                allowance,
            )
            .await
        } else {
            Ok(current)
        }
    }

    #[cfg(test)]
    pub(crate) async fn refresh_rejected(
        &self,
        observed: &RequestCredentials,
    ) -> Result<RequestCredentials> {
        let budget = OperationBudget::new(self.inner.http_timeout);
        let allowance = budget.begin_rotation(self.inner.http_timeout)?;
        self.refresh_with_allowance(observed, allowance).await
    }

    pub(crate) async fn refresh_with_allowance(
        &self,
        observed: &RequestCredentials,
        allowance: RefreshAllowance,
    ) -> Result<RequestCredentials> {
        ensure!(
            observed.route == AuthRoute::Chatgpt,
            "API-key requests cannot refresh a ChatGPT session"
        );
        let manager = self.clone();
        let observed = observed.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // A refresh can rotate its token after the caller disappears. The same
        // bounded worker owns its reactor, lease, response and durable write.
        let deadline = allowance.deadline();
        let span = tracing::Span::current();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        std::thread::Builder::new()
            .name("kuru-auth-refresh".into())
            .spawn(move || {
                let mut sender = sender;
                let result = (|| -> Result<_> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("create authentication refresh runtime")?;
                    tracing::dispatcher::with_default(&dispatch, || {
                        runtime.block_on(
                            async {
                                manager
                                    .refresh_owned(observed, allowance, &mut sender)
                                    .await
                            }
                            .instrument(span),
                        )
                    })
                })();
                let _ = sender.send(result);
            })
            .context("start authentication refresh worker")?;
        let remaining = allowance_wait(self.inner.http_timeout, deadline);
        tokio::time::timeout(remaining, receiver)
            .await
            .context("ChatGPT refresh deadline exceeded; completion remains owned")?
            .context("authentication refresh worker stopped")?
    }

    async fn refresh_owned(
        &self,
        observed: RequestCredentials,
        allowance: RefreshAllowance,
        sender: &mut tokio::sync::oneshot::Sender<Result<RequestCredentials>>,
    ) -> Result<RequestCredentials> {
        let started = Instant::now();
        let lease = self
            .inner
            .store
            .lease(
                false,
                allowance_wait(self.inner.http_timeout, allowance.deadline()),
            )
            .await?
            .context("ChatGPT session was removed; run kuru login")?;
        allowance.check_dispatch(sender.is_closed())?;
        let mut record = lease
            .read()?
            .context("ChatGPT session was removed; run kuru login")?;
        let original = record.clone();
        let session = record
            .session
            .as_mut()
            .context("ChatGPT was logged out; run kuru login")?;
        ensure!(
            Some(session.session_id.as_str()) == observed.session_id()
                && Some(session.account_id.as_str()) == observed.account_id(),
            "ChatGPT session changed; start a new conversation"
        );
        ensure!(
            !session.refresh_pending,
            "ChatGPT refresh was interrupted or rejected; run kuru login again"
        );
        if Some(session.generation) != observed.generation() {
            ensure!(
                Some(session.generation) > observed.generation(),
                "ChatGPT credential generation moved backwards"
            );
            return Ok(session.credentials());
        }
        let previous = session.clone();
        let prepared = http::PreparedRefresh::new(self, &previous.refresh_token)?;
        session.refresh_pending = true;
        record.revision = random(32)?;
        let pending = record.clone();
        let pending_write = lease.write(&pending);
        let installed = lease.read().map_err(|_| {
            anyhow::anyhow!("authentication pending publication is uncertain; run kuru login")
        })?;
        if installed.as_ref() != Some(&pending) {
            if installed.as_ref() == Some(&original) {
                pending_write?;
                bail!("authentication pending publication failed")
            }
            bail!("authentication pending publication is uncertain; run kuru login")
        }
        if let Err(error) = allowance.check_dispatch(sender.is_closed()) {
            rollback_refresh(&lease, &pending, &previous)?;
            return Err(error);
        }
        #[cfg(test)]
        if let Err(error) =
            wait_refresh_gate(self, RefreshPhase::BeforeFirst, sender, &allowance).await
        {
            rollback_refresh(&lease, &pending, &previous)?;
            return Err(error);
        }
        allowance.check_dispatch(sender.is_closed()).map_err(|error| {
            if rollback_refresh(&lease, &pending, &previous).is_err() {
                anyhow::anyhow!(
                    "authentication refresh stopped before dispatch and rollback state is unknown; run kuru login"
                )
            } else {
                error
            }
        })?;
        if let Err(error) = allowance.take_refresh_send(true) {
            rollback_refresh(&lease, &pending, &previous)?;
            return Err(error);
        }
        let response = match prepared.send(allowance.deadline()).await {
            Ok(response) => response,
            Err(http::RefreshFailure::NotDispatched) if allowance.can_repeat_refresh() => {
                match allowance.retry_delay() {
                    crate::retry::RetryDecision::Delay(delay) => {
                        tracing::info!(target: "kuru.provider", operation = "chatgpt-refresh", status = "not-dispatched", delay_ms = delay.as_millis() as u64, elapsed_ms = started.elapsed().as_millis() as u64, "refresh retry scheduled");
                        if let Err(error) = wait_refresh_backoff(sender, &allowance, delay).await {
                            if rollback_refresh(&lease, &pending, &previous).is_err() {
                                bail!(
                                    "authentication refresh stopped before dispatch and rollback state is unknown; run kuru login"
                                )
                            }
                            return Err(error);
                        }
                    }
                    crate::retry::RetryDecision::Exhausted => {
                        tracing::info!(target: "kuru.provider", operation = "chatgpt-refresh", status = "retry-exhausted", elapsed_ms = started.elapsed().as_millis() as u64, "refresh retry exhausted");
                        rollback_refresh(&lease, &pending, &previous)?;
                        bail!("ChatGPT refresh retry budget exhausted")
                    }
                }
                if let Err(error) = allowance.check_dispatch(sender.is_closed()) {
                    rollback_refresh(&lease, &pending, &previous)?;
                    return Err(error);
                }
                #[cfg(test)]
                if let Err(error) =
                    wait_refresh_gate(self, RefreshPhase::BeforeSecond, sender, &allowance).await
                {
                    rollback_refresh(&lease, &pending, &previous)?;
                    return Err(error);
                }
                allowance.check_dispatch(sender.is_closed()).map_err(|error| {
                    if rollback_refresh(&lease, &pending, &previous).is_err() {
                        anyhow::anyhow!(
                            "authentication refresh stopped before dispatch and rollback state is unknown; run kuru login"
                        )
                    } else {
                        error
                    }
                })?;
                if let Err(error) = allowance.take_refresh_send(true) {
                    rollback_refresh(&lease, &pending, &previous)?;
                    return Err(error);
                }
                match prepared.send(allowance.deadline()).await {
                    Ok(response) => response,
                    Err(http::RefreshFailure::NotDispatched) => {
                        tracing::info!(target: "kuru.provider", operation = "chatgpt-refresh", status = "not-dispatched-terminal", elapsed_ms = started.elapsed().as_millis() as u64, "refresh retry terminal");
                        rollback_refresh(&lease, &pending, &previous)?;
                        bail!("OpenAI authentication connection failed before token dispatch")
                    }
                    Err(http::RefreshFailure::PossiblyDispatched) => {
                        ensure_pending(&lease, &pending)?;
                        bail!("OpenAI authentication request outcome is uncertain; run kuru login")
                    }
                }
            }
            Err(http::RefreshFailure::NotDispatched) => {
                rollback_refresh(&lease, &pending, &previous)?;
                bail!("OpenAI authentication connection failed before token dispatch")
            }
            Err(http::RefreshFailure::PossiblyDispatched) => {
                ensure_pending(&lease, &pending)?;
                bail!("OpenAI authentication request outcome is uncertain; run kuru login")
            }
        };
        let Some(response_time) = allowance
            .deadline()
            .checked_duration_since(std::time::Instant::now())
        else {
            ensure_pending(&lease, &pending)?;
            bail!("OpenAI authentication request outcome is uncertain; run kuru login")
        };
        let response =
            match tokio::time::timeout(response_time, http::json_response(response)).await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    ensure_pending(&lease, &pending)?;
                    return Err(error);
                }
                Err(_) => {
                    ensure_pending(&lease, &pending)?;
                    bail!("OpenAI authentication request outcome is uncertain; run kuru login")
                }
            };
        let response: http::Tokens = match serde_json::from_value(response) {
            Ok(response) => response,
            Err(_) => {
                ensure_pending(&lease, &pending)?;
                bail!("invalid refresh response fields")
            }
        };
        let mut refreshed = match response.session(Some(&previous)) {
            Ok(refreshed) => refreshed,
            Err(error) => {
                ensure_pending(&lease, &pending)?;
                return Err(error);
            }
        };
        if refreshed.account_id != previous.account_id {
            ensure_pending(&lease, &pending)?;
            bail!("ChatGPT refresh changed the account; run kuru login again")
        }
        refreshed.session_id = previous.session_id;
        refreshed.generation = match previous.generation.checked_add(1) {
            Some(generation) => generation,
            None => {
                ensure_pending(&lease, &pending)?;
                bail!("authentication generation exhausted")
            }
        };
        let credentials = refreshed.credentials();
        let published = match store::Record::new(Some(refreshed)) {
            Ok(published) => published,
            Err(error) => {
                ensure_pending(&lease, &pending)?;
                return Err(error);
            }
        };
        let write = lease.write(&published);
        let observed_record = lease.read().map_err(|_| {
            anyhow::anyhow!("authentication rotation state is unknown; run kuru login")
        })?;
        if observed_record.as_ref() == Some(&published) {
            return Ok(credentials);
        }
        if let Err(error) = write {
            return Err(error)
                .context("authentication rotation publication is uncertain; run kuru login");
        }
        bail!("authentication rotation publication is uncertain; run kuru login")
    }

    async fn activate(
        &self,
        response: http::Tokens,
        expected_revision: Option<String>,
    ) -> Result<AuthStatus> {
        let session = response.session(None)?;
        let lease = self
            .inner
            .store
            .lease(true, self.inner.http_timeout)
            .await?
            .context("create private authentication lease")?;
        let current = lease.read()?;
        ensure!(
            current.as_ref().map(|record| &record.revision) == expected_revision.as_ref(),
            "authentication changed while login was pending; start login again"
        );
        let status = self.status_from(Some(&session));
        lease.write(&store::Record::new(Some(session))?)?;
        Ok(status)
    }

    #[cfg(test)]
    pub(crate) fn test_issuer(data_dir: PathBuf, tool_root: PathBuf, issuer: &str) -> Result<Self> {
        let url = url::Url::parse(issuer)?;
        ensure!(
            url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
            "test issuer must be loopback"
        );
        let mut manager = Self::new(data_dir, tool_root, None)?;
        let settings = Arc::get_mut(&mut manager.inner).unwrap();
        settings.issuer = issuer.trim_end_matches('/').into();
        settings.callback_port = 0;
        Ok(manager)
    }

    #[cfg(test)]
    pub(crate) async fn seed_test_session(
        &self,
        access_token: &str,
        refresh_token: &str,
        account_id: &str,
    ) -> Result<()> {
        validate_secret(access_token)?;
        validate_secret(refresh_token)?;
        let session = store::Session {
            access_token: access_token.into(),
            refresh_token: refresh_token.into(),
            id_token: tests::jwt(account_id, now()? + 3600),
            account_id: account_id.into(),
            expires_at: now()? + 3600,
            session_id: random(32)?,
            generation: 0,
            refresh_pending: false,
        };
        let lease = self
            .inner
            .store
            .lease(true, self.inner.http_timeout)
            .await?
            .unwrap();
        lease.write(&store::Record::new(Some(session))?)
    }

    #[cfg(test)]
    fn fail_refresh_preparation(&mut self) {
        Arc::get_mut(&mut self.inner)
            .expect("test manager must be uniquely owned")
            .fail_refresh_preparation = true;
    }

    #[cfg(test)]
    fn refresh_attempts(&self) -> usize {
        self.inner
            .refresh_attempts
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    fn use_refresh_proxy(&mut self, issuer: &str, proxy: String) {
        let settings = Arc::get_mut(&mut self.inner).expect("test manager must be uniquely owned");
        settings.issuer = issuer.into();
        settings.refresh_proxy = Some(proxy);
    }

    #[cfg(test)]
    fn pause_refresh(&mut self, phase: RefreshPhase) -> Arc<RefreshGate> {
        let gate = Arc::new(RefreshGate {
            phase,
            reached: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        Arc::get_mut(&mut self.inner)
            .expect("test manager must be uniquely owned")
            .refresh_gate = Some(gate.clone());
        gate
    }
}

fn allowance_wait(limit: Duration, deadline: std::time::Instant) -> Duration {
    deadline
        .checked_duration_since(std::time::Instant::now())
        .unwrap_or(Duration::ZERO)
        .min(limit)
}

fn rollback_refresh(
    lease: &store::Lease,
    pending: &store::Record,
    previous: &store::Session,
) -> Result<()> {
    let current = lease
        .read()
        .map_err(|_| anyhow::anyhow!("authentication rollback state is unknown; run kuru login"))?;
    ensure!(
        current.as_ref() == Some(pending),
        "authentication rollback state is unknown; run kuru login"
    );
    let rollback = store::Record::new(Some(previous.clone()))
        .map_err(|_| anyhow::anyhow!("authentication rollback state is unknown; run kuru login"))?;
    let write = lease.write(&rollback);
    let observed = lease
        .read()
        .map_err(|_| anyhow::anyhow!("authentication rollback state is unknown; run kuru login"))?;
    if observed.as_ref() == Some(&rollback) {
        return Ok(());
    }
    if let Err(error) = write {
        return Err(error).context("authentication rollback state is unknown; run kuru login");
    }
    bail!("authentication rollback state is unknown; run kuru login")
}

fn ensure_pending(lease: &store::Lease, pending: &store::Record) -> Result<()> {
    ensure!(
        lease
            .read()
            .map_err(|_| anyhow::anyhow!(
                "authentication refresh state is unknown; run kuru login"
            ))?
            .as_ref()
            == Some(pending),
        "authentication refresh state is unknown; run kuru login"
    );
    Ok(())
}

async fn wait_refresh_backoff(
    sender: &mut tokio::sync::oneshot::Sender<Result<RequestCredentials>>,
    allowance: &RefreshAllowance,
    delay: Duration,
) -> Result<()> {
    let Some(remaining) = allowance
        .deadline()
        .checked_duration_since(std::time::Instant::now())
    else {
        bail!("authentication refresh was cancelled before dispatch")
    };
    tokio::select! {
        _ = tokio::time::sleep(delay) => Ok(()),
        _ = tokio::time::sleep(remaining) => {
            bail!("authentication refresh was cancelled before dispatch")
        }
        _ = sender.closed() => {
            bail!("authentication refresh was cancelled before dispatch")
        }
    }
}

#[cfg(test)]
async fn wait_refresh_gate(
    manager: &AuthManager,
    phase: RefreshPhase,
    sender: &mut tokio::sync::oneshot::Sender<Result<RequestCredentials>>,
    allowance: &RefreshAllowance,
) -> Result<()> {
    let Some(gate) = manager
        .inner
        .refresh_gate
        .as_ref()
        .filter(|gate| gate.phase == phase)
    else {
        return Ok(());
    };
    gate.reached.notify_one();
    let Some(remaining) = allowance
        .deadline()
        .checked_duration_since(std::time::Instant::now())
    else {
        bail!("authentication refresh was cancelled before dispatch")
    };
    tokio::select! {
        _ = gate.release.notified() => Ok(()),
        _ = tokio::time::sleep(remaining) => {
            bail!("authentication refresh was cancelled before dispatch")
        }
        _ = sender.closed() => {
            bail!("authentication refresh was cancelled before dispatch")
        }
    }
}

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs())
}
fn random(bytes: usize) -> Result<String> {
    let mut value = vec![0; bytes];
    getrandom::fill(&mut value)
        .map_err(|_| anyhow::anyhow!("operating system random source failed"))?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}
fn validate_secret(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64 * 1024
            && reqwest::header::HeaderValue::from_str(value).is_ok(),
        "invalid authentication token"
    );
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_future_jwt(account_id: &str) -> String {
    tests::jwt(account_id, now().unwrap() + 3600)
}
