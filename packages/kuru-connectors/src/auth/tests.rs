use super::*;
use axum::{
    Router,
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::any,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::Notify,
};

pub(super) fn jwt(account: &str, expiry: u64) -> String {
    format!(
        "e30.{}.c2ln",
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(
                &json!({"exp":expiry,"https://api.openai.com/auth":{"chatgpt_account_id":account}})
            )
            .unwrap()
        )
    )
}
fn tokens(account: &str) -> Value {
    json!({"access_token":jwt(account, now().unwrap()+3600),"id_token":jwt(account, now().unwrap()+3600),"refresh_token":"synthetic-refresh"})
}
struct Reply {
    status: StatusCode,
    body: String,
    gate: Option<Arc<Notify>>,
}
impl Reply {
    fn json(body: Value) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.to_string(),
            gate: None,
        }
    }
}
struct Request {
    path: String,
    headers: HeaderMap,
    body: String,
    at: std::time::Instant,
    method: Method,
}
struct ServerState {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Request>>,
    arrived: Notify,
}
struct Fixture {
    temp: tempfile::TempDir,
    state: Arc<ServerState>,
    server: tokio::task::JoinHandle<()>,
    manager: AuthManager,
}
impl Fixture {
    async fn new(replies: Vec<Reply>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let state = Arc::new(ServerState {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
            arrived: Notify::new(),
        });
        let router = Router::new()
            .fallback(any(handle))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let manager = AuthManager::test_issuer(temp.path().join("data"), root, &issuer).unwrap();
        Self {
            temp,
            state,
            server,
            manager,
        }
    }
    async fn arrived(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.state.arrived.notified();
                if self.state.requests.lock().unwrap().len() >= count {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("auth fixture request did not arrive");
    }
    fn count(&self) -> usize {
        self.state.requests.lock().unwrap().len()
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(self.manager.inner.store.path().join("credentials.json")).unwrap()
    }
    async fn seed(&self) {
        self.manager
            .seed_test_session("old-access", "old-refresh", "account-one")
            .await
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn handle(
    State(state): State<Arc<ServerState>>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    state.requests.lock().unwrap().push(Request {
        path: uri.to_string(),
        method,
        headers,
        body: String::from_utf8(body.to_vec()).unwrap(),
        at: std::time::Instant::now(),
    });
    let reply = state
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .expect("unexpected auth fixture request");
    state.arrived.notify_waiters();
    if let Some(gate) = reply.gate {
        gate.notified().await;
    }
    (
        reply.status,
        [("Content-Type", "application/json")],
        reply.body,
    )
}
fn callback(login: &BrowserLogin) -> (u16, String, HashMap<String, String>) {
    let url = url::Url::parse(login.authorization_url()).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let redirect = url::Url::parse(&query["redirect_uri"]).unwrap();
    (redirect.port().unwrap(), query["state"].clone(), query)
}
async fn callback_request(port: u16, target: &str, method: &str, host: &str) -> String {
    let mut socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    socket
        .write_all(
            format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(5), socket.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    response
}

async fn clean_up_refresh_after_gate_timeout(
    task: tokio::task::JoinHandle<Result<RequestCredentials>>,
    manager: &AuthManager,
) -> String {
    task.abort();
    let task = match task.await {
        Err(error) if error.is_cancelled() => "caller cancelled",
        Err(_) => "caller join failed",
        Ok(Ok(_)) => "caller unexpectedly succeeded",
        Ok(Err(_)) => "caller returned an error",
    };
    let owner = match manager
        .inner
        .store
        .lease(false, Duration::from_secs(10))
        .await
    {
        Ok(Some(lease)) => {
            drop(lease);
            "owner released its lease"
        }
        Ok(None) => "credential store disappeared",
        Err(_) => "owner did not release its lease",
    };
    format!("{task}; {owner}")
}

#[tokio::test]
async fn native_auth_status_and_api_key_have_no_fresh_store_effects() {
    let fixture = Fixture::new(vec![]).await;
    assert!(!fixture.manager.status().await.unwrap().authenticated);
    assert!(!fixture.temp.path().join("data").exists());
    let key = AuthManager::new(
        fixture.temp.path().join("data"),
        fixture.temp.path().join("project"),
        Some("fake-api-key".into()),
    )
    .unwrap();
    let selected = key.credentials(AuthRoute::ApiKey).await.unwrap();
    assert_eq!(selected.bearer(), "fake-api-key");
    assert_eq!(selected.route(), AuthRoute::ApiKey);
    assert!(selected.account_id().is_none());
    assert!(selected.session_id().is_none());
    assert!(!fixture.temp.path().join("data").exists());
    key.logout().await.unwrap();
    assert!(!key.status().await.unwrap().authenticated);
    assert!(
        !String::from_utf8(fixture.bytes())
            .unwrap()
            .contains("fake-api-key")
    );
    let bad = AuthManager::new(
        fixture.temp.path().join("project/state"),
        fixture.temp.path().join("project"),
        None,
    )
    .unwrap();
    assert!(
        bad.status()
            .await
            .unwrap_err()
            .to_string()
            .contains("outside the tool root")
    );
    assert!(!fixture.temp.path().join("project/state").exists());
}

#[tokio::test]
async fn native_auth_browser_exchanges_exact_pkce_and_publishes_private_session() {
    let fixture = Fixture::new(vec![Reply::json(tokens("account-one"))]).await;
    let login = fixture.manager.begin_browser().await.unwrap();
    let (port, state, query) = callback(&login);
    assert_eq!(query["scope"], "openid profile email offline_access");
    assert_eq!(query["client_id"], CLIENT_ID);
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["originator"], "kuru");
    assert_eq!(URL_SAFE_NO_PAD.decode(&state).unwrap().len(), 32);
    let task = tokio::spawn(login.finish());
    let response = callback_request(
        port,
        &format!("/auth/callback?code=auth%2Bcode&state={state}"),
        "GET",
        &format!("localhost:{port}"),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200"));
    let status = task.await.unwrap().unwrap();
    assert!(status.authenticated);
    assert_eq!(status.account_id.as_deref(), Some("account-one"));
    assert!(!format!("{status:?}").contains("synthetic-refresh"));
    let requests = fixture.state.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.path, "/oauth/token");
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    let form: HashMap<_, _> = url::form_urlencoded::parse(request.body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "auth+code");
    assert_eq!(form["redirect_uri"], query["redirect_uri"]);
    assert_eq!(
        URL_SAFE_NO_PAD
            .decode(&form["code_verifier"])
            .unwrap()
            .len(),
        64
    );
    use sha2::Digest;
    assert_eq!(
        URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(form["code_verifier"].as_bytes())),
        query["code_challenge"]
    );
    let directory = kuru_platform::fs::Directory::open(
        fixture.manager.inner.store.path(),
        kuru_platform::fs::Privacy::OwnerOnly,
        kuru_platform::fs::NameRetention::Movable,
    )
    .unwrap();
    let file = directory
        .read(std::ffi::OsStr::new("credentials.json"))
        .unwrap();
    kuru_platform::fs::require_private(&file).unwrap();
    directory
        .verify(std::ffi::OsStr::new("credentials.json"), &file)
        .unwrap();
}

#[tokio::test]
async fn native_auth_invalid_callbacks_and_cancellation_preserve_old_session() {
    let fixture = Fixture::new(vec![]).await;
    fixture.seed().await;
    let original = fixture.bytes();
    let login = fixture.manager.begin_browser().await.unwrap();
    let (port, state, _) = callback(&login);
    let task = tokio::spawn(login.finish());
    let good_host = format!("localhost:{port}");
    let good_target = format!("/auth/callback?code=fake&state={state}");
    for (target, method, host) in [
        (
            "/auth/callback?code=fake&state=wrong".into(),
            "GET",
            good_host.as_str(),
        ),
        (
            format!("{good_target}&code=duplicate"),
            "GET",
            good_host.as_str(),
        ),
        (
            format!("{good_target}&state={state}"),
            "GET",
            good_host.as_str(),
        ),
        (good_target.clone(), "POST", good_host.as_str()),
        (good_target.clone(), "GET", "evil.invalid"),
        (
            format!("/different?code=fake&state={state}"),
            "GET",
            good_host.as_str(),
        ),
    ] {
        assert!(
            callback_request(port, &target, method, host)
                .await
                .starts_with("HTTP/1.1 400")
        );
        assert_eq!(fixture.bytes(), original);
        assert_eq!(fixture.count(), 0);
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let rebound = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    drop(rebound);
    assert_eq!(fixture.bytes(), original);
}

#[tokio::test]
async fn native_auth_browser_busy_port_expiry_and_inflight_cancel_are_owned() {
    let gate = Arc::new(Notify::new());
    let fixture = Fixture::new(vec![Reply {
        gate: Some(gate.clone()),
        ..Reply::json(tokens("account-one"))
    }])
    .await;
    fixture.seed().await;
    let original = fixture.bytes();
    let login = fixture.manager.begin_browser().await.unwrap();
    let (port, state, _) = callback(&login);
    let mut other = AuthManager::new(
        fixture.temp.path().join("data"),
        fixture.temp.path().join("project"),
        None,
    )
    .unwrap();
    Arc::get_mut(&mut other.inner).unwrap().callback_port = port;
    let error = other.begin_browser().await.err().unwrap();
    assert!(error.to_string().contains("--device"));
    let task = tokio::spawn(login.finish());
    let client = tokio::spawn(async move {
        callback_request(
            port,
            &format!("/auth/callback?code=valid&state={state}"),
            "GET",
            &format!("localhost:{port}"),
        )
        .await
    });
    fixture.arrived(1).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let _ = client.await.unwrap();
    gate.notify_one();
    assert_eq!(fixture.bytes(), original);
    let rebound = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    drop(rebound);
    let mut short = AuthManager::test_issuer(
        fixture.temp.path().join("data"),
        fixture.temp.path().join("project"),
        &fixture.manager.inner.issuer,
    )
    .unwrap();
    Arc::get_mut(&mut short.inner).unwrap().login_timeout = Duration::from_millis(80);
    let expired = short.begin_browser().await.unwrap();
    let (port, _, _) = callback(&expired);
    let mut stalled = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    stalled.write_all(b"GET /auth").await.unwrap();
    assert!(
        expired
            .finish()
            .await
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
    assert_eq!(fixture.bytes(), original);
    tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
}

#[tokio::test]
async fn native_auth_device_paces_pending_response_then_exchanges_code() {
    let fixture = Fixture::new(vec![
        Reply::json(json!({"device_auth_id":"device-id","user_code":"ABCD-EFGH","interval":"1","expires_in":30})),
        Reply { status: StatusCode::FORBIDDEN, ..Reply::json(json!({"error":"pending"})) },
        Reply::json(json!({"authorization_code":"device-auth-code","code_verifier":"device-verifier"})),
        Reply::json(tokens("account-one")),
    ]).await;
    let login = fixture.manager.begin_device().await.unwrap();
    assert_eq!(login.user_code(), "ABCD-EFGH");
    assert_eq!(
        login.verification_url(),
        format!("{}/codex/device", fixture.manager.inner.issuer)
    );
    assert!(login.finish().await.unwrap().authenticated);
    let requests = fixture.state.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].path, "/api/accounts/deviceauth/usercode");
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).unwrap(),
        json!({"client_id":CLIENT_ID})
    );
    for request in &requests[1..3] {
        assert_eq!(request.path, "/api/accounts/deviceauth/token");
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({"device_auth_id":"device-id","user_code":"ABCD-EFGH"})
        );
    }
    assert!(requests[1].at.duration_since(requests[0].at) >= Duration::from_secs(1));
    assert!(requests[2].at.duration_since(requests[1].at) >= Duration::from_secs(1));
    let form: HashMap<_, _> = url::form_urlencoded::parse(requests[3].body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(form["code"], "device-auth-code");
    assert_eq!(form["code_verifier"], "device-verifier");
    assert_eq!(
        form["redirect_uri"],
        format!("{}/deviceauth/callback", fixture.manager.inner.issuer)
    );
}

#[tokio::test]
async fn native_auth_device_cancellation_and_expiry_do_not_publish() {
    let fixture = Fixture::new(vec![
        Reply::json(json!({"device_auth_id":"cancel","user_code":"CANCEL","interval":1})),
        Reply::json(json!({"device_auth_id":"expire","user_code":"EXPIRE","interval":1})),
    ])
    .await;
    fixture.seed().await;
    let original = fixture.bytes();
    let login = fixture.manager.begin_device().await.unwrap();
    drop(login);
    let mut short = AuthManager::test_issuer(
        fixture.temp.path().join("data"),
        fixture.temp.path().join("project"),
        &fixture.manager.inner.issuer,
    )
    .unwrap();
    Arc::get_mut(&mut short.inner).unwrap().login_timeout = Duration::from_millis(60);
    assert!(
        short
            .begin_device()
            .await
            .unwrap()
            .finish()
            .await
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
    assert_eq!(fixture.count(), 2);
    assert_eq!(fixture.bytes(), original);
}

#[tokio::test]
async fn native_auth_concurrent_refresh_uses_one_rotation_and_rejects_old_session() {
    let fixture = Fixture::new(vec![Reply::json(tokens("account-one"))]).await;
    fixture.seed().await;
    let observed = fixture
        .manager
        .credentials(AuthRoute::Chatgpt)
        .await
        .unwrap();
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let manager = AuthManager::test_issuer(
            fixture.temp.path().join("data"),
            fixture.temp.path().join("project"),
            &fixture.manager.inner.issuer,
        )
        .unwrap();
        let observed = observed.clone();
        tasks.push(tokio::spawn(async move {
            manager.refresh_rejected(&observed).await
        }));
    }
    for task in tasks {
        let refreshed = task.await.unwrap().unwrap();
        assert_eq!(refreshed.generation(), Some(1));
        assert_eq!(refreshed.session_id(), observed.session_id());
        assert_ne!(refreshed.bearer(), observed.bearer());
    }
    assert_eq!(fixture.count(), 1);
    {
        let request = fixture.state.requests.lock().unwrap();
        assert_eq!(request[0].path, "/oauth/token");
        assert_eq!(request[0].headers["content-type"], "application/json");
        assert_eq!(
            serde_json::from_str::<Value>(&request[0].body).unwrap(),
            json!({"grant_type":"refresh_token","client_id":CLIENT_ID,"refresh_token":"old-refresh"})
        );
    }
    fixture.seed().await;
    let error = fixture
        .manager
        .refresh_rejected(&observed)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("session changed"));
    assert_eq!(fixture.count(), 1);
}

#[tokio::test]
async fn durable_newer_generation_reuse_consumes_the_logical_rotation() {
    let fixture = Fixture::new(vec![]).await;
    fixture.seed().await;
    let observed = fixture.manager.credentials_snapshot().await.unwrap();
    {
        let lease = fixture
            .manager
            .inner
            .store
            .lease(false, Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        let mut record = lease.read().unwrap().unwrap();
        let session = record.session.as_mut().unwrap();
        session.generation = 1;
        session.access_token = "new-access".into();
        record.revision = random(32).unwrap();
        lease.write(&record).unwrap();
    }
    let budget = OperationBudget::new(Duration::from_secs(5));
    let current = fixture
        .manager
        .resolve_for_operation(&observed, &budget)
        .await
        .unwrap();
    assert_eq!(current.generation(), Some(1));
    assert_eq!(current.bearer(), "new-access");
    assert_eq!(fixture.count(), 0);
    assert!(budget.begin_rotation(Duration::from_secs(5)).is_err());

    let next_budget = OperationBudget::new(Duration::from_secs(5));
    let next_start = fixture.manager.credentials_snapshot().await.unwrap();
    assert_eq!(
        fixture
            .manager
            .resolve_for_operation(&next_start, &next_budget)
            .await
            .unwrap()
            .generation(),
        Some(1)
    );
    assert!(next_budget.begin_rotation(Duration::from_secs(5)).is_ok());
}

#[tokio::test]
async fn native_auth_refresh_rejection_and_account_change_never_retry_or_echo_tokens() {
    for reply in [
        Reply {
            status: StatusCode::UNAUTHORIZED,
            ..Reply::json(json!({"error":"old-refresh synthetic-secret"}))
        },
        Reply::json(tokens("different-account")),
        Reply::json(json!({"refresh_token":"rotated-without-access"})),
        Reply {
            body: "old-refresh synthetic-secret invalid JSON".into(),
            ..Reply::json(json!({}))
        },
    ] {
        let fixture = Fixture::new(vec![reply]).await;
        fixture.seed().await;
        let old = fixture
            .manager
            .credentials(AuthRoute::Chatgpt)
            .await
            .unwrap();
        let error = fixture.manager.refresh_rejected(&old).await.err().unwrap();
        assert!(!format!("{error:#}").contains("old-refresh"));
        assert!(!format!("{error:#}").contains("synthetic-secret"));
        assert!(fixture.manager.refresh_rejected(&old).await.is_err());
        assert!(!fixture.manager.status().await.unwrap().authenticated);
        assert_eq!(fixture.count(), 1);
        fixture.manager.logout().await.unwrap();
        assert!(
            !String::from_utf8(fixture.bytes())
                .unwrap()
                .contains("old-refresh")
        );
    }
}

#[tokio::test]
async fn refresh_preparation_failure_and_connect_refusal_preserve_credentials() {
    let mut preparation = Fixture::new(vec![]).await;
    preparation.seed().await;
    let original = preparation.bytes();
    preparation.manager.fail_refresh_preparation();
    let observed = preparation.manager.credentials_snapshot().await.unwrap();
    let error = preparation
        .manager
        .refresh_rejected(&observed)
        .await
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("prepare replayable"),
        "{error:#}"
    );
    assert_eq!(preparation.manager.refresh_attempts(), 0);
    assert_eq!(preparation.count(), 0);
    assert_eq!(preparation.bytes(), original);

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let manager = AuthManager::test_issuer(temp.path().join("data"), project, &issuer).unwrap();
    manager
        .seed_test_session("old-access", "old-refresh", "account-one")
        .await
        .unwrap();
    let observed = manager.credentials_snapshot().await.unwrap();
    let error = manager.refresh_rejected(&observed).await.err().unwrap();
    assert!(
        error.to_string().contains("before token dispatch"),
        "{error:#}"
    );
    assert_eq!(manager.refresh_attempts(), 2);
    let record = manager.inner.store.read().unwrap().unwrap();
    let session = record.session.unwrap();
    assert!(!session.refresh_pending);
    assert_eq!(session.access_token, "old-access");
    assert_eq!(session.refresh_token, "old-refresh");
    assert_eq!(session.generation, 0);
}

#[tokio::test]
async fn caller_loss_during_proved_unsent_backoff_rolls_back_before_lease_release() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut manager = AuthManager::test_issuer(temp.path().join("data"), project, &issuer).unwrap();
    let gate = manager.pause_refresh(RefreshPhase::BeforeSecond);
    manager
        .seed_test_session("old-access", "old-refresh", "account-one")
        .await
        .unwrap();
    let observed = manager.credentials_snapshot().await.unwrap();
    let caller = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.refresh_rejected(&observed).await })
    };
    if tokio::time::timeout(Duration::from_secs(10), gate.reached.notified())
        .await
        .is_err()
    {
        let cleanup = clean_up_refresh_after_gate_timeout(caller, &manager).await;
        panic!("refresh did not reach the proved-unsent retry gate; {cleanup}");
    }
    caller.abort();
    match caller.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("refresh caller survived cancellation"),
    }
    let lease = manager
        .inner
        .store
        .lease(false, Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    let session = lease.read().unwrap().unwrap().session.unwrap();
    assert!(!session.refresh_pending);
    assert_eq!(session.access_token, "old-access");
    assert_eq!(session.generation, 0);
    assert_eq!(manager.refresh_attempts(), 1);
}

#[tokio::test]
async fn owned_refresh_gate_retries_once_after_safe_refusal_and_publishes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let issuer = format!("http://{address}");
    drop(listener);
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut manager = AuthManager::test_issuer(temp.path().join("data"), project, &issuer).unwrap();
    let gate = manager.pause_refresh(RefreshPhase::BeforeSecond);
    manager
        .seed_test_session("old-access", "old-refresh", "account-one")
        .await
        .unwrap();
    let observed = manager.credentials_snapshot().await.unwrap();
    let mut refresh = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.refresh_rejected(&observed).await })
    };
    if tokio::time::timeout(Duration::from_secs(10), gate.reached.notified())
        .await
        .is_err()
    {
        let cleanup = clean_up_refresh_after_gate_timeout(refresh, &manager).await;
        panic!("refresh did not reach the retry gate; {cleanup}");
    }
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    let mut server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let count = socket.read(&mut chunk).await.unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
            assert!(bytes.len() <= 16 * 1024);
            let text = String::from_utf8_lossy(&bytes);
            let Some(header_end) = text.find("\r\n\r\n") else {
                continue;
            };
            let length = text[..header_end]
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .unwrap()
                .parse::<usize>()
                .unwrap();
            if bytes.len() >= header_end + 4 + length {
                break;
            }
        }
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("POST /oauth/token HTTP/1.1\r\n"));
        let body = tokens("account-one").to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        bytes
    });
    gate.release.notify_one();
    let refresh_result = match tokio::time::timeout(Duration::from_secs(15), &mut refresh).await {
        Ok(result) => result,
        Err(_) => {
            server.abort();
            let _ = server.await;
            let cleanup = clean_up_refresh_after_gate_timeout(refresh, &manager).await;
            panic!("refresh did not finish after the retry gate; {cleanup}");
        }
    };
    let refreshed = match refresh_result {
        Ok(Ok(credentials)) => credentials,
        Ok(Err(error)) => {
            server.abort();
            let _ = server.await;
            panic!("refresh failed after the retry gate: {error:#}");
        }
        Err(error) => {
            server.abort();
            let _ = server.await;
            panic!("refresh task failed after the retry gate: {error}");
        }
    };
    assert_eq!(refreshed.generation(), Some(1));
    let captured = match tokio::time::timeout(Duration::from_secs(5), &mut server).await {
        Ok(result) => result.unwrap(),
        Err(_) => {
            server.abort();
            let _ = server.await;
            panic!("refresh endpoint did not finish")
        }
    };
    assert_eq!(
        String::from_utf8_lossy(&captured)
            .matches("POST /oauth/token")
            .count(),
        1
    );
    assert_eq!(manager.refresh_attempts(), 2);
    let session = manager
        .inner
        .store
        .read()
        .unwrap()
        .unwrap()
        .session
        .unwrap();
    assert_eq!(session.generation, 1);
    assert!(!session.refresh_pending);
}

#[tokio::test]
async fn owner_gate_caller_loss_and_expired_grant_stop_before_first_post() {
    for expire in [false, true] {
        let fixture = Fixture::new(vec![]).await;
        fixture.seed().await;
        let mut manager = AuthManager::test_issuer(
            fixture.temp.path().join("data"),
            fixture.temp.path().join("project"),
            &fixture.manager.inner.issuer,
        )
        .unwrap();
        if expire {
            Arc::get_mut(&mut manager.inner).unwrap().http_timeout = Duration::from_secs(2);
        }
        let gate = manager.pause_refresh(RefreshPhase::BeforeFirst);
        let before_record = manager.inner.store.read().unwrap().unwrap();
        let before_revision = before_record.revision.clone();
        let before = before_record.session.unwrap();
        let observed = manager.credentials_snapshot().await.unwrap();
        let caller = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.refresh_rejected(&observed).await })
        };
        tokio::time::timeout(Duration::from_secs(5), gate.reached.notified())
            .await
            .unwrap();
        if expire {
            assert!(caller.await.unwrap().is_err());
        } else {
            caller.abort();
            match caller.await {
                Err(error) => assert!(error.is_cancelled()),
                Ok(_) => panic!("refresh caller survived cancellation"),
            }
        }
        let lease = manager
            .inner
            .store
            .lease(false, Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        let after_record = lease.read().unwrap().unwrap();
        assert_ne!(after_record.revision, before_revision);
        assert!(after_record.session.unwrap() == before);
        assert_eq!(manager.refresh_attempts(), 0);
        assert_eq!(fixture.count(), 0);
    }
}

#[tokio::test]
async fn expired_allowance_before_start_and_at_second_gate_never_dispatch_late() {
    let fixture = Fixture::new(vec![]).await;
    fixture.seed().await;
    let original = fixture.bytes();
    let observed = fixture.manager.credentials_snapshot().await.unwrap();
    let budget = OperationBudget::new(Duration::from_secs(5));
    let expired = budget.begin_rotation(Duration::ZERO).unwrap();
    assert!(
        fixture
            .manager
            .refresh_with_allowance(&observed, expired)
            .await
            .is_err()
    );
    assert_eq!(fixture.bytes(), original);
    assert_eq!(fixture.count(), 0);

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut manager = AuthManager::test_issuer(temp.path().join("data"), project, &issuer).unwrap();
    let gate = manager.pause_refresh(RefreshPhase::BeforeSecond);
    manager
        .seed_test_session("old-access", "old-refresh", "account-one")
        .await
        .unwrap();
    let before = manager.inner.store.read().unwrap().unwrap();
    let observed = manager.credentials_snapshot().await.unwrap();
    let budget = OperationBudget::new(Duration::from_secs(15));
    let allowance = budget.begin_rotation(Duration::from_secs(10)).unwrap();
    let mut refresh = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.refresh_with_allowance(&observed, allowance).await })
    };
    if tokio::time::timeout(Duration::from_secs(12), gate.reached.notified())
        .await
        .is_err()
    {
        let cleanup = clean_up_refresh_after_gate_timeout(refresh, &manager).await;
        panic!("refresh did not reach the second-request gate; {cleanup}");
    }
    let result = match tokio::time::timeout(Duration::from_secs(12), &mut refresh).await {
        Ok(result) => result.unwrap(),
        Err(_) => {
            let cleanup = clean_up_refresh_after_gate_timeout(refresh, &manager).await;
            panic!("expired refresh did not finish; {cleanup}");
        }
    };
    assert!(result.is_err());
    let lease = manager
        .inner
        .store
        .lease(false, Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    let after = lease.read().unwrap().unwrap();
    assert_ne!(after.revision, before.revision);
    assert!(after.session == before.session);
    assert_eq!(manager.refresh_attempts(), 1);
}

#[tokio::test]
async fn native_proxy_connect_is_counted_separately_from_token_posts() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let transcripts = Arc::new(Mutex::new(Vec::new()));
    let observed = transcripts.clone();
    let mut server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let count = socket.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&chunk[..count]);
                assert!(bytes.len() <= 8 * 1024);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            observed.lock().unwrap().push(bytes);
            socket
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut manager = AuthManager::new(temp.path().join("data"), project, None).unwrap();
    manager.use_refresh_proxy("https://auth.invalid", proxy);
    manager
        .seed_test_session("old-access", "old-refresh", "account-one")
        .await
        .unwrap();
    let observed = manager.credentials_snapshot().await.unwrap();
    let error = manager.refresh_rejected(&observed).await.err().unwrap();
    assert!(
        error.to_string().contains("before token dispatch"),
        "{error:#}"
    );
    match tokio::time::timeout(Duration::from_secs(5), &mut server).await {
        Ok(result) => result.unwrap(),
        Err(_) => {
            server.abort();
            let _ = server.await;
            panic!("proxy fixture did not finish")
        }
    }
    let transcripts = transcripts.lock().unwrap();
    assert_eq!(transcripts.len(), 2);
    assert!(transcripts.iter().all(|bytes| {
        let text = String::from_utf8_lossy(bytes);
        text.lines().next() == Some("CONNECT auth.invalid:443 HTTP/1.1")
            && !text.contains("oauth/token")
    }));
    let session = manager
        .inner
        .store
        .read()
        .unwrap()
        .unwrap()
        .session
        .unwrap();
    assert!(!session.refresh_pending);
    assert_eq!(session.generation, 0);
}

#[tokio::test]
async fn native_auth_logout_waits_for_owned_rotation_and_cannot_be_resurrected() {
    let gate = Arc::new(Notify::new());
    let fixture = Fixture::new(vec![Reply {
        gate: Some(gate.clone()),
        ..Reply::json(tokens("account-one"))
    }])
    .await;
    fixture.seed().await;
    let old = fixture
        .manager
        .credentials(AuthRoute::Chatgpt)
        .await
        .unwrap();
    let manager = fixture.manager.clone();
    let copy = old.clone();
    let refresh = tokio::spawn(async move { manager.refresh_rejected(&copy).await });
    fixture.arrived(1).await;
    let manager = fixture.manager.clone();
    let logout = tokio::spawn(async move { manager.logout().await });
    gate.notify_one();
    refresh.await.unwrap().unwrap();
    logout.await.unwrap().unwrap();
    assert!(!fixture.manager.status().await.unwrap().authenticated);
    assert!(fixture.manager.refresh_rejected(&old).await.is_err());
    assert_eq!(fixture.count(), 1);
    assert!(
        !String::from_utf8(fixture.bytes())
            .unwrap()
            .contains("synthetic-refresh")
    );
}

#[tokio::test]
async fn native_auth_refresh_survives_caller_runtime_destruction_and_persists_rotation() {
    let gate = Arc::new(Notify::new());
    let fixture = Fixture::new(vec![Reply {
        gate: Some(gate.clone()),
        ..Reply::json(tokens("account-one"))
    }])
    .await;
    fixture.seed().await;
    let observed = fixture
        .manager
        .credentials(AuthRoute::Chatgpt)
        .await
        .unwrap();
    let manager = fixture.manager.clone();
    let (destroy, destroyed) = tokio::sync::oneshot::channel();
    let caller = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::spawn(async move { manager.refresh_rejected(&observed).await });
            destroyed.await.unwrap();
        });
    });
    fixture.arrived(1).await;
    destroy.send(()).unwrap();
    caller.join().unwrap();
    gate.notify_one();
    // The real exclusive lease is released only after the detached owner writes.
    let lease = fixture
        .manager
        .inner
        .store
        .lease(false, Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    let record = lease.read().unwrap().unwrap();
    let session = record.session.unwrap();
    assert_eq!(session.generation, 1);
    assert!(!session.refresh_pending);
    assert_eq!(session.refresh_token, "synthetic-refresh");
    assert_eq!(fixture.count(), 1);
}

#[tokio::test]
async fn refresh_publication_reply_loss_and_read_failure_reconcile_exact_state() {
    for write in [1, 2] {
        let fixture = Fixture::new(vec![Reply::json(tokens("account-one"))]).await;
        fixture.seed().await;
        let observed = fixture.manager.credentials_snapshot().await.unwrap();
        fixture
            .manager
            .inner
            .store
            .fail_after_publish_on_write(write);
        let refreshed = fixture.manager.refresh_rejected(&observed).await.unwrap();
        assert_eq!(refreshed.generation(), Some(1));
        assert_eq!(fixture.count(), 1);
        let session = fixture
            .manager
            .inner
            .store
            .read()
            .unwrap()
            .unwrap()
            .session
            .unwrap();
        assert_eq!(session.generation, 1);
        assert!(!session.refresh_pending);
    }

    let fixture = Fixture::new(vec![Reply::json(tokens("account-one"))]).await;
    fixture.seed().await;
    let observed = fixture.manager.credentials_snapshot().await.unwrap();
    fixture.manager.inner.store.fail_on_read(3);
    let error = fixture
        .manager
        .refresh_rejected(&observed)
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "authentication rotation state is unknown; run kuru login"
    );
    assert_eq!(fixture.count(), 1);
    let session = fixture
        .manager
        .inner
        .store
        .read()
        .unwrap()
        .unwrap()
        .session
        .unwrap();
    assert_eq!(session.generation, 1);
    assert!(!session.refresh_pending);
}

#[tokio::test]
async fn native_auth_expiry_rotates_and_logout_fences_pending_browser_login() {
    let fixture = Fixture::new(vec![
        Reply::json(tokens("account-one")),
        Reply::json(tokens("account-one")),
    ])
    .await;
    fixture.seed().await;
    {
        let lease = fixture
            .manager
            .inner
            .store
            .lease(false, Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        let mut record = lease.read().unwrap().unwrap();
        record.session.as_mut().unwrap().expires_at = now().unwrap() - 1;
        lease.write(&record).unwrap();
    }
    assert_eq!(
        fixture
            .manager
            .credentials(AuthRoute::Chatgpt)
            .await
            .unwrap()
            .generation(),
        Some(1)
    );
    let login = fixture.manager.begin_browser().await.unwrap();
    let (port, state, _) = callback(&login);
    fixture.manager.logout().await.unwrap();
    let original = fixture.bytes();
    let task = tokio::spawn(login.finish());
    let received = callback_request(
        port,
        &format!("/auth/callback?code=valid&state={state}"),
        "GET",
        &format!("localhost:{port}"),
    )
    .await;
    assert!(received.starts_with("HTTP/1.1 200"));
    assert!(received.contains("Authorization received; return to Kuru for the result."));
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("authentication changed")
    );
    assert_eq!(fixture.bytes(), original);
    assert!(!fixture.manager.status().await.unwrap().authenticated);
}

#[tokio::test]
async fn native_auth_rejects_unsafe_and_malformed_private_records() {
    let fixture = Fixture::new(vec![]).await;
    fixture.seed().await;
    let path = fixture.manager.inner.store.path().join("credentials.json");
    std::fs::hard_link(&path, fixture.temp.path().join("additional-link")).unwrap();
    assert!(fixture.manager.status().await.is_err());
    std::fs::remove_file(fixture.temp.path().join("additional-link")).unwrap();
    std::fs::write(&path, b"{\"secret\":\"synthetic-never-print\"}").unwrap();
    let error = fixture.manager.status().await.unwrap_err();
    assert!(!format!("{error:#}").contains("synthetic-never-print"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            fixture
                .manager
                .status()
                .await
                .unwrap_err()
                .to_string()
                .contains("private authentication")
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn native_auth_rejects_symlinked_store_and_replaced_lock_identity() {
    let fixture = Fixture::new(vec![]).await;
    let data = fixture.temp.path().join("data");
    std::os::unix::fs::symlink(fixture.temp.path().join("project"), &data).unwrap();
    assert!(fixture.manager.status().await.is_err());
    std::fs::remove_file(&data).unwrap();
    fixture.seed().await;
    let lease = fixture
        .manager
        .inner
        .store
        .lease(false, Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    let lock = fixture.manager.inner.store.path().join("credentials.lock");
    std::fs::rename(
        &lock,
        fixture.manager.inner.store.path().join("displaced.lock"),
    )
    .unwrap();
    let directory = kuru_platform::fs::Directory::open(
        fixture.manager.inner.store.path(),
        kuru_platform::fs::Privacy::OwnerOnly,
        kuru_platform::fs::NameRetention::Movable,
    )
    .unwrap();
    directory
        .lock_file(std::ffi::OsStr::new("credentials.lock"))
        .unwrap();
    let original = fixture.bytes();
    assert!(lease.write(&store::Record::new(None).unwrap()).is_err());
    assert_eq!(fixture.bytes(), original);
}

#[tokio::test]
async fn native_auth_fresh_logout_fences_browser_and_device_activation() {
    let browser = Fixture::new(vec![Reply::json(tokens("account-one"))]).await;
    let login = browser.manager.begin_browser().await.unwrap();
    let (port, state, _) = callback(&login);
    assert!(!browser.temp.path().join("data").exists());
    browser.manager.logout().await.unwrap();
    let tombstone = browser.bytes();
    let task = tokio::spawn(login.finish());
    let received = callback_request(
        port,
        &format!("/auth/callback?code=valid&state={state}"),
        "GET",
        &format!("localhost:{port}"),
    )
    .await;
    assert!(received.starts_with("HTTP/1.1 200"));
    assert!(received.contains("Authorization received; return to Kuru for the result."));
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("authentication changed")
    );
    assert_eq!(browser.bytes(), tombstone);
    assert!(!browser.manager.status().await.unwrap().authenticated);

    let device = Fixture::new(vec![
        Reply::json(json!({"device_auth_id":"device","user_code":"CODE","interval":1})),
        Reply::json(json!({"authorization_code":"code","code_verifier":"verifier"})),
        Reply::json(tokens("account-one")),
    ])
    .await;
    let login = device.manager.begin_device().await.unwrap();
    assert!(!device.temp.path().join("data").exists());
    device.manager.logout().await.unwrap();
    let tombstone = device.bytes();
    assert!(
        login
            .finish()
            .await
            .unwrap_err()
            .to_string()
            .contains("authentication changed")
    );
    assert_eq!(device.bytes(), tombstone);
    assert!(!device.manager.status().await.unwrap().authenticated);
}
