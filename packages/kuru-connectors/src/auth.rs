//! Kuru-owned OpenAI credentials. No other application's store is inspected.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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
            self.refresh_rejected(&credentials).await
        } else {
            Ok(credentials)
        }
    }

    pub(crate) async fn refresh_rejected(
        &self,
        observed: &RequestCredentials,
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
        std::thread::Builder::new().name("kuru-auth-refresh".into()).spawn(move || {
            let result = (|| -> Result<_> {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
                    .context("create authentication refresh runtime")?;
                runtime.block_on(async {
                    tokio::time::timeout(manager.inner.http_timeout, manager.refresh_owned(observed, &sender))
                        .await.context("ChatGPT refresh deadline exceeded; run kuru login if rotation is uncertain")?
                })
            })();
            let _ = sender.send(result);
        }).context("start authentication refresh worker")?;
        tokio::time::timeout(self.inner.http_timeout, receiver)
            .await
            .context("ChatGPT refresh deadline exceeded; completion remains owned")?
            .context("authentication refresh worker stopped")?
    }

    async fn refresh_owned(
        &self,
        observed: RequestCredentials,
        sender: &tokio::sync::oneshot::Sender<Result<RequestCredentials>>,
    ) -> Result<RequestCredentials> {
        let lease = self
            .inner
            .store
            .lease(false, self.inner.http_timeout)
            .await?
            .context("ChatGPT session was removed; run kuru login")?;
        if sender.is_closed() {
            bail!("authentication refresh was cancelled before dispatch");
        }
        let mut record = lease
            .read()?
            .context("ChatGPT session was removed; run kuru login")?;
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
        session.refresh_pending = true;
        let previous = session.clone();
        record.revision = random(32)?;
        lease.write(&record)?;
        let response = http::refresh(self, &previous.refresh_token).await?;
        let mut refreshed = response.session(Some(&previous))?;
        ensure!(
            refreshed.account_id == previous.account_id,
            "ChatGPT refresh changed the account; run kuru login again"
        );
        refreshed.session_id = previous.session_id;
        refreshed.generation = previous
            .generation
            .checked_add(1)
            .context("authentication generation exhausted")?;
        let credentials = refreshed.credentials();
        lease.write(&store::Record::new(Some(refreshed))?)?;
        Ok(credentials)
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
