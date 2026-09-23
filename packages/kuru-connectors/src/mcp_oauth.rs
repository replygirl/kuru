//! Bounded protocol values for HTTP MCP authorization.
//!
//! Transport and persistence live in the MCP client. This module validates the
//! untrusted metadata and response shapes before either boundary receives them.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
    future::Future,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::header::{CONTENT_TYPE, LOCATION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::Instant,
};
use url::Url;

const MAX_OAUTH_DOCUMENT_BYTES: usize = 256 * 1024;
const MAX_URL_BYTES: usize = 2_048;
const MAX_ISSUERS: usize = 8;
const MAX_SCOPES: usize = 64;
const MAX_SCOPE_BYTES: usize = 256;
const MAX_TOKEN_BYTES: usize = 16 * 1024;
const MAX_USER_CODE_BYTES: usize = 256;
const MAX_ERROR_CODE_BYTES: usize = 128;
const MAX_REDIRECTS: usize = 4;
const MAX_CHALLENGE_BYTES: usize = 16 * 1024;
const MAX_CALLBACK_BYTES: usize = 8 * 1024;
const MAX_CALLBACK_CODE_BYTES: usize = 4 * 1024;
const CALLBACK_IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UrlPolicy {
    Https,
    LoopbackRedirect,
    #[cfg(test)]
    LoopbackFixture,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct CheckedUrl(Url);

impl CheckedUrl {
    pub(crate) fn parse(value: &str, policy: UrlPolicy) -> Result<Self> {
        ensure!(
            !value.is_empty() && value.len() <= MAX_URL_BYTES,
            "OAuth URL must contain 1-{MAX_URL_BYTES} bytes"
        );
        let url = Url::parse(value).context("OAuth URL is invalid")?;
        ensure!(
            url.username().is_empty(),
            "OAuth URL cannot contain credentials"
        );
        ensure!(
            url.password().is_none(),
            "OAuth URL cannot contain credentials"
        );
        ensure!(
            url.fragment().is_none(),
            "OAuth URL cannot contain a fragment"
        );
        ensure!(url.host_str().is_some(), "OAuth URL must have a host");
        match policy {
            UrlPolicy::Https => ensure!(url.scheme() == "https", "OAuth URL must use HTTPS"),
            UrlPolicy::LoopbackRedirect => {
                let loopback = url.host().is_some_and(|host| match host {
                    url::Host::Ipv4(address) => address.is_loopback(),
                    url::Host::Ipv6(address) => address.is_loopback(),
                    url::Host::Domain(_) => false,
                });
                ensure!(
                    url.scheme() == "https" || (url.scheme() == "http" && loopback),
                    "OAuth redirect URL must use HTTPS or loopback HTTP"
                );
            }
            #[cfg(test)]
            UrlPolicy::LoopbackFixture => {
                let loopback = url
                    .host_str()
                    .is_some_and(|host| host.eq_ignore_ascii_case("localhost"))
                    || url.host().is_some_and(|host| match host {
                        url::Host::Ipv4(address) => address.is_loopback(),
                        url::Host::Ipv6(address) => address.is_loopback(),
                        url::Host::Domain(_) => false,
                    });
                ensure!(
                    url.scheme() == "https" || (url.scheme() == "http" && loopback),
                    "OAuth fixture URL must use HTTPS or loopback HTTP"
                );
            }
        }
        Ok(Self(url))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn origin(&self) -> url::Origin {
        self.0.origin()
    }
}

impl fmt::Debug for CheckedUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CheckedUrl")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ScopeSet(BTreeSet<String>);

impl ScopeSet {
    pub(crate) fn from_space_separated(value: &str) -> Result<Self> {
        Self::from_values(value.split_ascii_whitespace())
    }

    pub(crate) fn from_values<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        let mut scopes = BTreeSet::new();
        for value in values {
            ensure!(
                !value.is_empty() && value.len() <= MAX_SCOPE_BYTES,
                "OAuth scope must contain 1-{MAX_SCOPE_BYTES} bytes"
            );
            ensure!(
                value
                    .bytes()
                    .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e)),
                "OAuth scope contains an invalid byte"
            );
            ensure!(scopes.insert(value.to_owned()), "duplicate OAuth scope");
            ensure!(scopes.len() <= MAX_SCOPES, "too many OAuth scopes");
        }
        Ok(Self(scopes))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    pub(crate) fn as_space_separated(&self) -> String {
        self.iter().collect::<Vec<_>>().join(" ")
    }

    pub(crate) fn is_subset(&self, ceiling: &Self) -> bool {
        self.0.is_subset(&ceiling.0)
    }

    fn intersection(&self, ceiling: &Self) -> Self {
        Self(self.0.intersection(&ceiling.0).cloned().collect())
    }

    fn union(&self, other: &Self) -> Result<Self> {
        let scopes = self.0.union(&other.0).cloned().collect::<BTreeSet<_>>();
        ensure!(scopes.len() <= MAX_SCOPES, "too many OAuth scopes");
        Ok(Self(scopes))
    }

    pub(crate) fn into_values(self) -> Vec<String> {
        self.0.into_iter().collect()
    }
}

#[derive(Debug, Deserialize)]
struct RawProtectedResourceMetadata {
    resource: String,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProtectedResourceMetadata {
    pub(crate) resource: CheckedUrl,
    pub(crate) authorization_servers: Vec<CheckedUrl>,
    pub(crate) scopes_supported: ScopeSet,
}

impl ProtectedResourceMetadata {
    pub(crate) fn parse(
        body: &[u8],
        expected_resource: &CheckedUrl,
        policy: UrlPolicy,
    ) -> Result<Self> {
        let raw: RawProtectedResourceMetadata = parse_document(body, "protected-resource")?;
        let resource = CheckedUrl::parse(&raw.resource, policy)?;
        ensure!(
            resource == *expected_resource,
            "protected-resource metadata changed the canonical resource"
        );
        ensure!(
            !raw.authorization_servers.is_empty() && raw.authorization_servers.len() <= MAX_ISSUERS,
            "protected-resource metadata must advertise 1-{MAX_ISSUERS} authorization servers"
        );
        let authorization_servers = raw
            .authorization_servers
            .iter()
            .map(|value| CheckedUrl::parse(value, policy))
            .collect::<Result<Vec<_>>>()?;
        let scopes_supported =
            ScopeSet::from_values(raw.scopes_supported.iter().map(String::as_str))?;
        Ok(Self {
            resource,
            authorization_servers,
            scopes_supported,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawAuthorizationServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    device_authorization_endpoint: Option<String>,
    #[serde(default)]
    revocation_endpoint: Option<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    grant_types_supported: Vec<String>,
    #[serde(default)]
    client_id_metadata_document_supported: bool,
    #[serde(default)]
    authorization_response_iss_parameter_supported: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizationServerMetadata {
    pub(crate) issuer: CheckedUrl,
    pub(crate) authorization_endpoint: CheckedUrl,
    pub(crate) token_endpoint: CheckedUrl,
    pub(crate) registration_endpoint: Option<CheckedUrl>,
    pub(crate) device_authorization_endpoint: Option<CheckedUrl>,
    pub(crate) revocation_endpoint: Option<CheckedUrl>,
    pub(crate) client_id_metadata_document_supported: bool,
    pub(crate) authorization_response_iss_parameter_supported: bool,
    pub(crate) supports_pkce_s256: bool,
    pub(crate) supports_device_code: bool,
}

impl AuthorizationServerMetadata {
    pub(crate) fn parse(
        body: &[u8],
        expected_issuer: &CheckedUrl,
        policy: UrlPolicy,
    ) -> Result<Self> {
        let raw: RawAuthorizationServerMetadata = parse_document(body, "authorization-server")?;
        ensure!(
            raw.issuer == expected_issuer.as_str(),
            "authorization issuer changed"
        );
        let issuer = CheckedUrl::parse(&raw.issuer, policy)?;
        ensure!(
            raw.code_challenge_methods_supported.len() <= 16
                && raw.grant_types_supported.len() <= 32,
            "authorization-server capability list exceeds its bound"
        );
        // OAuth metadata may deliberately delegate endpoints to another HTTPS
        // origin. Each endpoint is validated independently; issuer equality is
        // enforced on the metadata document itself.
        let endpoint = |value: &str| CheckedUrl::parse(value, policy);
        let optional_endpoint =
            |value: Option<&str>| -> Result<Option<CheckedUrl>> { value.map(endpoint).transpose() };
        let authorization_endpoint = endpoint(&raw.authorization_endpoint)?;
        let token_endpoint = endpoint(&raw.token_endpoint)?;
        let registration_endpoint = optional_endpoint(raw.registration_endpoint.as_deref())?;
        let device_authorization_endpoint =
            optional_endpoint(raw.device_authorization_endpoint.as_deref())?;
        let revocation_endpoint = optional_endpoint(raw.revocation_endpoint.as_deref())?;
        Ok(Self {
            issuer,
            authorization_endpoint,
            token_endpoint,
            registration_endpoint,
            device_authorization_endpoint,
            revocation_endpoint,
            client_id_metadata_document_supported: raw.client_id_metadata_document_supported,
            authorization_response_iss_parameter_supported: raw
                .authorization_response_iss_parameter_supported,
            supports_pkce_s256: raw
                .code_challenge_methods_supported
                .iter()
                .any(|method| method == "S256"),
            supports_device_code: raw
                .grant_types_supported
                .iter()
                .any(|grant| grant == "urn:ietf:params:oauth:grant-type:device_code"),
        })
    }
}

pub(crate) fn protected_resource_well_known(
    resource: &CheckedUrl,
    policy: UrlPolicy,
) -> Result<Vec<CheckedUrl>> {
    let mut root = resource.0.clone();
    root.set_query(None);
    root.set_path("/.well-known/oauth-protected-resource");
    let mut candidates = Vec::with_capacity(2);
    let path = resource.0.path().trim_start_matches('/');
    if !path.is_empty() {
        let mut path_candidate = root.clone();
        path_candidate.set_path(&format!("/.well-known/oauth-protected-resource/{path}"));
        candidates.push(CheckedUrl::parse(path_candidate.as_str(), policy)?);
    }
    candidates.push(CheckedUrl::parse(root.as_str(), policy)?);
    Ok(candidates)
}

pub(crate) fn authorization_server_well_known(
    issuer: &CheckedUrl,
    policy: UrlPolicy,
) -> Result<Vec<CheckedUrl>> {
    let mut root = issuer.0.clone();
    root.set_query(None);
    let path = issuer.0.path().trim_matches('/');
    let mut candidates = Vec::with_capacity(3);
    let suffix = |name: &str| {
        if path.is_empty() {
            format!("/.well-known/{name}")
        } else {
            format!("/.well-known/{name}/{path}")
        }
    };
    root.set_path(&suffix("oauth-authorization-server"));
    candidates.push(CheckedUrl::parse(root.as_str(), policy)?);
    root.set_path(&suffix("openid-configuration"));
    candidates.push(CheckedUrl::parse(root.as_str(), policy)?);
    if !path.is_empty() {
        root.set_path(&format!("/{path}/.well-known/openid-configuration"));
        candidates.push(CheckedUrl::parse(root.as_str(), policy)?);
    }
    Ok(candidates)
}

/// Fetch one OAuth metadata document without forwarding protected-resource
/// headers. Only bounded same-origin redirects are accepted.
async fn fetch_metadata_document(
    client: &reqwest::Client,
    initial: &CheckedUrl,
    policy: UrlPolicy,
) -> Result<Option<Vec<u8>>> {
    let mut guard = RedirectGuard::new(initial, policy);
    let mut current = initial.clone();
    loop {
        let mut response = client.get(current.as_str()).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .context("OAuth metadata redirect lacks Location")?
                .to_str()
                .context("OAuth metadata redirect Location is invalid")?;
            current = guard.follow(location)?;
            continue;
        }
        ensure!(
            response.status().is_success(),
            "OAuth metadata request failed: {}",
            response.status()
        );
        ensure!(
            response.content_length().unwrap_or(0) <= MAX_OAUTH_DOCUMENT_BYTES as u64,
            "OAuth metadata exceeds its byte bound"
        );
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                body.len() + chunk.len() <= MAX_OAUTH_DOCUMENT_BYTES,
                "OAuth metadata exceeds its byte bound"
            );
            body.extend_from_slice(&chunk);
        }
        return Ok(Some(body));
    }
}

pub(crate) async fn discover_protected_resource(
    client: &reqwest::Client,
    resource: &CheckedUrl,
    challenged_metadata: Option<&CheckedUrl>,
    policy: UrlPolicy,
) -> Result<ProtectedResourceMetadata> {
    let candidates = if let Some(metadata) = challenged_metadata {
        vec![metadata.clone()]
    } else {
        protected_resource_well_known(resource, policy)?
    };
    for candidate in candidates {
        if let Some(body) = fetch_metadata_document(client, &candidate, policy).await? {
            return ProtectedResourceMetadata::parse(&body, resource, policy);
        }
    }
    anyhow::bail!("OAuth protected-resource metadata was not found")
}

pub(crate) async fn discover_authorization_server(
    client: &reqwest::Client,
    issuer: &CheckedUrl,
    policy: UrlPolicy,
) -> Result<AuthorizationServerMetadata> {
    for candidate in authorization_server_well_known(issuer, policy)? {
        if let Some(body) = fetch_metadata_document(client, &candidate, policy).await? {
            return AuthorizationServerMetadata::parse(&body, issuer, policy);
        }
    }
    anyhow::bail!("OAuth authorization-server metadata was not found")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizationChallenge {
    pub(crate) resource_metadata: CheckedUrl,
    pub(crate) scopes: Option<ScopeSet>,
    pub(crate) error: Option<String>,
}

impl AuthorizationChallenge {
    pub(crate) fn parse(value: &str, policy: UrlPolicy) -> Result<Self> {
        ensure!(
            !value.is_empty() && value.len() <= MAX_CHALLENGE_BYTES && value.is_ascii(),
            "MCP authorization challenge exceeds its bound"
        );
        let (scheme, parameters) = value
            .split_once(char::is_whitespace)
            .context("MCP authorization challenge lacks parameters")?;
        ensure!(
            scheme.eq_ignore_ascii_case("Bearer"),
            "MCP authorization challenge is not Bearer"
        );
        let parameters = parse_auth_parameters(parameters)?;
        let resource_metadata = parameters
            .iter()
            .find_map(|(name, value)| (name == "resource_metadata").then_some(value))
            .context("MCP authorization challenge lacks resource_metadata")?;
        let scopes = parameters
            .iter()
            .find_map(|(name, value)| (name == "scope").then_some(value))
            .map(|value| ScopeSet::from_space_separated(value))
            .transpose()?;
        let error = parameters
            .iter()
            .find_map(|(name, value)| (name == "error").then_some(value.clone()));
        if let Some(error) = &error {
            ensure!(
                !error.is_empty()
                    && error.len() <= MAX_ERROR_CODE_BYTES
                    && error.bytes().all(|byte| byte.is_ascii_graphic()),
                "MCP authorization challenge error is invalid"
            );
        }
        Ok(Self {
            resource_metadata: CheckedUrl::parse(resource_metadata, policy)?,
            scopes,
            error,
        })
    }
}

fn parse_auth_parameters(value: &str) -> Result<Vec<(String, String)>> {
    let bytes = value.as_bytes();
    let mut offset = 0;
    let mut parameters = Vec::new();
    while offset < bytes.len() {
        while offset < bytes.len() && (bytes[offset].is_ascii_whitespace() || bytes[offset] == b',')
        {
            offset += 1;
        }
        if offset == bytes.len() {
            break;
        }
        let name_start = offset;
        while offset < bytes.len()
            && (bytes[offset].is_ascii_alphanumeric() || matches!(bytes[offset], b'_' | b'-'))
        {
            offset += 1;
        }
        ensure!(
            offset > name_start,
            "MCP authorization parameter name is invalid"
        );
        let name = value[name_start..offset].to_ascii_lowercase();
        while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        ensure!(
            bytes.get(offset) == Some(&b'='),
            "MCP authorization parameter lacks a value"
        );
        offset += 1;
        while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        let parameter = if bytes.get(offset) == Some(&b'"') {
            offset += 1;
            let mut decoded = String::new();
            let mut closed = false;
            while offset < bytes.len() {
                match bytes[offset] {
                    b'"' => {
                        offset += 1;
                        closed = true;
                        break;
                    }
                    b'\\' => {
                        offset += 1;
                        let byte = *bytes
                            .get(offset)
                            .context("MCP authorization quoted value has a trailing escape")?;
                        ensure!(
                            byte.is_ascii() && !matches!(byte, b'\r' | b'\n'),
                            "MCP authorization quoted value is invalid"
                        );
                        decoded.push(char::from(byte));
                        offset += 1;
                    }
                    byte => {
                        ensure!(
                            byte.is_ascii() && !matches!(byte, b'\r' | b'\n'),
                            "MCP authorization quoted value is invalid"
                        );
                        decoded.push(char::from(byte));
                        offset += 1;
                    }
                }
            }
            ensure!(closed, "MCP authorization quoted value is unterminated");
            decoded
        } else {
            let value_start = offset;
            while offset < bytes.len()
                && !bytes[offset].is_ascii_whitespace()
                && bytes[offset] != b','
            {
                offset += 1;
            }
            ensure!(
                offset > value_start,
                "MCP authorization parameter value is empty"
            );
            value[value_start..offset].to_owned()
        };
        ensure!(
            !parameters.iter().any(|(existing, _)| existing == &name),
            "duplicate MCP authorization parameter"
        );
        parameters.push((name, parameter));
        ensure!(
            parameters.len() <= 16,
            "too many MCP authorization parameters"
        );
        while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        if offset < bytes.len() {
            ensure!(
                bytes[offset] == b',',
                "MCP authorization parameters are malformed"
            );
        }
    }
    Ok(parameters)
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct SecretText(String);

impl SecretText {
    pub(crate) fn new(kind: &str, value: String) -> Result<Self> {
        ensure!(
            !value.is_empty() && value.len() <= MAX_TOKEN_BYTES,
            "OAuth {kind} must contain 1-{MAX_TOKEN_BYTES} bytes"
        );
        Ok(Self(value))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretText([REDACTED])")
    }
}

#[derive(Debug, Deserialize)]
struct RawRegistrationResponse {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct RegistrationResponse {
    pub(crate) client_id: String,
    pub(crate) client_secret: Option<SecretText>,
}

impl fmt::Debug for RegistrationResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistrationResponse")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl RegistrationResponse {
    pub(crate) fn parse(body: &[u8]) -> Result<Self> {
        let raw: RawRegistrationResponse = parse_document(body, "registration")?;
        ensure!(
            !raw.client_id.is_empty() && raw.client_id.len() <= MAX_URL_BYTES,
            "registered client ID exceeds its bound"
        );
        Ok(Self {
            client_id: raw.client_id,
            client_secret: raw
                .client_secret
                .map(|value| SecretText::new("client secret", value))
                .transpose()?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawTokenResponse {
    access_token: String,
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: SecretText,
    pub(crate) expires_in: Option<u64>,
    pub(crate) refresh_token: Option<SecretText>,
    pub(crate) scopes: Option<ScopeSet>,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenResponse")
            .field("access_token", &"[REDACTED]")
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl TokenResponse {
    pub(crate) fn parse(body: &[u8]) -> Result<Self> {
        let raw: RawTokenResponse = parse_document(body, "token")?;
        ensure!(
            raw.token_type.eq_ignore_ascii_case("bearer"),
            "OAuth token type must be Bearer"
        );
        ensure!(
            raw.expires_in
                .is_none_or(|seconds| seconds > 0 && seconds <= 31_536_000),
            "OAuth token lifetime exceeds its bound"
        );
        Ok(Self {
            access_token: SecretText::new("access token", raw.access_token)?,
            expires_in: raw.expires_in,
            refresh_token: raw
                .refresh_token
                .map(|value| SecretText::new("refresh token", value))
                .transpose()?,
            scopes: raw
                .scope
                .as_deref()
                .map(ScopeSet::from_space_separated)
                .transpose()?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawDeviceAuthorizationResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct DeviceAuthorizationResponse {
    pub(crate) device_code: SecretText,
    pub(crate) user_code: SecretText,
    pub(crate) verification_uri: CheckedUrl,
    pub(crate) verification_uri_complete: Option<CheckedUrl>,
    pub(crate) expires_in: u64,
    pub(crate) interval: u64,
}

impl fmt::Debug for DeviceAuthorizationResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceAuthorizationResponse")
            .field("device_code", &"[REDACTED]")
            .field("user_code", &"[REDACTED]")
            .field("verification_uri", &self.verification_uri)
            .field("verification_uri_complete", &self.verification_uri_complete)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish()
    }
}

impl DeviceAuthorizationResponse {
    pub(crate) fn parse(body: &[u8], policy: UrlPolicy) -> Result<Self> {
        let raw: RawDeviceAuthorizationResponse = parse_document(body, "device authorization")?;
        ensure!(
            !raw.user_code.is_empty() && raw.user_code.len() <= MAX_USER_CODE_BYTES,
            "OAuth user code exceeds its bound"
        );
        ensure!(
            raw.expires_in > 0 && raw.expires_in <= 3_600,
            "OAuth device-code lifetime exceeds its bound"
        );
        let interval = raw.interval.unwrap_or(5);
        ensure!(
            (1..=60).contains(&interval),
            "OAuth device polling interval exceeds its bound"
        );
        Ok(Self {
            device_code: SecretText::new("device code", raw.device_code)?,
            user_code: SecretText::new("user code", raw.user_code)?,
            verification_uri: CheckedUrl::parse(&raw.verification_uri, policy)?,
            verification_uri_complete: raw
                .verification_uri_complete
                .as_deref()
                .map(|value| CheckedUrl::parse(value, policy))
                .transpose()?,
            expires_in: raw.expires_in,
            interval,
        })
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct OAuthErrorResponse {
    code: String,
}

impl OAuthErrorResponse {
    pub(crate) fn parse(body: &[u8]) -> Result<Self> {
        #[derive(Deserialize)]
        struct RawErrorResponse {
            error: String,
            #[serde(default)]
            error_description: Option<String>,
            #[serde(default)]
            error_uri: Option<String>,
        }

        let raw: RawErrorResponse = parse_document(body, "error")?;
        ensure!(
            !raw.error.is_empty()
                && raw.error.len() <= MAX_ERROR_CODE_BYTES
                && raw.error.bytes().all(|byte| byte.is_ascii_graphic()),
            "OAuth error code is invalid"
        );
        // Remote descriptions and URIs are intentionally parsed only to bound
        // the complete document. They never enter diagnostics because an
        // authority can reflect submitted credentials into either field.
        let _ = raw.error_description;
        let _ = raw.error_uri;
        Ok(Self { code: raw.error })
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Debug for OAuthErrorResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthErrorResponse")
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RevocationOutcome {
    Revoked,
    NoAdvertisedEndpoint,
    RemoteRefused,
    RemoteOutcomeUncertain,
}

/// Exchange one accepted loopback authorization code. The caller deliberately
/// owns retry policy: a transport failure after dispatch is ambiguous and this
/// function never repeats a token request.
pub(crate) async fn exchange_authorization_code(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    client_id: &str,
    client_secret: Option<&SecretText>,
    resource: &CheckedUrl,
    authorization: &AuthorizationCode,
) -> Result<TokenResponse> {
    let mut form = vec![
        ("grant_type", "authorization_code".to_owned()),
        ("client_id", client_id.to_owned()),
        ("code", authorization.code().to_owned()),
        ("code_verifier", authorization.verifier().to_owned()),
        ("redirect_uri", authorization.redirect_uri().to_owned()),
        ("resource", resource.as_str().to_owned()),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret.expose().to_owned()));
    }
    token_request(client, &metadata.token_endpoint, &form).await
}

/// Perform advertised native dynamic registration. Redirect handling is
/// disabled on the shared client, so registration can never escape the exact
/// endpoint selected from validated metadata.
pub(crate) async fn register_dynamic_client(
    client: &reqwest::Client,
    endpoint: &CheckedUrl,
    redirect_uri: &CheckedUrl,
) -> Result<RegistrationResponse> {
    let response = client
        .post(endpoint.as_str())
        .json(&DynamicRegistrationRequest::native(redirect_uri))
        .send()
        .await
        .context("dispatch MCP OAuth dynamic registration")?;
    let status = response.status();
    let body = bounded_oauth_body(response).await?;
    ensure!(
        status.is_success(),
        "MCP OAuth dynamic registration was refused: {status}"
    );
    RegistrationResponse::parse(&body)
}

/// Start one advertised device grant. The returned device code remains a
/// redacted in-memory value and is never persisted as a credential.
pub(crate) async fn begin_device_authorization(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    client_id: &str,
    client_secret: Option<&SecretText>,
    resource: &CheckedUrl,
    scopes: Option<&ScopeSet>,
    policy: UrlPolicy,
) -> Result<DeviceAuthorizationResponse> {
    ensure!(
        metadata.supports_device_code,
        "authorization server does not advertise device authorization"
    );
    let endpoint = metadata
        .device_authorization_endpoint
        .as_ref()
        .context("authorization server omitted its device authorization endpoint")?;
    let mut form = vec![
        ("client_id", client_id.to_owned()),
        ("resource", resource.as_str().to_owned()),
    ];
    if let Some(scopes) = scopes {
        form.push(("scope", scopes.as_space_separated()));
    }
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret.expose().to_owned()));
    }
    let response = client
        .post(endpoint.as_str())
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(encode_form(&form))
        .send()
        .await
        .context("dispatch MCP OAuth device authorization")?;
    let status = response.status();
    let body = bounded_oauth_body(response).await?;
    ensure!(
        status.is_success(),
        "MCP OAuth device authorization was refused: {status}"
    );
    DeviceAuthorizationResponse::parse(&body, policy)
}

/// Poll a device grant at the server's pace. Dropping this future cancels all
/// later polling; each token request itself is issued at most once.
pub(crate) async fn finish_device_authorization(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    client_id: &str,
    client_secret: Option<&SecretText>,
    resource: &CheckedUrl,
    device: &DeviceAuthorizationResponse,
) -> Result<TokenResponse> {
    finish_device_authorization_with_cancellation(
        client,
        metadata,
        client_id,
        client_secret,
        resource,
        device,
        std::future::pending(),
    )
    .await
}

pub(crate) async fn finish_device_authorization_with_cancellation<C>(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    client_id: &str,
    client_secret: Option<&SecretText>,
    resource: &CheckedUrl,
    device: &DeviceAuthorizationResponse,
    cancellation: C,
) -> Result<TokenResponse>
where
    C: Future<Output = Result<()>>,
{
    tokio::pin!(cancellation);
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = Duration::from_secs(device.interval);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until((Instant::now() + interval).min(deadline)) => {}
            cancelled = &mut cancellation => {
                cancelled?;
                anyhow::bail!("MCP OAuth login cancelled");
            }
        }
        ensure!(
            Instant::now() < deadline,
            "MCP OAuth device authorization expired"
        );
        let mut form = vec![
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code".to_owned(),
            ),
            ("client_id", client_id.to_owned()),
            ("device_code", device.device_code.expose().to_owned()),
            ("resource", resource.as_str().to_owned()),
        ];
        if let Some(secret) = client_secret {
            form.push(("client_secret", secret.expose().to_owned()));
        }
        let response = client
            .post(metadata.token_endpoint.as_str())
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(encode_form(&form))
            .send()
            .await
            .context("dispatch MCP OAuth device token request")?;
        let status = response.status();
        let body = bounded_oauth_body(response).await?;
        if status.is_success() {
            return TokenResponse::parse(&body);
        }
        let error = OAuthErrorResponse::parse(&body)?;
        match error.code() {
            "authorization_pending" => {}
            "slow_down" => {
                interval = interval
                    .checked_add(Duration::from_secs(5))
                    .context("MCP OAuth device polling interval overflowed")?;
                ensure!(
                    interval <= Duration::from_secs(60),
                    "MCP OAuth device polling interval exceeds its bound"
                );
            }
            "access_denied" => anyhow::bail!("MCP OAuth device authorization was declined"),
            "expired_token" => anyhow::bail!("MCP OAuth device authorization expired"),
            code => anyhow::bail!("MCP OAuth device authorization failed: {code}"),
        }
    }
}

/// Rotate an access snapshot once. An ambiguous dispatch is returned to the
/// caller and is never retried with the same rotating refresh credential.
pub(crate) async fn refresh_access_token(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    client_id: &str,
    client_secret: Option<&SecretText>,
    resource: &CheckedUrl,
    refresh_token: &SecretText,
    scopes: Option<&ScopeSet>,
) -> Result<TokenResponse> {
    let mut form = vec![
        ("grant_type", "refresh_token".to_owned()),
        ("client_id", client_id.to_owned()),
        ("refresh_token", refresh_token.expose().to_owned()),
        ("resource", resource.as_str().to_owned()),
    ];
    if let Some(scopes) = scopes {
        form.push(("scope", scopes.as_space_separated()));
    }
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret.expose().to_owned()));
    }
    token_request(client, &metadata.token_endpoint, &form).await
}

/// Attempt remote revocation exactly once. Local deletion is intentionally a
/// separate caller-owned step and must run for every returned outcome.
pub(crate) async fn revoke_remote(
    client: &reqwest::Client,
    metadata: &AuthorizationServerMetadata,
    client_id: &str,
    client_secret: Option<&SecretText>,
    token: &SecretText,
) -> RevocationOutcome {
    let Some(endpoint) = metadata.revocation_endpoint.as_ref() else {
        return RevocationOutcome::NoAdvertisedEndpoint;
    };
    let mut form = vec![
        ("token", token.expose().to_owned()),
        ("token_type_hint", "refresh_token".to_owned()),
        ("client_id", client_id.to_owned()),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret.expose().to_owned()));
    }
    let response = match client
        .post(endpoint.as_str())
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(encode_form(&form))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return RevocationOutcome::RemoteOutcomeUncertain,
    };
    let status = response.status();
    match bounded_oauth_body_allow_empty(response).await {
        Ok(_) if status.is_success() => RevocationOutcome::Revoked,
        Ok(_) => RevocationOutcome::RemoteRefused,
        Err(_) => RevocationOutcome::RemoteOutcomeUncertain,
    }
}

async fn token_request(
    client: &reqwest::Client,
    endpoint: &CheckedUrl,
    form: &[(&'static str, String)],
) -> Result<TokenResponse> {
    let response = client
        .post(endpoint.as_str())
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(encode_form(form))
        .send()
        .await
        .context("dispatch MCP OAuth token request")?;
    let status = response.status();
    let body = bounded_oauth_body(response).await?;
    if status.is_success() {
        TokenResponse::parse(&body)
    } else {
        let error = OAuthErrorResponse::parse(&body)?;
        anyhow::bail!("MCP OAuth token request failed: {}", error.code())
    }
}

fn encode_form(form: &[(&'static str, String)]) -> String {
    let mut encoded = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in form {
        encoded.append_pair(name, value);
    }
    encoded.finish()
}

async fn bounded_oauth_body(response: reqwest::Response) -> Result<Vec<u8>> {
    let body = bounded_oauth_body_allow_empty(response).await?;
    ensure!(!body.is_empty(), "OAuth response body is empty");
    Ok(body)
}

async fn bounded_oauth_body_allow_empty(mut response: reqwest::Response) -> Result<Vec<u8>> {
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_OAUTH_DOCUMENT_BYTES as u64,
        "OAuth response exceeds its byte bound"
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            body.len() + chunk.len() <= MAX_OAUTH_DOCUMENT_BYTES,
            "OAuth response exceeds its byte bound"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DynamicRegistrationRequest<'a> {
    pub(crate) application_type: &'static str,
    pub(crate) client_name: &'static str,
    pub(crate) redirect_uris: [&'a str; 1],
    pub(crate) grant_types: [&'static str; 2],
    pub(crate) response_types: [&'static str; 1],
    pub(crate) token_endpoint_auth_method: &'static str,
}

impl<'a> DynamicRegistrationRequest<'a> {
    pub(crate) fn native(redirect_uri: &'a CheckedUrl) -> Self {
        Self {
            application_type: "native",
            client_name: "Kuru",
            redirect_uris: [redirect_uri.as_str()],
            grant_types: [
                "authorization_code",
                "urn:ietf:params:oauth:grant-type:device_code",
            ],
            response_types: ["code"],
            token_endpoint_auth_method: "none",
        }
    }
}

pub(crate) fn validate_authorization_response_issuer(
    expected_issuer: &CheckedUrl,
    issuer_parameter_supported: bool,
    returned_issuer: Option<&str>,
    policy: UrlPolicy,
) -> Result<()> {
    if issuer_parameter_supported {
        ensure!(
            returned_issuer.is_some(),
            "authorization response omitted required issuer"
        );
    }
    if let Some(returned) = returned_issuer {
        ensure!(
            returned == expected_issuer.as_str(),
            "authorization response issuer changed"
        );
        let _ = CheckedUrl::parse(returned, policy)?;
    }
    Ok(())
}

/// One owned loopback authorization-code attempt. The state and verifier have
/// no Debug or serialization surface and disappear with this value.
pub(crate) struct AuthorizationCodePreparation {
    listener: TcpListener,
    redirect_uri: String,
    state: String,
    verifier: String,
    issuer: CheckedUrl,
    authorization_endpoint: CheckedUrl,
    issuer_parameter_supported: bool,
    resource: CheckedUrl,
    scopes: Option<ScopeSet>,
    policy: UrlPolicy,
    deadline: Instant,
}

pub(crate) struct AuthorizationCodeLogin {
    listener: TcpListener,
    authorization_url: String,
    redirect_uri: String,
    state: String,
    verifier: String,
    issuer: CheckedUrl,
    issuer_parameter_supported: bool,
    policy: UrlPolicy,
    deadline: Instant,
}

pub(crate) struct AuthorizationCode {
    code: SecretText,
    verifier: SecretText,
    redirect_uri: String,
}

impl AuthorizationCode {
    pub(crate) fn code(&self) -> &str {
        self.code.expose()
    }

    pub(crate) fn verifier(&self) -> &str {
        self.verifier.expose()
    }

    pub(crate) fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }
}

impl AuthorizationCodeLogin {
    #[cfg(test)]
    pub(crate) async fn bind(
        metadata: &AuthorizationServerMetadata,
        client_id: &str,
        resource: &CheckedUrl,
        scopes: Option<&ScopeSet>,
        timeout: Duration,
        policy: UrlPolicy,
    ) -> Result<Self> {
        AuthorizationCodePreparation::bind(metadata, resource, scopes, timeout, policy)
            .await?
            .authorize(client_id)
    }

    pub(crate) fn authorization_url(&self) -> &str {
        &self.authorization_url
    }

    #[cfg(test)]
    pub(crate) fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub(crate) async fn finish(self) -> Result<AuthorizationCode> {
        tokio::time::timeout_at(self.deadline, self.accept_callback())
            .await
            .context("MCP OAuth browser login expired")?
    }

    async fn accept_callback(self) -> Result<AuthorizationCode> {
        loop {
            let (mut socket, peer) = self
                .listener
                .accept()
                .await
                .context("accept MCP OAuth callback")?;
            ensure!(
                peer.ip().is_loopback(),
                "OAuth callback peer is not loopback"
            );
            let callback = tokio::time::timeout(
                CALLBACK_IO_TIMEOUT,
                read_authorization_callback(
                    &mut socket,
                    self.listener.local_addr()?.port(),
                    &self.state,
                ),
            )
            .await;
            let callback = match callback {
                Ok(Ok(callback)) => callback,
                _ => {
                    reply_callback(&mut socket, 400, "Invalid authorization callback.").await;
                    continue;
                }
            };
            if callback.denied {
                reply_callback(&mut socket, 400, "Authorization was declined.").await;
                anyhow::bail!("MCP OAuth authorization was declined");
            }
            if let Err(error) = validate_authorization_response_issuer(
                &self.issuer,
                self.issuer_parameter_supported,
                callback.issuer.as_deref(),
                self.policy,
            ) {
                reply_callback(&mut socket, 400, "Invalid authorization issuer.").await;
                return Err(error);
            }
            let code = callback.code.context("authorization callback lacks code")?;
            reply_callback(&mut socket, 200, "Authorization received; return to Kuru.").await;
            return Ok(AuthorizationCode {
                code: SecretText::new("authorization code", code)?,
                verifier: SecretText::new("PKCE verifier", self.verifier)?,
                redirect_uri: self.redirect_uri,
            });
        }
    }
}

impl AuthorizationCodePreparation {
    pub(crate) async fn bind(
        metadata: &AuthorizationServerMetadata,
        resource: &CheckedUrl,
        scopes: Option<&ScopeSet>,
        timeout: Duration,
        policy: UrlPolicy,
    ) -> Result<Self> {
        ensure!(
            metadata.supports_pkce_s256,
            "authorization server does not support PKCE S256"
        );
        ensure!(
            !timeout.is_zero() && timeout <= MAX_LOGIN_TIMEOUT,
            "OAuth login timeout exceeds its bound"
        );
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .context("bind MCP OAuth loopback callback")?;
        let redirect_uri = format!(
            "http://127.0.0.1:{}/oauth/callback",
            listener.local_addr()?.port()
        );
        let verifier = random_secret(64)?;
        let state = random_secret(32)?;
        Ok(Self {
            listener,
            redirect_uri,
            state,
            verifier,
            issuer: metadata.issuer.clone(),
            authorization_endpoint: metadata.authorization_endpoint.clone(),
            issuer_parameter_supported: metadata.authorization_response_iss_parameter_supported,
            resource: resource.clone(),
            scopes: scopes.cloned(),
            policy,
            deadline: Instant::now() + timeout,
        })
    }

    pub(crate) fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub(crate) fn authorize(self, client_id: &str) -> Result<AuthorizationCodeLogin> {
        ensure!(
            !client_id.is_empty() && client_id.len() <= MAX_URL_BYTES,
            "OAuth client ID exceeds its bound"
        );
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()));
        let mut authorization_url = self.authorization_endpoint.0.clone();
        {
            let mut query = authorization_url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", client_id)
                .append_pair("redirect_uri", &self.redirect_uri)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &self.state)
                .append_pair("resource", self.resource.as_str());
            if let Some(scopes) = &self.scopes {
                query.append_pair("scope", &scopes.iter().collect::<Vec<_>>().join(" "));
            }
        }
        Ok(AuthorizationCodeLogin {
            listener: self.listener,
            authorization_url: authorization_url.into(),
            redirect_uri: self.redirect_uri,
            state: self.state,
            verifier: self.verifier,
            issuer: self.issuer,
            issuer_parameter_supported: self.issuer_parameter_supported,
            policy: self.policy,
            deadline: self.deadline,
        })
    }
}

struct AuthorizationCallback {
    code: Option<String>,
    issuer: Option<String>,
    denied: bool,
}

async fn read_authorization_callback(
    socket: &mut TcpStream,
    port: u16,
    expected_state: &str,
) -> Result<AuthorizationCallback> {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0_u8; 1024];
        let count = socket.read(&mut chunk).await?;
        ensure!(count > 0, "incomplete OAuth callback request");
        ensure!(
            bytes.len() + count <= MAX_CALLBACK_BYTES,
            "OAuth callback request exceeds its byte bound"
        );
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            ensure!(
                end + 4 == bytes.len(),
                "OAuth callback cannot contain a body"
            );
            break;
        }
    }
    let request = std::str::from_utf8(&bytes).context("OAuth callback request is not UTF-8")?;
    let mut lines = request.split("\r\n");
    let first = lines
        .next()
        .unwrap_or_default()
        .split(' ')
        .collect::<Vec<_>>();
    ensure!(
        first.len() == 3 && first[0] == "GET" && first[2] == "HTTP/1.1",
        "OAuth callback method or protocol is invalid"
    );
    ensure!(
        first[1].starts_with('/') && !first[1].starts_with("//") && !first[1].contains('#'),
        "OAuth callback target is invalid"
    );
    let mut headers = HashMap::new();
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .context("OAuth callback header is invalid")?;
        ensure!(
            !name.is_empty() && !name.chars().any(char::is_whitespace),
            "OAuth callback header name is invalid"
        );
        ensure!(
            headers
                .insert(name.to_ascii_lowercase(), value.trim())
                .is_none(),
            "duplicate OAuth callback header"
        );
    }
    let expected_host = format!("127.0.0.1:{port}");
    ensure!(
        headers.get("host") == Some(&expected_host.as_str()),
        "OAuth callback Host is invalid"
    );
    ensure!(
        !headers.contains_key("transfer-encoding")
            && headers
                .get("content-length")
                .is_none_or(|value| *value == "0"),
        "OAuth callback body is unsupported"
    );
    let url = Url::parse(&format!("http://127.0.0.1{}", first[1]))?;
    ensure!(
        url.path() == "/oauth/callback",
        "OAuth callback path is invalid"
    );
    let mut seen = HashSet::new();
    let mut values = HashMap::new();
    for (name, value) in url.query_pairs() {
        ensure!(seen.insert(name.clone()), "duplicate OAuth callback value");
        values.insert(name.into_owned(), value.into_owned());
    }
    ensure!(
        values.get("state").map(String::as_str) == Some(expected_state),
        "OAuth callback state changed"
    );
    let denied = values.contains_key("error");
    ensure!(
        !(denied && values.contains_key("code")),
        "OAuth callback result is ambiguous"
    );
    let code = values.remove("code");
    if let Some(code) = &code {
        ensure!(
            !code.is_empty() && code.len() <= MAX_CALLBACK_CODE_BYTES,
            "OAuth authorization code exceeds its bound"
        );
    }
    ensure!(denied || code.is_some(), "OAuth callback lacks a result");
    Ok(AuthorizationCallback {
        code,
        issuer: values.remove("iss"),
        denied,
    })
}

async fn reply_callback(socket: &mut TcpStream, status: u16, text: &str) {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = tokio::time::timeout(CALLBACK_IO_TIMEOUT, socket.write_all(response.as_bytes())).await;
}

fn random_secret(bytes: usize) -> Result<String> {
    let mut value = vec![0_u8; bytes];
    getrandom::fill(&mut value).context("operating system randomness is unavailable")?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}

pub(crate) fn select_initial_scopes(
    challenge: Option<&ScopeSet>,
    resource_supported: &ScopeSet,
    configured_ceiling: Option<&ScopeSet>,
) -> Result<Option<ScopeSet>> {
    if let Some(authoritative) = challenge {
        if let Some(ceiling) = configured_ceiling {
            ensure!(
                authoritative.is_subset(ceiling),
                "authorization challenge requires a scope outside the configured ceiling"
            );
        }
        return Ok((!authoritative.is_empty()).then(|| authoritative.clone()));
    }
    let selected = configured_ceiling.map_or_else(
        || resource_supported.clone(),
        |ceiling| resource_supported.intersection(ceiling),
    );
    Ok((!selected.is_empty()).then_some(selected))
}

pub(crate) fn select_step_up_scopes(
    prior: &ScopeSet,
    challenge: &ScopeSet,
    configured_ceiling: Option<&ScopeSet>,
) -> Result<ScopeSet> {
    if let Some(ceiling) = configured_ceiling {
        ensure!(
            challenge.is_subset(ceiling),
            "authorization challenge requires a scope outside the configured ceiling"
        );
    }
    let selected = prior.union(challenge)?;
    if let Some(ceiling) = configured_ceiling {
        ensure!(
            selected.is_subset(ceiling),
            "previously granted scope is outside the configured ceiling"
        );
    }
    Ok(selected)
}

#[derive(Debug)]
pub(crate) struct RedirectGuard {
    origin: url::Origin,
    followed: usize,
    policy: UrlPolicy,
}

impl RedirectGuard {
    pub(crate) fn new(initial: &CheckedUrl, policy: UrlPolicy) -> Self {
        Self {
            origin: initial.origin(),
            followed: 0,
            policy,
        }
    }

    pub(crate) fn follow(&mut self, target: &str) -> Result<CheckedUrl> {
        ensure!(
            self.followed < MAX_REDIRECTS,
            "too many OAuth metadata redirects"
        );
        let target = CheckedUrl::parse(target, self.policy)?;
        ensure!(
            target.origin() == self.origin,
            "OAuth metadata redirect escaped its authority"
        );
        self.followed += 1;
        Ok(target)
    }
}

fn parse_document<T: for<'de> Deserialize<'de>>(body: &[u8], kind: &str) -> Result<T> {
    ensure!(
        !body.is_empty() && body.len() <= MAX_OAUTH_DOCUMENT_BYTES,
        "OAuth {kind} document must contain 1-{MAX_OAUTH_DOCUMENT_BYTES} bytes"
    );
    serde_json::from_slice(body).with_context(|| format!("OAuth {kind} document is invalid"))
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use axum::{
        Json, Router,
        body::{Body, Bytes},
        extract::State,
        http::{HeaderMap, Method, StatusCode, Uri},
        response::{IntoResponse, Response},
        routing::{any, get, post},
    };
    use serde_json::json;
    use tokio::sync::Mutex;

    use super::*;

    fn loopback(value: &str) -> CheckedUrl {
        CheckedUrl::parse(value, UrlPolicy::LoopbackFixture).unwrap()
    }

    fn scopes(values: &[&str]) -> ScopeSet {
        ScopeSet::from_values(values.iter().copied()).unwrap()
    }

    fn authorization_metadata(base: &str) -> AuthorizationServerMetadata {
        let issuer = loopback(&format!("{base}/issuer"));
        AuthorizationServerMetadata::parse(
            serde_json::to_vec(&json!({
                "issuer": issuer.as_str(),
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
                "code_challenge_methods_supported": ["S256"],
                "grant_types_supported": ["authorization_code"],
                "authorization_response_iss_parameter_supported": true
            }))
            .unwrap()
            .as_slice(),
            &issuer,
            UrlPolicy::LoopbackFixture,
        )
        .unwrap()
    }

    async fn callback(redirect: &str, query: &str) -> String {
        let url = Url::parse(redirect).unwrap();
        let mut socket = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
            .await
            .unwrap();
        socket
            .write_all(
                format!(
                    "GET {}?{query} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                    url.path(),
                    url.port().unwrap()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).await.unwrap();
        response
    }

    #[derive(Clone)]
    struct DiscoveryFixture {
        base: String,
        requests: Arc<Mutex<Vec<(String, HeaderMap)>>>,
    }

    async fn discovery_reply(
        State(state): State<DiscoveryFixture>,
        uri: Uri,
        headers: HeaderMap,
    ) -> Response {
        state
            .requests
            .lock()
            .await
            .push((uri.path().into(), headers));
        match uri.path() {
            "/.well-known/oauth-protected-resource/resource"
            | "/.well-known/oauth-authorization-server/issuer" => {
                StatusCode::NOT_FOUND.into_response()
            }
            "/.well-known/oauth-protected-resource" => Json(json!({
                "resource": format!("{}/resource", state.base),
                "authorization_servers": [format!("{}/issuer", state.base)],
                "scopes_supported": ["mcp.read"]
            }))
            .into_response(),
            "/.well-known/openid-configuration/issuer" => Json(json!({
                "issuer": format!("{}/issuer", state.base),
                "authorization_endpoint": format!("{}/authorize", state.base),
                "token_endpoint": format!("{}/token", state.base),
                "code_challenge_methods_supported": ["S256"],
                "grant_types_supported": ["authorization_code"]
            }))
            .into_response(),
            _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }

    #[tokio::test]
    async fn metadata_discovery_uses_exact_fallback_order_without_forwarded_secrets() {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let fixture = DiscoveryFixture {
            base: base.clone(),
            requests: requests.clone(),
        };
        let task = tokio::spawn(async move {
            axum::serve(
                socket,
                Router::new()
                    .fallback(get(discovery_reply))
                    .with_state(fixture),
            )
            .await
            .unwrap();
        });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let resource = loopback(&format!("{base}/resource"));
        let protected =
            discover_protected_resource(&client, &resource, None, UrlPolicy::LoopbackFixture)
                .await
                .unwrap();
        let metadata = discover_authorization_server(
            &client,
            &protected.authorization_servers[0],
            UrlPolicy::LoopbackFixture,
        )
        .await
        .unwrap();
        assert!(metadata.supports_pkce_s256);

        let requests = requests.lock().await;
        assert_eq!(
            requests
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            [
                "/.well-known/oauth-protected-resource/resource",
                "/.well-known/oauth-protected-resource",
                "/.well-known/oauth-authorization-server/issuer",
                "/.well-known/openid-configuration/issuer"
            ]
        );
        assert!(requests.iter().all(|(_, headers)| {
            !headers.contains_key("authorization")
                && !headers.contains_key("proxy-authorization")
                && !headers.contains_key("x-private-static")
        }));
        drop(requests);
        task.abort();
    }

    #[tokio::test]
    async fn loopback_code_flow_binds_state_pkce_resource_issuer_and_single_use_listener() {
        let metadata = authorization_metadata("http://127.0.0.1:4200");
        let resource = loopback("http://127.0.0.1:4100/mcp");
        let selected_scopes = scopes(&["mcp.call", "mcp.read"]);
        let login = AuthorizationCodeLogin::bind(
            &metadata,
            "client-one",
            &resource,
            Some(&selected_scopes),
            Duration::from_secs(5),
            UrlPolicy::LoopbackFixture,
        )
        .await
        .unwrap();
        let authorization_url = Url::parse(login.authorization_url()).unwrap();
        let query = authorization_url
            .query_pairs()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect::<HashMap<_, _>>();
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["client_id"], "client-one");
        assert_eq!(query["resource"], resource.as_str());
        assert_eq!(query["scope"], "mcp.call mcp.read");
        assert_eq!(query["code_challenge_method"], "S256");
        assert!(!query["code_challenge"].is_empty());
        let redirect = login.redirect_uri().to_owned();
        assert_eq!(query["redirect_uri"], redirect);
        let state = query["state"].clone();
        let issuer = metadata.issuer.as_str().to_owned();
        let finished = tokio::spawn(login.finish());

        let rejected = callback(&redirect, "state=wrong&code=must-not-be-used").await;
        assert!(rejected.starts_with("HTTP/1.1 400"));
        let accepted = callback(
            &redirect,
            &format!(
                "state={state}&code=accepted-code&iss={}",
                url::form_urlencoded::byte_serialize(issuer.as_bytes()).collect::<String>()
            ),
        )
        .await;
        assert!(accepted.starts_with("HTTP/1.1 200"));
        let code = finished.await.unwrap().unwrap();
        assert_eq!(code.code(), "accepted-code");
        assert_eq!(code.redirect_uri(), redirect);
        assert!(code.verifier().len() >= 64);

        let port = Url::parse(&redirect).unwrap().port().unwrap();
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }

    #[test]
    fn metadata_urls_redirects_and_capabilities_are_bounded() {
        let resource = loopback("http://127.0.0.1:4100/mcp");
        let protected = ProtectedResourceMetadata::parse(
            serde_json::to_vec(&json!({
                "resource": resource.as_str(),
                "authorization_servers": ["http://127.0.0.1:4200/issuer"],
                "scopes_supported": ["mcp.read", "mcp.call"],
                "ignored_extension": {"safe": true}
            }))
            .unwrap()
            .as_slice(),
            &resource,
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        let issuer = &protected.authorization_servers[0];
        let metadata = AuthorizationServerMetadata::parse(
            serde_json::to_vec(&json!({
                "issuer": issuer.as_str(),
                "authorization_endpoint": "http://127.0.0.1:4200/authorize",
                "token_endpoint": "http://127.0.0.1:4200/token",
                "registration_endpoint": "http://127.0.0.1:4200/register",
                "device_authorization_endpoint": "http://127.0.0.1:4200/device",
                "revocation_endpoint": "http://127.0.0.1:4200/revoke",
                "code_challenge_methods_supported": ["S256"],
                "grant_types_supported": ["authorization_code", "urn:ietf:params:oauth:grant-type:device_code"],
                "client_id_metadata_document_supported": true
            }))
            .unwrap()
            .as_slice(),
            issuer,
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        assert!(metadata.supports_pkce_s256);
        assert!(metadata.supports_device_code);
        assert!(metadata.client_id_metadata_document_supported);

        let mut redirects = RedirectGuard::new(issuer, UrlPolicy::LoopbackFixture);
        for suffix in ["one", "two", "three", "four"] {
            redirects
                .follow(&format!("http://127.0.0.1:4200/{suffix}"))
                .unwrap();
        }
        assert!(redirects.follow("http://127.0.0.1:4200/five").is_err());
        let mut redirects = RedirectGuard::new(issuer, UrlPolicy::LoopbackFixture);
        assert!(redirects.follow("http://127.0.0.1:4300/escape").is_err());
        assert!(CheckedUrl::parse("https://user@example.com/x", UrlPolicy::Https).is_err());
        assert!(CheckedUrl::parse("https://example.com/x#fragment", UrlPolicy::Https).is_err());
        assert!(CheckedUrl::parse("http://example.com/x", UrlPolicy::Https).is_err());

        let resource_candidates = protected_resource_well_known(
            &loopback("http://127.0.0.1:4100/public/mcp?ignored=yes"),
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        assert_eq!(
            resource_candidates
                .iter()
                .map(CheckedUrl::as_str)
                .collect::<Vec<_>>(),
            [
                "http://127.0.0.1:4100/.well-known/oauth-protected-resource/public/mcp",
                "http://127.0.0.1:4100/.well-known/oauth-protected-resource"
            ]
        );
        let issuer_candidates = authorization_server_well_known(
            &loopback("http://127.0.0.1:4200/tenant1"),
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        assert_eq!(
            issuer_candidates
                .iter()
                .map(CheckedUrl::as_str)
                .collect::<Vec<_>>(),
            [
                "http://127.0.0.1:4200/.well-known/oauth-authorization-server/tenant1",
                "http://127.0.0.1:4200/.well-known/openid-configuration/tenant1",
                "http://127.0.0.1:4200/tenant1/.well-known/openid-configuration"
            ]
        );
    }

    #[test]
    fn issuer_matrix_and_scope_authority_are_exact() {
        let issuer = loopback("http://127.0.0.1:4200/issuer");
        assert!(
            validate_authorization_response_issuer(
                &issuer,
                true,
                Some(issuer.as_str()),
                UrlPolicy::LoopbackFixture
            )
            .is_ok()
        );
        assert!(
            validate_authorization_response_issuer(&issuer, true, None, UrlPolicy::LoopbackFixture)
                .is_err()
        );
        assert!(
            validate_authorization_response_issuer(
                &issuer,
                false,
                Some("http://127.0.0.1:4200/other"),
                UrlPolicy::LoopbackFixture
            )
            .is_err()
        );
        assert!(
            validate_authorization_response_issuer(
                &issuer,
                false,
                None,
                UrlPolicy::LoopbackFixture
            )
            .is_ok()
        );

        let ceiling = scopes(&["mcp.call", "mcp.read", "prior"]);
        let metadata = scopes(&["mcp.read", "metadata-only"]);
        let challenge = scopes(&["mcp.call"]);
        assert_eq!(
            select_initial_scopes(Some(&challenge), &metadata, Some(&ceiling)).unwrap(),
            Some(challenge.clone())
        );
        assert_eq!(
            select_initial_scopes(None, &metadata, Some(&ceiling)).unwrap(),
            Some(scopes(&["mcp.read"]))
        );
        assert!(
            select_initial_scopes(Some(&scopes(&["outside"])), &metadata, Some(&ceiling)).is_err()
        );
        assert_eq!(
            select_initial_scopes(Some(&challenge), &metadata, None).unwrap(),
            Some(challenge.clone())
        );
        assert_eq!(
            select_step_up_scopes(&scopes(&["prior"]), &challenge, Some(&ceiling)).unwrap(),
            scopes(&["mcp.call", "prior"])
        );
    }

    #[test]
    fn bearer_challenge_is_bounded_and_preserves_authoritative_scope() {
        let challenge = AuthorizationChallenge::parse(
            r#"Bearer resource_metadata="http://127.0.0.1:4100/.well-known/oauth-protected-resource", scope="files:read files:write", error="insufficient_scope""#,
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        assert_eq!(
            challenge.resource_metadata.as_str(),
            "http://127.0.0.1:4100/.well-known/oauth-protected-resource"
        );
        assert_eq!(
            challenge.scopes.unwrap(),
            scopes(&["files:read", "files:write"])
        );
        assert_eq!(challenge.error.as_deref(), Some("insufficient_scope"));
        for hostile in [
            "Basic value=x",
            "Bearer scope=files:read",
            "Bearer resource_metadata=\"https://example.test\", scope=\"a\", scope=\"b\"",
            "Bearer resource_metadata=\"https://example.test",
            "Bearer resource_metadata=\"https://example.test\" trailing",
        ] {
            assert!(AuthorizationChallenge::parse(hostile, UrlPolicy::Https).is_err());
        }
        assert!(
            AuthorizationChallenge::parse(
                &format!(
                    "Bearer resource_metadata=\"{}\"",
                    "a".repeat(MAX_CHALLENGE_BYTES)
                ),
                UrlPolicy::Https
            )
            .is_err()
        );
    }

    #[test]
    fn response_shapes_redact_secrets_and_reject_hostile_bounds() {
        let secret = "secret-value-that-must-not-render";
        let registration = RegistrationResponse::parse(
            serde_json::to_vec(&json!({"client_id":"client", "client_secret":secret}))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let token = TokenResponse::parse(
            serde_json::to_vec(&json!({
                "access_token": secret,
                "token_type": "Bearer",
                "expires_in": 300,
                "refresh_token": format!("refresh-{secret}"),
                "scope": "mcp.read"
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();
        let device = DeviceAuthorizationResponse::parse(
            serde_json::to_vec(&json!({
                "device_code": secret,
                "user_code": "ABCD-EFGH",
                "verification_uri": "http://127.0.0.1:4200/verify",
                "expires_in": 300,
                "interval": 5
            }))
            .unwrap()
            .as_slice(),
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        for rendered in [
            format!("{registration:?}"),
            format!("{token:?}"),
            format!("{device:?}"),
        ] {
            assert!(!rendered.contains(secret));
        }
        let protocol_error = OAuthErrorResponse::parse(
            serde_json::to_vec(&json!({
                "error": "authorization_pending",
                "error_description": format!("reflected {secret}"),
                "error_uri": format!("https://example.test/{secret}")
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();
        assert_eq!(protocol_error.code(), "authorization_pending");
        assert!(!format!("{protocol_error:?}").contains(secret));
        assert_eq!(token.access_token.expose(), secret);
        assert!(TokenResponse::parse(&vec![b' '; MAX_OAUTH_DOCUMENT_BYTES + 1]).is_err());
        assert!(TokenResponse::parse(br#"{"access_token":"x","token_type":"mac"}"#).is_err());
        assert!(DeviceAuthorizationResponse::parse(
            br#"{"device_code":"x","user_code":"x","verification_uri":"https://example.com","expires_in":3601}"#,
            UrlPolicy::Https
        )
        .is_err());
    }

    #[test]
    fn dynamic_registration_is_native_and_scope_bounds_are_finite() {
        let redirect = loopback("http://127.0.0.1:4100/callback");
        let request = DynamicRegistrationRequest::native(&redirect);
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["application_type"], "native");
        assert_eq!(json["redirect_uris"][0], redirect.as_str());

        let values = (0..=MAX_SCOPES)
            .map(|index| format!("scope-{index}"))
            .collect::<Vec<_>>();
        assert!(ScopeSet::from_values(values.iter().map(String::as_str)).is_err());
        assert!(ScopeSet::from_values(["duplicate", "duplicate"]).is_err());
    }

    type ObservedRequest = (Method, String, HeaderMap, String);

    #[derive(Clone)]
    struct GrantFixture {
        replies: Arc<Mutex<VecDeque<(StatusCode, serde_json::Value)>>>,
        requests: Arc<Mutex<Vec<ObservedRequest>>>,
    }

    async fn grant_reply(
        State(state): State<GrantFixture>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        state.requests.lock().await.push((
            method,
            uri.path().to_owned(),
            headers,
            String::from_utf8(body.to_vec()).unwrap(),
        ));
        let (status, body) = state.replies.lock().await.pop_front().unwrap();
        (status, Json(body)).into_response()
    }

    #[tokio::test]
    async fn native_grants_are_single_dispatch_resource_bound_and_server_paced() {
        let replies: Arc<Mutex<VecDeque<(StatusCode, serde_json::Value)>>> = Arc::new(Mutex::new(
            vec![
                (
                    StatusCode::CREATED,
                    json!({"client_id":"dynamic-client","client_secret":"dynamic-secret"}),
                ),
                (
                    StatusCode::OK,
                    json!({"access_token":"code-access","token_type":"Bearer","expires_in":300,"refresh_token":"code-refresh","scope":"mcp.read"}),
                ),
                (
                    StatusCode::OK,
                    json!({"device_code":"device-private","user_code":"ABCD-EFGH","verification_uri":"PLACEHOLDER","expires_in":30,"interval":1}),
                ),
                (
                    StatusCode::OK,
                    json!({"access_token":"device-access","token_type":"Bearer","expires_in":300,"refresh_token":"device-refresh","scope":"mcp.read"}),
                ),
                (
                    StatusCode::OK,
                    json!({"access_token":"rotated-access","token_type":"Bearer","expires_in":300,"refresh_token":"rotated-refresh","scope":"mcp.read"}),
                ),
                (StatusCode::BAD_REQUEST, json!({"error":"invalid_token"})),
            ]
            .into(),
        ));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        replies.lock().await[2].1["verification_uri"] = json!(format!("{base}/verify"));
        let task = tokio::spawn({
            let state = GrantFixture {
                replies: replies.clone(),
                requests: requests.clone(),
            };
            async move {
                axum::serve(
                    socket,
                    Router::new().fallback(any(grant_reply)).with_state(state),
                )
                .await
                .unwrap();
            }
        });
        let issuer = loopback(&format!("{base}/issuer"));
        let metadata = AuthorizationServerMetadata::parse(
            &serde_json::to_vec(&json!({
                "issuer": issuer.as_str(),
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
                "registration_endpoint": format!("{base}/register"),
                "device_authorization_endpoint": format!("{base}/device"),
                "revocation_endpoint": format!("{base}/revoke"),
                "code_challenge_methods_supported": ["S256"],
                "grant_types_supported": ["authorization_code", "urn:ietf:params:oauth:grant-type:device_code"]
            }))
            .unwrap(),
            &issuer,
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        let resource = loopback(&format!("{base}/mcp"));
        let redirect = loopback("http://127.0.0.1:48888/oauth/callback");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let registration = register_dynamic_client(
            &client,
            metadata.registration_endpoint.as_ref().unwrap(),
            &redirect,
        )
        .await
        .unwrap();
        let client_secret = registration.client_secret.as_ref().unwrap();
        let code = AuthorizationCode {
            code: SecretText::new("authorization code", "accepted-code".into()).unwrap(),
            verifier: SecretText::new("PKCE verifier", "v".repeat(64)).unwrap(),
            redirect_uri: redirect.as_str().to_owned(),
        };
        let token = exchange_authorization_code(
            &client,
            &metadata,
            &registration.client_id,
            Some(client_secret),
            &resource,
            &code,
        )
        .await
        .unwrap();
        assert_eq!(token.access_token.expose(), "code-access");
        let selected = scopes(&["mcp.read"]);
        let device = begin_device_authorization(
            &client,
            &metadata,
            &registration.client_id,
            Some(client_secret),
            &resource,
            Some(&selected),
            UrlPolicy::LoopbackFixture,
        )
        .await
        .unwrap();
        let device_token = finish_device_authorization(
            &client,
            &metadata,
            &registration.client_id,
            Some(client_secret),
            &resource,
            &device,
        )
        .await
        .unwrap();
        assert_eq!(device_token.access_token.expose(), "device-access");
        let refresh = refresh_access_token(
            &client,
            &metadata,
            &registration.client_id,
            Some(client_secret),
            &resource,
            token.refresh_token.as_ref().unwrap(),
            Some(&selected),
        )
        .await
        .unwrap();
        assert_eq!(refresh.access_token.expose(), "rotated-access");
        assert_eq!(
            revoke_remote(
                &client,
                &metadata,
                &registration.client_id,
                Some(client_secret),
                refresh.refresh_token.as_ref().unwrap()
            )
            .await,
            RevocationOutcome::RemoteRefused
        );

        let requests = requests.lock().await;
        assert_eq!(
            requests
                .iter()
                .map(|(_, path, _, _)| path.as_str())
                .collect::<Vec<_>>(),
            [
                "/register",
                "/token",
                "/device",
                "/token",
                "/token",
                "/revoke"
            ]
        );
        assert!(
            requests[0].2[CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("application/json")
        );
        let forms = requests[1..]
            .iter()
            .map(|(_, _, headers, body)| {
                assert_eq!(
                    headers[CONTENT_TYPE].to_str().unwrap(),
                    "application/x-www-form-urlencoded"
                );
                url::form_urlencoded::parse(body.as_bytes())
                    .into_owned()
                    .collect::<HashMap<_, _>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(forms[0]["resource"], resource.as_str());
        assert_eq!(forms[0]["code_verifier"], "v".repeat(64));
        assert_eq!(forms[1]["scope"], "mcp.read");
        assert_eq!(
            forms[2]["grant_type"],
            "urn:ietf:params:oauth:grant-type:device_code"
        );
        assert_eq!(forms[3]["refresh_token"], "code-refresh");
        assert_eq!(forms[4]["token"], "rotated-refresh");
        assert!(requests.iter().all(|(_, _, headers, _)| {
            !headers.contains_key("authorization") && !headers.contains_key("proxy-authorization")
        }));
        drop(requests);
        task.abort();
    }

    #[tokio::test]
    async fn rotating_refresh_lost_response_is_returned_without_replay() {
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let task = tokio::spawn({
            let requests = requests.clone();
            async move {
                let app = Router::new().route(
                    "/token",
                    post(move |body: String| {
                        let requests = requests.clone();
                        async move {
                            requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            assert!(body.contains("refresh_token=rotating-secret"));
                            let failed = futures::stream::once(async {
                                Err::<Bytes, std::io::Error>(std::io::Error::new(
                                    std::io::ErrorKind::ConnectionReset,
                                    "fixture dropped the accepted refresh response",
                                ))
                            });
                            Response::new(Body::from_stream(failed))
                        }
                    }),
                );
                axum::serve(socket, app).await.unwrap();
            }
        });
        let issuer = loopback(&format!("{base}/issuer"));
        let metadata = AuthorizationServerMetadata::parse(
            &serde_json::to_vec(&json!({
                "issuer": issuer.as_str(),
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
                "code_challenge_methods_supported": ["S256"]
            }))
            .unwrap(),
            &issuer,
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        let resource = loopback(&format!("{base}/mcp"));
        let token = SecretText::new("refresh token", "rotating-secret".into()).unwrap();
        let error = refresh_access_token(
            &reqwest::Client::new(),
            &metadata,
            "native-client",
            None,
            &resource,
            &token,
            None,
        )
        .await
        .unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains("dispatch MCP OAuth token request")
                || diagnostic.contains("read OAuth response body"),
            "{diagnostic}"
        );
        assert_eq!(requests.load(std::sync::atomic::Ordering::Relaxed), 1);
        task.abort();
    }

    #[tokio::test]
    async fn device_polling_honors_pending_slow_down_denial_expiry_and_cancellation() {
        let replies: Arc<Mutex<VecDeque<(StatusCode, serde_json::Value)>>> = Arc::new(Mutex::new(
            vec![
                (
                    StatusCode::BAD_REQUEST,
                    json!({"error":"authorization_pending"}),
                ),
                (StatusCode::BAD_REQUEST, json!({"error":"slow_down"})),
                (
                    StatusCode::OK,
                    json!({"access_token":"device-access","token_type":"Bearer","expires_in":300,"refresh_token":"device-refresh","scope":"mcp.read"}),
                ),
                (StatusCode::BAD_REQUEST, json!({"error":"access_denied"})),
                (StatusCode::BAD_REQUEST, json!({"error":"expired_token"})),
            ]
            .into(),
        ));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let task = tokio::spawn({
            let replies = replies.clone();
            let requests = requests.clone();
            async move {
                let app = Router::new().route(
                    "/token",
                    post(move |body: String| {
                        let replies = replies.clone();
                        let requests = requests.clone();
                        async move {
                            requests.lock().await.push((Instant::now(), body));
                            let (status, body) = replies.lock().await.pop_front().unwrap();
                            (status, Json(body))
                        }
                    }),
                );
                axum::serve(socket, app).await.unwrap();
            }
        });
        let issuer = loopback(&format!("{base}/issuer"));
        let metadata = AuthorizationServerMetadata::parse(
            &serde_json::to_vec(&json!({
                "issuer": issuer.as_str(),
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
                "device_authorization_endpoint": format!("{base}/device"),
                "code_challenge_methods_supported": ["S256"],
                "grant_types_supported": ["urn:ietf:params:oauth:grant-type:device_code"]
            }))
            .unwrap(),
            &issuer,
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();
        let resource = loopback(&format!("{base}/mcp"));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let device = DeviceAuthorizationResponse::parse(
            &serde_json::to_vec(&json!({
                "device_code": "synthetic-device-code",
                "user_code": "ABCD-EFGH",
                "verification_uri": format!("{base}/verify"),
                "expires_in": 30,
                "interval": 1
            }))
            .unwrap(),
            UrlPolicy::LoopbackFixture,
        )
        .unwrap();

        let token = finish_device_authorization(
            &client,
            &metadata,
            "native-client",
            None,
            &resource,
            &device,
        )
        .await
        .unwrap();
        assert_eq!(token.access_token.expose(), "device-access");
        let requests_after_success = requests.lock().await;
        assert_eq!(requests_after_success.len(), 3);
        assert!(
            requests_after_success[1]
                .0
                .duration_since(requests_after_success[0].0)
                >= Duration::from_secs(1)
        );
        assert!(
            requests_after_success[2]
                .0
                .duration_since(requests_after_success[1].0)
                >= Duration::from_secs(6)
        );
        assert!(requests_after_success.iter().all(|(_, body)| {
            body.contains("device_code=synthetic-device-code") && body.contains("resource=")
        }));
        drop(requests_after_success);

        let denied = finish_device_authorization(
            &client,
            &metadata,
            "native-client",
            None,
            &resource,
            &device,
        )
        .await
        .unwrap_err();
        assert!(denied.to_string().contains("declined"));
        let expired = finish_device_authorization(
            &client,
            &metadata,
            "native-client",
            None,
            &resource,
            &device,
        )
        .await
        .unwrap_err();
        assert!(expired.to_string().contains("expired"));
        let before_cancel = requests.lock().await.len();
        let cancelled = finish_device_authorization_with_cancellation(
            &client,
            &metadata,
            "native-client",
            None,
            &resource,
            &device,
            std::future::ready(Ok(())),
        )
        .await
        .unwrap_err();
        assert!(cancelled.to_string().contains("cancelled"));
        assert_eq!(requests.lock().await.len(), before_cancel);

        let mut unsupported = metadata.clone();
        unsupported.supports_device_code = false;
        assert!(
            begin_device_authorization(
                &client,
                &unsupported,
                "native-client",
                None,
                &resource,
                None,
                UrlPolicy::LoopbackFixture,
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("does not advertise")
        );
        assert_eq!(requests.lock().await.len(), before_cancel);
        assert!(replies.lock().await.is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn revocation_distinguishes_absent_lost_and_hostile_remote_outcomes() {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let token = SecretText::new("revocation token", "synthetic-refresh".into()).unwrap();
        let metadata = authorization_metadata("http://127.0.0.1:4200");
        assert_eq!(
            revoke_remote(&client, &metadata, "native-client", None, &token).await,
            RevocationOutcome::NoAdvertisedEndpoint
        );

        let mut lost = metadata.clone();
        lost.revocation_endpoint = Some(loopback("http://127.0.0.1:1/revoke"));
        assert_eq!(
            revoke_remote(&client, &lost, "native-client", None, &token).await,
            RevocationOutcome::RemoteOutcomeUncertain
        );

        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/revoke", socket.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let app = Router::new().route(
                "/revoke",
                post(|| async { vec![b'x'; MAX_OAUTH_DOCUMENT_BYTES + 1] }),
            );
            axum::serve(socket, app).await.unwrap();
        });
        let mut hostile = metadata;
        hostile.revocation_endpoint = Some(loopback(&endpoint));
        assert_eq!(
            revoke_remote(&client, &hostile, "native-client", None, &token).await,
            RevocationOutcome::RemoteOutcomeUncertain
        );
        task.abort();
    }
}
