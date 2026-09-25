//! Synthetic TLS MCP server shared by CLI and real-PTY integration fixtures.
//! Its CA is trusted only by the owned test child through the test-support seam.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(crate) struct HttpsMcpFixture {
    pub(crate) base: String,
    pub(crate) ca_path: PathBuf,
    #[allow(
        dead_code,
        reason = "only the trust integration binary counts every request"
    )]
    requests: Arc<AtomicUsize>,
    pub(crate) device_requests: Arc<AtomicUsize>,
    pub(crate) token_requests: Arc<AtomicUsize>,
    #[allow(
        dead_code,
        reason = "only the Linux unavailable-store fixture uses the static alias"
    )]
    pub(crate) static_requests: Arc<AtomicUsize>,
    #[allow(
        dead_code,
        reason = "only the Linux unavailable-store fixture checks static header isolation"
    )]
    pub(crate) static_bad_headers: Arc<AtomicUsize>,
    token_forms: Arc<std::sync::Mutex<Vec<std::collections::BTreeMap<String, String>>>>,
    pub(crate) revocations: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for HttpsMcpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl HttpsMcpFixture {
    pub(crate) async fn start(root: &Path) -> Self {
        use rcgen::{
            BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType,
            ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
        };
        use tokio_rustls::rustls::{
            ServerConfig,
            pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
        };

        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Kuru synthetic MCP test CA");
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let issuer = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
        let mut leaf_params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        leaf_params.distinguished_name = DistinguishedName::new();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "localhost");
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let leaf_key = KeyPair::generate().unwrap();
        let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
        let ca_path = root.join("synthetic-mcp-ca.pem");
        std::fs::write(&ca_path, issuer.pem()).unwrap();
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
            )
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("https://localhost:{port}");
        let requests = Arc::new(AtomicUsize::new(0));
        let device_requests = Arc::new(AtomicUsize::new(0));
        let token_requests = Arc::new(AtomicUsize::new(0));
        let static_requests = Arc::new(AtomicUsize::new(0));
        let static_bad_headers = Arc::new(AtomicUsize::new(0));
        let token_forms = Arc::new(std::sync::Mutex::new(Vec::new()));
        let revocations = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let base = base.clone();
            let requests = Arc::clone(&requests);
            let device_requests = Arc::clone(&device_requests);
            let token_requests = Arc::clone(&token_requests);
            let static_requests = Arc::clone(&static_requests);
            let static_bad_headers = Arc::clone(&static_bad_headers);
            let token_forms = Arc::clone(&token_forms);
            let revocations = Arc::clone(&revocations);
            async move {
                loop {
                    let (socket, _) = listener.accept().await.unwrap();
                    let acceptor = acceptor.clone();
                    let base = base.clone();
                    let requests = Arc::clone(&requests);
                    let device_requests = Arc::clone(&device_requests);
                    let token_requests = Arc::clone(&token_requests);
                    let static_requests = Arc::clone(&static_requests);
                    let static_bad_headers = Arc::clone(&static_bad_headers);
                    let token_forms = Arc::clone(&token_forms);
                    let revocations = Arc::clone(&revocations);
                    tokio::spawn(async move {
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        let Ok(Ok(mut stream)) = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            acceptor.accept(socket),
                        )
                        .await
                        else {
                            return;
                        };
                        let mut request = Vec::new();
                        let headers_end = loop {
                            let mut chunk = [0u8; 1024];
                            let Ok(Ok(count)) = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                stream.read(&mut chunk),
                            )
                            .await
                            else {
                                return;
                            };
                            if count == 0 || request.len() + count > 16 * 1024 {
                                return;
                            }
                            request.extend_from_slice(&chunk[..count]);
                            if let Some(end) =
                                request.windows(4).position(|part| part == b"\r\n\r\n")
                            {
                                break end + 4;
                            }
                        };
                        let headers = String::from_utf8_lossy(&request[..headers_end]).into_owned();
                        let Some(first) = headers.lines().next() else {
                            return;
                        };
                        let mut parts = first.split_whitespace();
                        let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
                            return;
                        };
                        requests.fetch_add(1, Ordering::Relaxed);
                        if path == "/static-mcp" {
                            static_requests.fetch_add(1, Ordering::Relaxed);
                            let mut authorization = Vec::new();
                            let mut proxy_authorization = false;
                            for line in headers.lines() {
                                if let Some((name, value)) = line.split_once(':') {
                                    if name.eq_ignore_ascii_case("authorization") {
                                        authorization.push(value.trim());
                                    }
                                    proxy_authorization |=
                                        name.eq_ignore_ascii_case("proxy-authorization");
                                }
                            }
                            if authorization != ["Bearer synthetic-static-proof"]
                                || proxy_authorization
                            {
                                static_bad_headers.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if length > 8 * 1024 {
                            return;
                        }
                        while request.len() - headers_end < length {
                            let mut chunk = [0u8; 1024];
                            let Ok(Ok(count)) = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                stream.read(&mut chunk),
                            )
                            .await
                            else {
                                return;
                            };
                            if count == 0 || request.len() + count > 24 * 1024 {
                                return;
                            }
                            request.extend_from_slice(&chunk[..count]);
                        }
                        let payload = &request[headers_end..headers_end + length];
                        let mut extra = String::new();
                        let (status, body) = match (method, path) {
                            ("GET", "/mcp") => {
                                extra = format!(
                                    "WWW-Authenticate: Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\", scope=\"mcp.read\"\r\n"
                                );
                                (401, serde_json::json!({"error":"unauthorized"}))
                            }
                            ("GET", "/.well-known/oauth-protected-resource/mcp") => (
                                200,
                                serde_json::json!({"resource":format!("{base}/mcp"),"authorization_servers":[format!("{base}/")],"scopes_supported":["mcp.read"]}),
                            ),
                            ("GET", "/.well-known/oauth-authorization-server") => (
                                200,
                                serde_json::json!({"issuer":format!("{base}/"),"authorization_endpoint":format!("{base}/authorize"),"token_endpoint":format!("{base}/token"),"device_authorization_endpoint":format!("{base}/device"),"revocation_endpoint":format!("{base}/revoke"),"grant_types_supported":["authorization_code","urn:ietf:params:oauth:grant-type:device_code","refresh_token"],"code_challenge_methods_supported":["S256"],"authorization_response_iss_parameter_supported":true}),
                            ),
                            ("POST", "/device") => {
                                device_requests.fetch_add(1, Ordering::Relaxed);
                                (
                                    200,
                                    serde_json::json!({"device_code":"synthetic-device","user_code":"TEST-CODE","verification_uri":format!("{base}/verify"),"expires_in":120,"interval":1}),
                                )
                            }
                            ("POST", "/token") => {
                                let body = std::str::from_utf8(payload).unwrap();
                                let form = reqwest::Url::parse(&format!("http://fixture/?{body}"))
                                    .unwrap()
                                    .query_pairs()
                                    .into_owned()
                                    .collect::<std::collections::BTreeMap<_, _>>();
                                token_forms.lock().unwrap().push(form);
                                token_requests.fetch_add(1, Ordering::Relaxed);
                                (
                                    200,
                                    serde_json::json!({"access_token":"synthetic-mcp-access","refresh_token":"synthetic-mcp-refresh","token_type":"Bearer","expires_in":120,"scope":"mcp.read"}),
                                )
                            }
                            ("POST", "/revoke") => {
                                revocations.fetch_add(1, Ordering::Relaxed);
                                (200, serde_json::json!({}))
                            }
                            ("POST", "/mcp") => {
                                let parsed: Value = serde_json::from_slice(payload).unwrap();
                                let result = match parsed["method"].as_str() {
                                    Some("initialize") => {
                                        serde_json::json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}}})
                                    }
                                    Some("tools/list") => serde_json::json!({"tools":[]}),
                                    Some("notifications/initialized") => serde_json::json!({}),
                                    _ => serde_json::json!({}),
                                };
                                (
                                    200,
                                    serde_json::json!({"jsonrpc":"2.0","id":parsed["id"],"result":result}),
                                )
                            }
                            ("POST", "/static-mcp") => {
                                let parsed: Value = serde_json::from_slice(payload).unwrap();
                                let result = match parsed["method"].as_str() {
                                    Some("initialize") => {
                                        serde_json::json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}}})
                                    }
                                    Some("tools/list") => {
                                        serde_json::json!({"tools":[{"name":"static-proof","description":"synthetic static route","inputSchema":{"type":"object"}}]})
                                    }
                                    _ => serde_json::json!({}),
                                };
                                (
                                    200,
                                    serde_json::json!({"jsonrpc":"2.0","id":parsed["id"],"result":result}),
                                )
                            }
                            _ => (404, serde_json::json!({"error":"not found"})),
                        };
                        let body = body.to_string();
                        let response = format!(
                            "HTTP/1.1 {status} Synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });
        Self {
            base,
            ca_path,
            requests,
            device_requests,
            token_requests,
            static_requests,
            static_bad_headers,
            token_forms,
            revocations,
            task,
        }
    }

    pub(crate) fn token_forms(&self) -> Vec<std::collections::BTreeMap<String, String>> {
        self.token_forms.lock().unwrap().clone()
    }

    #[allow(
        dead_code,
        reason = "only the trust integration binary checks pretrust traffic"
    )]
    pub(crate) fn requests(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }
}
