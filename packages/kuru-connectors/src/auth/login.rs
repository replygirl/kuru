use super::{AuthManager, AuthStatus, CLIENT_ID, http, random};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::Instant,
};

const CALLBACK_BYTES: usize = 8192;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Owns the callback socket. Dropping this handle or `finish` closes it.
/// PKCE verifier and state intentionally have no Debug/serialization surface.
pub struct BrowserLogin {
    manager: AuthManager,
    listener: TcpListener,
    url: String,
    redirect: String,
    state: String,
    verifier: String,
    revision: Option<String>,
    deadline: Instant,
}
impl BrowserLogin {
    pub fn authorization_url(&self) -> &str {
        &self.url
    }
    pub async fn finish(self) -> Result<AuthStatus> {
        tokio::time::timeout_at(self.deadline, self.run())
            .await
            .context("ChatGPT browser login expired; run kuru login again")?
    }
    async fn run(self) -> Result<AuthStatus> {
        loop {
            let (mut socket, peer) = self
                .listener
                .accept()
                .await
                .context("accept local authentication callback")?;
            ensure!(
                peer.ip().is_loopback(),
                "authentication callback peer is not loopback"
            );
            let callback = tokio::time::timeout(
                CALLBACK_TIMEOUT,
                read_callback(&mut socket, self.listener.local_addr()?.port(), &self.state),
            )
            .await;
            let code = match callback {
                Ok(Ok(Callback::Code(code))) => code,
                Ok(Ok(Callback::Denied)) => {
                    reply(&mut socket, 400, "Login was declined. Return to Kuru.").await;
                    bail!(
                        "ChatGPT authorization was declined; existing credentials were preserved"
                    );
                }
                _ => {
                    reply(
                        &mut socket,
                        400,
                        "Invalid login callback. Return to the original login tab.",
                    )
                    .await;
                    continue;
                }
            };
            let tokens =
                match http::exchange(&self.manager, &code, &self.verifier, &self.redirect).await {
                    Ok(tokens) => tokens,
                    Err(error) => {
                        reply(
                            &mut socket,
                            400,
                            "Login could not complete. See Kuru for the next step.",
                        )
                        .await;
                        return Err(error);
                    }
                };
            // Finish callback I/O before publication. No await follows the
            // checked synchronous commit inside activate.
            reply(
                &mut socket,
                200,
                "Authorization received; return to Kuru for the result.",
            )
            .await;
            return self.manager.activate(tokens, self.revision).await;
        }
    }
}

pub(super) async fn browser(manager: AuthManager) -> Result<BrowserLogin> {
    let revision = manager.inner.store.read()?.map(|record| record.revision);
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, manager.inner.callback_port)).await
        .context("cannot bind the local ChatGPT callback; close your other login or use kuru login --device")?;
    let redirect = format!(
        "http://localhost:{}/auth/callback",
        listener.local_addr()?.port()
    );
    let verifier = random(64)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random(32)?;
    let mut url = url::Url::parse(&format!("{}/oauth/authorize", manager.inner.issuer))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", &redirect)
        .append_pair("scope", "openid profile email offline_access")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", "kuru");
    let deadline = Instant::now() + manager.inner.login_timeout;
    Ok(BrowserLogin {
        manager,
        listener,
        url: url.into(),
        redirect,
        state,
        verifier,
        revision,
        deadline,
    })
}

enum Callback {
    Code(String),
    Denied,
}
async fn read_callback(
    socket: &mut TcpStream,
    port: u16,
    expected_state: &str,
) -> Result<Callback> {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        let count = socket.read(&mut chunk).await?;
        ensure!(count > 0, "incomplete callback request");
        ensure!(
            bytes.len() + count <= CALLBACK_BYTES,
            "callback request exceeds size limit"
        );
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            ensure!(
                end + 4 == bytes.len(),
                "callback request must not contain a body"
            );
            break;
        }
    }
    let request = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid callback request encoding"))?;
    let mut lines = request.split("\r\n");
    let first: Vec<_> = lines.next().unwrap_or_default().split(' ').collect();
    ensure!(
        first.len() == 3 && first[0] == "GET" && first[2] == "HTTP/1.1",
        "invalid callback method or protocol"
    );
    ensure!(
        first[1].starts_with('/') && !first[1].starts_with("//") && !first[1].contains('#'),
        "invalid callback target"
    );
    let mut headers = HashMap::new();
    for line in lines.take_while(|line| !line.is_empty()) {
        let (key, value) = line.split_once(':').context("invalid callback header")?;
        ensure!(
            !key.is_empty() && !key.chars().any(char::is_whitespace),
            "invalid callback header name"
        );
        ensure!(
            headers
                .insert(key.to_ascii_lowercase(), value.trim())
                .is_none(),
            "duplicate callback header"
        );
    }
    ensure!(
        headers.get("host") == Some(&format!("localhost:{port}").as_str()),
        "invalid callback host"
    );
    ensure!(
        !headers.contains_key("transfer-encoding")
            && headers
                .get("content-length")
                .is_none_or(|length| *length == "0"),
        "callback body is unsupported"
    );
    let url = url::Url::parse(&format!("http://localhost{target}", target = first[1]))?;
    ensure!(url.path() == "/auth/callback", "invalid callback path");
    let mut seen = HashSet::new();
    let mut values = HashMap::new();
    for (key, value) in url.query_pairs() {
        ensure!(
            seen.insert(key.clone().into_owned()),
            "duplicate callback value"
        );
        values.insert(key.into_owned(), value.into_owned());
    }
    ensure!(
        values.get("state").map(String::as_str) == Some(expected_state),
        "callback state mismatch"
    );
    if values.contains_key("error") {
        ensure!(!values.contains_key("code"), "ambiguous callback result");
        return Ok(Callback::Denied);
    }
    let code = values
        .remove("code")
        .context("callback lacks authorization code")?;
    ensure!(
        !code.is_empty() && code.len() <= 4096,
        "invalid callback authorization code"
    );
    Ok(Callback::Code(code))
}

async fn reply(socket: &mut TcpStream, status: u16, text: &str) {
    let response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{text}",
        if status == 200 { "OK" } else { "Bad Request" },
        text.len()
    );
    let _ = tokio::time::timeout(CALLBACK_TIMEOUT, socket.write_all(response.as_bytes())).await;
}

/// User-code metadata is intended for display; refresh/access tokens are not.
pub struct DeviceLogin {
    manager: AuthManager,
    verification_url: String,
    user_code: String,
    device_auth_id: String,
    interval: Duration,
    revision: Option<String>,
    deadline: Instant,
}
impl DeviceLogin {
    pub fn verification_url(&self) -> &str {
        &self.verification_url
    }
    pub fn user_code(&self) -> &str {
        &self.user_code
    }
    pub async fn finish(self) -> Result<AuthStatus> {
        tokio::time::timeout_at(self.deadline, self.run())
            .await
            .context("ChatGPT device login expired; run kuru login again")?
    }
    async fn run(self) -> Result<AuthStatus> {
        loop {
            tokio::time::sleep(self.interval).await;
            let response = http::post_json(
                &self.manager,
                "/api/accounts/deviceauth/token",
                &json!({"device_auth_id":self.device_auth_id,"user_code":self.user_code}),
            )
            .await?;
            if http::pending(response.status()) {
                drop(response);
                continue;
            }
            let value = http::json_response(response).await?;
            let code = nonempty(&value, "authorization_code")?;
            let verifier = nonempty(&value, "code_verifier")?;
            let redirect = format!("{}/deviceauth/callback", self.manager.inner.issuer);
            let tokens = http::exchange(&self.manager, code, verifier, &redirect).await?;
            return self.manager.activate(tokens, self.revision).await;
        }
    }
}

pub(super) async fn device(manager: AuthManager) -> Result<DeviceLogin> {
    let started = Instant::now();
    let revision = manager.inner.store.read()?.map(|record| record.revision);
    let response = http::post_json(
        &manager,
        "/api/accounts/deviceauth/usercode",
        &json!({"client_id":CLIENT_ID}),
    )
    .await?;
    let value = http::json_response(response).await?;
    let device_auth_id = nonempty(&value, "device_auth_id")?.to_owned();
    let user_code = nonempty(&value, "user_code")?.to_owned();
    let seconds = |name: &str, default: u64| -> Result<u64> {
        match value.get(name) {
            None => Ok(default),
            Some(value) => value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                .filter(|value| *value > 0)
                .context("invalid device login interval or expiry"),
        }
    };
    let interval = Duration::from_secs(seconds("interval", 5)?);
    ensure!(
        interval <= super::LOGIN_TIMEOUT,
        "device polling interval exceeds the login window"
    );
    let expiry = Duration::from_secs(seconds("expires_in", 600)?).min(manager.inner.login_timeout);
    let verification_url = format!("{}/codex/device", manager.inner.issuer);
    Ok(DeviceLogin {
        manager,
        verification_url,
        user_code,
        device_auth_id,
        interval,
        revision,
        deadline: started + expiry,
    })
}
fn nonempty<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= CALLBACK_BYTES
                && !value.chars().any(char::is_control)
        })
        .context("device login response lacks a valid required field")
}
