use super::{AuthManager, CLIENT_ID, now, random, store::Session, validate_secret};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Default, Deserialize)]
pub(super) struct Tokens {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_in: Option<u64>,
}
impl Tokens {
    pub fn session(self, previous: Option<&Session>) -> Result<Session> {
        let access_token = self
            .access_token
            .context("token response lacks an access token")?;
        let refresh_token = self
            .refresh_token
            .or_else(|| previous.map(|session| session.refresh_token.clone()))
            .context("token response lacks a refresh token")?;
        let id_token = self
            .id_token
            .or_else(|| previous.map(|session| session.id_token.clone()))
            .context("token response lacks an identity token")?;
        for token in [&access_token, &refresh_token, &id_token] {
            validate_secret(token)?;
        }
        let access = claims(&access_token)?;
        let identity = claims(&id_token)?;
        let account = |value: &Value| {
            value
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_account_id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let access_account = account(&access);
        let id_account = account(&identity);
        if let (Some(access), Some(identity)) = (&access_account, &id_account) {
            ensure!(
                access == identity,
                "token response account identities disagree"
            );
        }
        let account_id = access_account
            .or(id_account)
            .context("token response lacks a ChatGPT account identity")?;
        ensure!(
            !account_id.is_empty()
                && account_id.len() <= 1024
                && !account_id.chars().any(char::is_control),
            "invalid ChatGPT account identity"
        );
        let now = now()?;
        let expires_at = access
            .get("exp")
            .and_then(Value::as_u64)
            .or_else(|| self.expires_in.and_then(|seconds| now.checked_add(seconds)))
            .context("token response lacks a valid expiry")?;
        ensure!(expires_at > now, "token response is already expired");
        Ok(Session {
            access_token,
            refresh_token,
            id_token,
            account_id,
            expires_at,
            session_id: random(32)?,
            generation: 0,
            refresh_pending: false,
        })
    }
}

fn claims(token: &str) -> Result<Value> {
    let parts: Vec<_> = token.split('.').collect();
    ensure!(
        parts.len() == 3 && parts.iter().all(|part| !part.is_empty()),
        "invalid token claim encoding"
    );
    let bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| anyhow::anyhow!("invalid token claim encoding"))?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid token claims"))
}

pub(super) fn client(manager: &AuthManager) -> Result<Client> {
    Client::builder()
        .timeout(manager.inner.http_timeout)
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Kuru/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("create authentication HTTP client")
}

pub(super) async fn json_response(mut response: Response) -> Result<Value> {
    let status = response.status();
    ensure!(
        response.content_length().unwrap_or(0) <= crate::MAX_BYTES as u64,
        "authentication response exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("authentication response read failed"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= crate::MAX_BYTES,
            "authentication response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    // Remote descriptions and parser errors can contain echoed grant material.
    ensure!(
        status.is_success(),
        "OpenAI authentication returned HTTP {}; run kuru login if the grant was rejected",
        status.as_u16()
    );
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid authentication JSON response"))
}

pub(super) async fn post_json(manager: &AuthManager, path: &str, body: &Value) -> Result<Response> {
    client(manager)?
        .post(format!("{}{path}", manager.inner.issuer))
        .header("originator", "kuru")
        .json(body)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("OpenAI authentication request failed"))
}

pub(super) async fn exchange(
    manager: &AuthManager,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Result<Tokens> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("code", code)
        .append_pair("redirect_uri", redirect)
        .append_pair("code_verifier", verifier)
        .finish();
    let response = client(manager)?
        .post(format!("{}/oauth/token", manager.inner.issuer))
        .header("originator", "kuru")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("OpenAI token exchange request failed"))?;
    serde_json::from_value(json_response(response).await?)
        .map_err(|_| anyhow::anyhow!("invalid token response fields"))
}

pub(super) async fn refresh(manager: &AuthManager, refresh_token: &str) -> Result<Tokens> {
    let response = post_json(
        manager,
        "/oauth/token",
        &json!({"grant_type":"refresh_token", "client_id":CLIENT_ID,"refresh_token":refresh_token}),
    )
    .await?;
    serde_json::from_value(json_response(response).await?)
        .map_err(|_| anyhow::anyhow!("invalid refresh response fields"))
}

pub(super) fn pending(status: StatusCode) -> bool {
    matches!(status.as_u16(), 403 | 404)
}
