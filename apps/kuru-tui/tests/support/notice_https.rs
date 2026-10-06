//! Isolated release-manifest TLS peer. Requests and held-response EOF are
//! observed causally; no fixture environment is installed in the test runner.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Notify,
    task::JoinHandle,
};

pub struct NoticeHttps {
    pub base: String,
    pub ca: PathBuf,
    requests: Arc<Mutex<Vec<String>>>,
    pub held: Arc<Notify>,
    pub disconnected: Arc<Notify>,
    task: JoinHandle<()>,
}

impl NoticeHttps {
    pub async fn start(root: &Path) -> Result<Self> {
        use rcgen::{
            BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair, KeyUsagePurpose,
        };
        use tokio_rustls::rustls::{
            ServerConfig,
            pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
        };
        let mut ca = CertificateParams::default();
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let issuer = CertifiedIssuer::self_signed(ca, KeyPair::generate()?)?;
        let key = KeyPair::generate()?;
        let leaf = CertificateParams::new(vec!["localhost".into()])?.signed_by(&key, &issuer)?;
        let ca = root.join("notice-fixture-ca.pem");
        std::fs::write(&ca, issuer.pem())?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![leaf.der().clone()],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
                )?,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("https://localhost:{}", listener.local_addr()?.port());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let held = Arc::new(Notify::new());
        let disconnected = Arc::new(Notify::new());
        let task = tokio::spawn({
            let (requests, held, disconnected, base) = (
                requests.clone(),
                held.clone(),
                disconnected.clone(),
                base.clone(),
            );
            async move {
                // Child sockets are owned by this finite server and abort with it.
                let mut children = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let Ok((socket, _)) = accepted else { break };
                            let (acceptor, requests, held, disconnected, base) = (acceptor.clone(), requests.clone(), held.clone(), disconnected.clone(), base.clone());
                            children.spawn(async move {
                                let Ok(Ok(mut stream)) = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(socket)).await else { return };
                                let mut request = Vec::new();
                                loop {
                                    let mut bytes = [0u8; 1024];
                                    let Ok(Ok(count)) = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut bytes)).await else { return };
                                    if count == 0 || request.len() + count > 8192 { return }
                                    request.extend_from_slice(&bytes[..count]);
                                    if request.windows(4).any(|part| part == b"\r\n\r\n") { break }
                                }
                                let request = String::from_utf8_lossy(&request).into_owned();
                                let path = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("").to_owned();
                                {
                                    let mut rows = requests.lock().unwrap();
                                    if rows.len() >= 32 { return }
                                    rows.push(request);
                                }
                                if path == "/held" {
                                    held.notify_one();
                                    let mut byte = [0u8; 1];
                                    // EOF proves the advisory client released its owned response.
                                    if matches!(tokio::time::timeout(Duration::from_secs(10), stream.read(&mut byte)).await, Ok(Ok(0)) | Ok(Err(_))) {
                                        disconnected.notify_one();
                                    }
                                    return;
                                }
                                let target = kuru_delivery::archive::host_target().unwrap();
                                let manifest = format!("{}  kuru-999.0.0-{target}.tar.gz\n", "a".repeat(64));
                                let (status, extra, body) = match path.as_str() {
                                    "/manifest" | "/signed?token=fixture" => (200, String::new(), manifest),
                                    "/redirect" => (302, format!("Location: {base}/signed?token=fixture\r\n"), String::new()),
                                    "/loop" => (302, format!("Location: {base}/loop\r\n"), String::new()),
                                    "/downgrade" => (302, "Location: http://127.0.0.1:9/forbidden\r\n".into(), String::new()),
                                    "/oversize" => (200, String::new(), "x".repeat(64 * 1024 + 1)),
                                    "/malformed" => (200, String::new(), "PRIVATE_HOSTILE\u{1b}[31m".into()),
                                    _ => (503, String::new(), "PRIVATE_ERROR_BODY".into()),
                                };
                                let reply = if path == "/oversize" {
                                    // No Content-Length: the production streamed-byte bound
                                    // must reject the response, not just its header.
                                    format!("HTTP/1.1 200 Fixture\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len())
                                } else {
                                    format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}", body.len())
                                };
                                let _ = stream.write_all(reply.as_bytes()).await;
                                let _ = stream.shutdown().await;
                            });
                        }
                        _ = children.join_next(), if !children.is_empty() => {}
                    }
                }
            }
        });
        Ok(Self {
            base,
            ca,
            requests,
            held,
            disconnected,
            task,
        })
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    pub async fn close(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for NoticeHttps {
    fn drop(&mut self) {
        self.task.abort();
    }
}
