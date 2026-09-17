use super::*;
use crate::test_support::{Reply, request};
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
};
use std::collections::VecDeque;

struct Recorded {
    uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Value,
}
#[derive(Clone)]
struct StateData {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}
struct Peer {
    url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Peer {
    async fn new(replies: Vec<Reply>) -> Self {
        let state = StateData {
            replies: Arc::new(Mutex::new(replies.into())),
            requests: Arc::new(Mutex::new(vec![])),
        };
        let router = Router::new()
            .fallback(any(handle))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url,
            requests: state.requests,
            task,
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn handle(
    State(state): State<StateData>,
    uri: Uri,
    method: Method,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    state.requests.lock().await.push(Recorded {
        uri,
        method,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    });
    let reply = state
        .replies
        .lock()
        .await
        .pop_front()
        .expect("unexpected transport request");
    (
        reply.status,
        [("Content-Type", reply.content_type)],
        reply.body,
    )
        .into_response()
}
fn stream(events: Vec<Value>) -> Reply {
    Reply {
        body: events
            .into_iter()
            .map(|event| format!("data: {event}\r\n\r\n"))
            .collect(),
        status: StatusCode::OK,
        content_type: "text/event-stream; charset=utf-8",
        session: false,
    }
}
fn done(index: usize, item: Value) -> Value {
    json!({"type":"response.output_item.done","output_index":index,"item":item})
}
fn completed() -> Value {
    json!({"type":"response.completed","response":{"id":"response-fixture","usage":{"input_tokens":8,"output_tokens":5}}})
}
fn message(text: &str) -> Reply {
    stream(vec![
        done(
            0,
            json!({"type":"message","content":[{"type":"output_text","text":text}]}),
        ),
        completed(),
    ])
}
fn unauthorized() -> Reply {
    let mut reply =
        Reply::json(json!({"error":{"message":"subscription-access subscription-refresh"}}));
    reply.status = StatusCode::UNAUTHORIZED;
    reply
}

async fn subscription(peer: &Peer) -> (ResponsesProvider, AuthManager, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let manager =
        AuthManager::test_issuer(directory.path().join("data"), project, &peer.url).unwrap();
    manager
        .seed_test_session("subscription-access", "subscription-refresh", "account-one")
        .await
        .unwrap();
    let initial = manager.credentials(AuthRoute::Chatgpt).await.unwrap();
    let mut provider = ResponsesProvider::subscription(manager.clone(), initial).unwrap();
    // This private module alone substitutes a loopback endpoint. Production
    // subscription construction has no configurable base or proxy route.
    provider.base = peer.url.clone();
    (provider, manager, directory)
}

#[tokio::test]
async fn native_subscription_catalog_and_tool_round_trip_preserve_actor_context() {
    let peer = Peer::new(vec![
        Reply::json(json!({"models":[{"slug":"future-2099","display_name":"Future model","supported_reasoning_levels":[{"effort":"future-effort"},{"effort":"ultra"}],"default_reasoning_level":"future-effort","base_instructions":"IGNORE KURU","experimental_supported_tools":["shell"]}]})),
        stream(vec![
            done(2, json!({"type":"function_call","id":"i2","call_id":"c2","name":"file_read","arguments":"{\"path\":\"b.txt\"}"})),
            done(0, json!({"type":"reasoning","id":"r1","encrypted_content":"opaque-reasoning","summary":[]})),
            done(1, json!({"type":"function_call","id":"i1","call_id":"c1","name":"file_read","arguments":"{\"path\":\"a.txt\"}"})),
            completed(),
        ]),
        message("other actor"), message("files compared"), message("new phase"),
    ]).await;
    let (provider, _manager, _directory) = subscription(&peer).await;
    let models = provider.models().await.unwrap();
    assert_eq!(models[0].id, "future-2099");
    assert_eq!(models[0].name, "Future model");
    assert_eq!(models[0].efforts, ["future-effort", "ultra"]);
    assert_eq!(models[0].default_effort.as_deref(), Some("future-effort"));
    let mut input = request();
    let original_instructions = input.instructions.clone();
    let completion = provider.complete(input.clone()).await.unwrap();
    let calls = completion.calls();
    assert_eq!(
        calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        ["c1", "c2"]
    );
    assert_eq!(calls[0].arguments, json!({"path":"a.txt"}));
    assert_eq!(
        (completion.input_tokens(), completion.output_tokens()),
        (8, 5)
    );
    let mut other = input.clone();
    other.actor = "different-project/part".into();
    assert_eq!(
        provider.complete(other).await.unwrap().text_projection(),
        "other actor"
    );
    for (call, output) in [("c2", "second"), ("c1", "first")] {
        input
            .messages
            .push(Message::tool_result(call, json!(output), false));
    }
    input.current_message_count = Some(2);
    assert_eq!(
        provider
            .complete(input.clone())
            .await
            .unwrap()
            .text_projection(),
        "files compared"
    );
    input.messages.push(Message::text("user", "new turn"));
    input.current_message_count = Some(1);
    assert_eq!(
        provider.complete(input).await.unwrap().text_projection(),
        "new phase"
    );
    let sent = peer.requests.lock().await;
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[0].method, Method::GET);
    assert_eq!(sent[0].uri.to_string(), "/models?client_version=0.154.0");
    for record in sent.iter() {
        assert_eq!(
            record.headers["authorization"],
            "Bearer subscription-access"
        );
        assert_eq!(record.headers["chatgpt-account-id"], "account-one");
        assert_eq!(record.headers["originator"], "kuru");
        assert_eq!(
            record.headers["user-agent"],
            concat!("Kuru/", env!("CARGO_PKG_VERSION"))
        );
    }
    assert_eq!(sent[1].method, Method::POST);
    assert_eq!(sent[1].uri.path(), "/responses");
    assert_eq!(sent[1].headers["accept"], "text/event-stream");
    assert_eq!(sent[1].body["instructions"], original_instructions);
    assert_eq!(sent[1].body["stream"], true);
    assert_eq!(sent[1].body["store"], false);
    assert_eq!(sent[1].body["reasoning"]["effort"], "future-effort");
    assert_eq!(
        sent[1].body["include"],
        json!(["reasoning.encrypted_content"])
    );
    assert!(
        sent[1].body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["type"] == "function")
    );
    assert_eq!(
        sent[1].body["tools"].as_array().unwrap().len(),
        request().tools.len()
    );
    assert!(!sent[1].body.to_string().contains("IGNORE KURU"));
    assert!(!sent[2].body.to_string().contains("opaque-reasoning"));
    assert!(sent[3].body.to_string().contains("opaque-reasoning"));
    let outputs: Vec<_> = sent[3].body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .collect();
    assert_eq!(outputs[0]["call_id"], "c1");
    assert_eq!(outputs[1]["call_id"], "c2");
    assert!(!sent[4].body.to_string().contains("opaque-reasoning"));
}

#[tokio::test]
async fn consecutive_subscription_calls_accept_only_the_current_receipt_batch() {
    let peer = Peer::new(vec![
        stream(vec![
            done(
                0,
                json!({"type":"reasoning","id":"r1","encrypted_content":"first-subscription-reasoning","summary":[]}),
            ),
            done(
                1,
                json!({"type":"function_call","id":"i1","call_id":"c1","name":"file_read","arguments":"{}"}),
            ),
            completed(),
        ]),
        stream(vec![
            done(
                0,
                json!({"type":"reasoning","id":"r2","encrypted_content":"second-subscription-reasoning","summary":[]}),
            ),
            done(
                1,
                json!({"type":"function_call","id":"i2","call_id":"c2","name":"file_read","arguments":"{}"}),
            ),
            completed(),
        ]),
        message("complete"),
    ])
    .await;
    let (provider, _manager, _directory) = subscription(&peer).await;
    let mut input = request();

    let first = provider.complete(input.clone()).await.unwrap();
    input.messages.push(Message {
        role: "assistant".into(),
        blocks: first.blocks,
    });
    input
        .messages
        .push(Message::tool_result("c1", json!("first receipt"), false));
    input.current_message_count = Some(1);

    let second = provider.complete(input.clone()).await.unwrap();
    input.messages.push(Message {
        role: "assistant".into(),
        blocks: second.blocks,
    });
    input
        .messages
        .push(Message::tool_result("c2", json!("second receipt"), false));
    input.current_message_count = Some(1);

    assert_eq!(
        provider.complete(input).await.unwrap().text_projection(),
        "complete"
    );
    let sent = peer.requests.lock().await;
    assert_eq!(sent.len(), 3);
    let outputs: Vec<_> = sent[2].body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .collect();
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0]["call_id"], "c1");
    assert_eq!(outputs[0]["output"], "first receipt");
    assert_eq!(outputs[1]["call_id"], "c2");
    assert_eq!(outputs[1]["output"], "second receipt");
    assert!(
        sent[2]
            .body
            .to_string()
            .contains("first-subscription-reasoning")
    );
    assert!(
        sent[2]
            .body
            .to_string()
            .contains("second-subscription-reasoning")
    );
}

#[tokio::test]
async fn rejected_subscription_rotates_once_without_changing_account_or_route() {
    let rotated = crate::auth::test_future_jwt("account-one");
    let peer = Peer::new(vec![
        unauthorized(),
        Reply::json(
            json!({"access_token":rotated,"refresh_token":"rotated-refresh","expires_in":3600}),
        ),
        message("rotated"),
    ])
    .await;
    let (provider, manager, _directory) = subscription(&peer).await;
    assert_eq!(
        provider
            .complete(request())
            .await
            .unwrap()
            .text_projection(),
        "rotated"
    );
    let sent = peer.requests.lock().await;
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].uri.path(), "/responses");
    assert_eq!(sent[1].uri.path(), "/oauth/token");
    assert_eq!(
        sent[1].body,
        json!({"client_id":"app_EMoamEEZ73f0CkXaXp7hrann","grant_type":"refresh_token","refresh_token":"subscription-refresh"})
    );
    assert!(!sent[1].headers.contains_key("chatgpt-account-id"));
    assert_eq!(sent[2].uri.path(), "/responses");
    assert_eq!(
        sent[2].headers["authorization"],
        format!("Bearer {rotated}")
    );
    assert_eq!(sent[2].headers["chatgpt-account-id"], "account-one");
    assert_eq!(sent[0].body, sent[2].body);
    drop(sent);
    assert_eq!(
        manager
            .credentials(AuthRoute::Chatgpt)
            .await
            .unwrap()
            .bearer(),
        rotated
    );
}

#[tokio::test]
async fn rejected_503_then_401_shares_one_finite_rotation_budget() {
    let mut unavailable = Reply::json(json!({"error":{"message":"subscription-transient-secret"}}));
    unavailable.status = StatusCode::SERVICE_UNAVAILABLE;
    let rotated = crate::auth::test_future_jwt("account-one");
    let peer = Peer::new(vec![
        unavailable,
        unauthorized(),
        Reply::json(
            json!({"access_token":rotated,"refresh_token":"rotated-refresh","expires_in":3600}),
        ),
        message("bounded rotation"),
        message("later operation"),
    ])
    .await;
    let (provider, _manager, _directory) = subscription(&peer).await;
    assert_eq!(
        provider
            .complete(request())
            .await
            .unwrap()
            .text_projection(),
        "bounded rotation"
    );
    assert_eq!(
        provider
            .complete(request())
            .await
            .unwrap()
            .text_projection(),
        "later operation"
    );
    let sent = peer.requests.lock().await;
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[0].uri.path(), "/responses");
    assert_eq!(sent[1].uri.path(), "/responses");
    assert_eq!(sent[2].uri.path(), "/oauth/token");
    assert_eq!(sent[3].uri.path(), "/responses");
    assert_eq!(sent[4].uri.path(), "/responses");
    assert_eq!(sent[0].body, sent[1].body);
    assert_eq!(sent[1].body, sent[3].body);
    assert_eq!(
        sent[0].headers["authorization"],
        "Bearer subscription-access"
    );
    assert_eq!(
        sent[1].headers["authorization"],
        "Bearer subscription-access"
    );
    assert_eq!(
        sent[3].headers["authorization"],
        format!("Bearer {rotated}")
    );
    assert_eq!(
        sent[4].headers["authorization"],
        format!("Bearer {rotated}")
    );
}

#[tokio::test]
async fn subscription_completion_and_catalog_retry_explicit_rejections() {
    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let mut rejected = Reply::json(json!({"error":{"message":"native-retry-secret"}}));
        rejected.status = status;
        let peer = Peer::new(vec![rejected, message("retried")]).await;
        let (provider, _manager, _directory) = subscription(&peer).await;
        assert_eq!(
            provider
                .complete(request())
                .await
                .unwrap()
                .text_projection(),
            "retried"
        );
        let sent = peer.requests.lock().await;
        assert_eq!(sent.len(), 2, "completion status {status}");
        assert_eq!(sent[0].body, sent[1].body);
        assert_eq!(
            sent[0].headers["authorization"],
            sent[1].headers["authorization"]
        );

        let mut rejected = Reply::json(json!({"error":{"message":"native-retry-secret"}}));
        rejected.status = status;
        let peer = Peer::new(vec![
            rejected,
            Reply::json(json!({"models":[{"slug":"future-model"}]})),
        ])
        .await;
        let (provider, _manager, _directory) = subscription(&peer).await;
        assert_eq!(provider.models().await.unwrap()[0].id, "future-model");
        let sent = peer.requests.lock().await;
        assert_eq!(sent.len(), 2, "catalog status {status}");
        assert_eq!(sent[0].method, Method::GET);
        assert_eq!(sent[1].method, Method::GET);
        assert_eq!(
            sent[0].headers["authorization"],
            sent[1].headers["authorization"]
        );
    }

    let peer = Peer::new(
        (0..3)
            .map(|_| {
                let mut reply = Reply::json(json!({
                    "error": {
                        "type": "insufficient_quota",
                        "code": "project_spend_limit_exceeded",
                        "message": "subscription-secret"
                    }
                }));
                reply.status = StatusCode::TOO_MANY_REQUESTS;
                reply
            })
            .collect(),
    )
    .await;
    let (provider, _manager, _directory) = subscription(&peer).await;
    let error = provider.complete(request()).await.err().unwrap();
    assert_eq!(peer.requests.lock().await.len(), 3);
    let diagnostic = format!("{error:#}");
    assert!(
        diagnostic.contains("retry budget exhausted"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("billing") && !diagnostic.contains("subscription-secret"));
}

#[tokio::test]
async fn repeated_401_and_partial_stream_errors_do_not_retry_or_leak_tokens() {
    let peer = Peer::new(vec![
        unauthorized(),
        Reply::json(
            json!({"access_token":crate::auth::test_future_jwt("account-one"),"expires_in":3600}),
        ),
        unauthorized(),
    ])
    .await;
    let (provider, _manager, _directory) = subscription(&peer).await;
    let error = provider.complete(request()).await.unwrap_err();
    let diagnostic = format!("{error:#}");
    assert!(diagnostic.contains("401"), "{diagnostic}");
    assert!(
        !diagnostic.contains("subscription-access") && !diagnostic.contains("subscription-refresh")
    );
    assert_eq!(peer.requests.lock().await.len(), 3);

    let peer = Peer::new(vec![stream(vec![done(0, json!({"type":"message","content":[{"text":"partial"}]})), json!({"type":"response.failed","response":{"error":{"message":"subscription-access"}}})]), message("next")]).await;
    let (provider, _manager, _directory) = subscription(&peer).await;
    let error = provider.complete(request()).await.unwrap_err();
    assert!(!format!("{error:#}").contains("subscription-access"));
    assert_eq!(peer.requests.lock().await.len(), 1);
    assert_eq!(
        provider
            .complete(request())
            .await
            .unwrap()
            .text_projection(),
        "next"
    );
    assert_eq!(peer.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn native_failed_events_and_initial_statuses_use_fixed_diagnostics() {
    let mut initial = Reply::json(
        json!({"error":{"message":"native-status-secret","code":"model_not_found","type":"insufficient_quota"}}),
    );
    initial.status = StatusCode::FORBIDDEN;
    let peer = Peer::new(vec![
        stream(vec![json!({"type":"response.failed","response":{"error":{"code":"server_is_overloaded","message":"native-known-secret"}}})]),
        stream(vec![json!({"type":"response.failed","response":{"error":{"code":"future-native-code","type":"future-native-type","message":"native-unknown-secret"}}})]),
        initial,
    ])
    .await;
    let (provider, _manager, _directory) = subscription(&peer).await;
    for expected in [
        "ChatGPT service is overloaded",
        "ChatGPT completion stream failed before completion",
        "ChatGPT completion access was denied (HTTP 403)",
    ] {
        let error = provider.complete(request()).await.unwrap_err();
        let display = format!("{error:#}");
        let debug = format!("{error:?}");
        assert!(display.contains(expected), "{display}");
        for secret in [
            "native-status-secret",
            "native-known-secret",
            "native-unknown-secret",
            "future-native-code",
            "future-native-type",
        ] {
            assert!(
                !display.contains(secret) && !debug.contains(secret),
                "native value escaped: {display} / {debug}"
            );
        }
    }
    assert_eq!(
        peer.requests.lock().await.len(),
        3,
        "failed streams were replayed"
    );
}

#[tokio::test]
async fn replacement_login_rejects_old_provider_before_sending_pending_context() {
    for account in ["account-one", "different-account"] {
        let peer = Peer::new(vec![stream(vec![
            done(
                0,
                json!({"type":"function_call","call_id":"c1","name":"file_read","arguments":"{}"}),
            ),
            completed(),
        ])])
        .await;
        let (provider, manager, _directory) = subscription(&peer).await;
        let mut input = request();
        provider.complete(input.clone()).await.unwrap();
        manager
            .seed_test_session("new-access", "new-refresh", account)
            .await
            .unwrap();
        input
            .messages
            .push(Message::tool_result("c1", json!("private-result"), false));
        let error = provider.complete(input).await.unwrap_err();
        assert!(error.to_string().contains("session changed"), "{error:#}");
        assert_eq!(peer.requests.lock().await.len(), 1);
    }
}

#[tokio::test]
async fn malformed_incomplete_and_oversized_subscription_responses_are_errors() {
    let mut malformed = stream(vec![]);
    malformed.body = "data: not-json\n\n".into();
    let mut oversized = stream(vec![]);
    oversized.body = "x".repeat(crate::MAX_BYTES * 32 + 1);
    let peer = Peer::new(vec![
        malformed,
        stream(vec![json!({"type":"response.incomplete"})]),
        stream(vec![done(
            0,
            json!({"type":"message","content":[{"text":"unfinished"}]}),
        )]),
        oversized,
        Reply::json(json!({"output":[]})),
    ])
    .await;
    let (provider, _manager, _directory) = subscription(&peer).await;
    for expected in [
        "invalid protocol",
        "stream failed before completion",
        "before response.completed",
        "limit",
        "event stream",
    ] {
        let error = provider.complete(request()).await.unwrap_err();
        assert!(
            format!("{error:#}").contains(expected),
            "expected {expected}: {error:#}"
        );
    }
    assert_eq!(peer.requests.lock().await.len(), 5);
}

#[test]
fn invalid_catalog_shapes_fail_without_replacing_future_capabilities() {
    for value in [
        json!({}),
        json!({"models":[{}]}),
        json!({"models":[{"slug":""}]}),
        json!({"models":[{"slug":"x","supported_reasoning_levels":"invalid"}]}),
        json!({"models":[{"slug":"x","supported_reasoning_levels":[{}]}]}),
        json!({"models":[{"slug":"x","default_reasoning_level":3}]}),
    ] {
        assert!(subscription_models(&value).is_err());
    }
    assert!(
        subscription_models(&json!({"models":[{"slug":"future"}]})).unwrap()[0]
            .efforts
            .is_empty()
    );
}

struct AbortOnDrop(tokio::task::AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0, "request ended before headers/body");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= crate::MAX_BYTES + 8192);
        if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return;
            }
        }
    }
}

async fn assert_closed(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    let mut byte = [0];
    let result = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut byte))
        .await
        .expect("cancelled stream retained its HTTP connection");
    // Closing an unread response can produce either FIN/EOF or a native TCP
    // reset. Both observe closure; data, other errors and timeout still fail.
    match result {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("expected closed HTTP connection, observed {other:?}"),
    }
}

#[tokio::test]
async fn cancelled_partial_stream_closes_socket_and_releases_same_actor() {
    use tokio::io::AsyncWriteExt;
    let peer = Peer::new(vec![]).await;
    let (mut provider, _manager, _directory) = subscription(&peer).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    provider.base = format!("http://{}", listener.local_addr().unwrap());
    let provider = Arc::new(provider);
    let (ready, started) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let data = b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
        socket
            .write_all(format!("{:x}\r\n", data.len()).as_bytes())
            .await
            .unwrap();
        socket.write_all(data).await.unwrap();
        socket.write_all(b"\r\n").await.unwrap();
        ready.send(()).unwrap();
        assert_closed(&mut socket).await;
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        let body = message("after cancellation").body;
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    let _abort = AbortOnDrop(server.abort_handle());
    let first = tokio::spawn({
        let provider = provider.clone();
        async move { provider.complete(request()).await }
    });
    let _first_abort = AbortOnDrop(first.abort_handle());
    tokio::time::timeout(Duration::from_secs(5), started)
        .await
        .unwrap()
        .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let result = tokio::time::timeout(Duration::from_secs(5), provider.complete(request()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.text_projection(), "after cancellation");
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        peer.requests.lock().await.is_empty(),
        "cancellation attempted token refresh"
    );
}

#[tokio::test]
async fn real_idle_stream_is_bounded_and_dropped() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        assert_closed(&mut socket).await;
    });
    let _abort = AbortOnDrop(server.abort_handle());
    let response = http::client().unwrap().get(url).send().await.unwrap();
    let error = sse::response(
        response,
        Duration::from_millis(100),
        diagnostics::Operation::ChatgptCompletion,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("idle timeout"));
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

/// A real chunked HTTP response with deliberately controlled header presence.
/// One-byte HTTP chunks exercise boundaries inside JSON strings and UTF-8.
async fn raw_subscription_stream(
    body: String,
    content_type: Option<&str>,
) -> (String, tokio::task::JoinHandle<()>) {
    raw_subscription_stream_with_chunks(body, content_type, 1).await
}

async fn raw_subscription_stream_with_chunks(
    body: String,
    content_type: Option<&str>,
    chunk_bytes: usize,
) -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::AsyncWriteExt;
    assert_ne!(chunk_bytes, 0);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let content_type = content_type
        .map(|value| format!("Content-Type: {value}\r\n"))
        .unwrap_or_default();
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            let headers = format!(
                "HTTP/1.1 200 OK\r\n{content_type}Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            for bytes in body.as_bytes().chunks(chunk_bytes) {
                // Invalid content can be rejected before the server finishes.
                if socket
                    .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
                    .await
                    .is_err()
                    || socket.write_all(bytes).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }).await.expect("raw subscription fixture exceeded its bound");
    });
    (base, task)
}

#[tokio::test]
async fn fragmented_discarded_subscription_sse_does_not_exhaust_retained_output_budget() {
    let peer = Peer::new(vec![]).await;
    let (mut provider, _manager, _directory) = subscription(&peer).await;
    let discarded = ": ignored framing\n".repeat(crate::MAX_BYTES / 17 + 1);
    let body = format!("{discarded}\n{}", message("small completion").body);
    let (base, server) = raw_subscription_stream_with_chunks(body, None, 1024).await;
    let _abort = AbortOnDrop(server.abort_handle());
    provider.base = base;
    let result = tokio::time::timeout(Duration::from_secs(5), provider.complete(request()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.text_projection(), "small completion");
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn missing_content_type_accepts_fragmented_subscription_sse() {
    let peer = Peer::new(vec![]).await;
    let (mut provider, _manager, _directory) = subscription(&peer).await;
    let body = stream(vec![
        done(0, json!({"type":"message","content":[{"type":"output_text","text":"réponse"}]})),
        done(1, json!({"type":"function_call","id":"item-one","call_id":"call-one","name":"file_read","arguments":"{\"path\":\"one.txt\"}"})),
        completed(),
    ]).body;
    let (base, server) = raw_subscription_stream(body, None).await;
    let _abort = AbortOnDrop(server.abort_handle());
    provider.base = base;
    let result = tokio::time::timeout(Duration::from_secs(5), provider.complete(request()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.text_projection(), "réponse");
    assert_eq!((result.input_tokens(), result.output_tokens()), (8, 5));
    let calls = result.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call-one");
    assert_eq!(calls[0].arguments, json!({"path":"one.txt"}));
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        peer.requests.lock().await.is_empty(),
        "stream acceptance attempted refresh"
    );
}

#[tokio::test]
async fn missing_content_type_still_rejects_malformed_and_truncated_streams() {
    let partial = stream(vec![
        done(0, json!({"type":"reasoning","id":"partial-reasoning","encrypted_content":"unaccepted-opaque"})),
        done(1, json!({"type":"function_call","id":"partial-item","call_id":"partial-call","name":"file_read","arguments":"{}"})),
    ]).body;
    for (body, content_type, expected) in [
        (
            format!("{partial}data: not-json\n\n"),
            None,
            "invalid protocol",
        ),
        (partial, None, "before response.completed"),
        (
            json!({"output":[],"status":"completed"}).to_string(),
            None,
            "before response.completed",
        ),
        (
            message("valid SSE with wrong declared type").body,
            Some("application/json"),
            "event stream",
        ),
    ] {
        let peer = Peer::new(vec![message("clean request")]).await;
        let (mut provider, _manager, _directory) = subscription(&peer).await;
        let (base, server) = raw_subscription_stream(body, content_type).await;
        let _abort = AbortOnDrop(server.abort_handle());
        provider.base = base;
        let error = tokio::time::timeout(Duration::from_secs(5), provider.complete(request()))
            .await
            .unwrap()
            .unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert!(
            peer.requests.lock().await.is_empty(),
            "invalid stream attempted a retry"
        );
        provider.base = peer.url.clone();
        assert_eq!(
            provider
                .complete(request())
                .await
                .unwrap()
                .text_projection(),
            "clean request"
        );
        let sent = peer.requests.lock().await;
        assert_eq!(sent.len(), 1);
        let input = sent[0].body.to_string();
        assert!(!input.contains("unaccepted-opaque"));
        assert!(!input.contains("partial-call"));
    }
}
