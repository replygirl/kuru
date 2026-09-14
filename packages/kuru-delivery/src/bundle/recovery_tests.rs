use super::{
    tests::{assert_clean, asset, options},
    *,
};
use kuru_platform::fs::{FileIdentity, regular_file_info};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
    task::JoinHandle,
};

const EXPECTED: &[u8] = b"verified immutable archive";

enum Reply {
    Bytes(Vec<u8>),
    StallHeaders,
    StallBody,
    DropBody,
    ObserveErrorDrop,
}

enum Event {
    Request,
    ErrorDropped,
}

fn response(status: u16, extra: &str, body: &[u8]) -> Reply {
    let mut bytes = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    Reply::Bytes(bytes)
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    events: mpsc::UnboundedReceiver<Event>,
    task: Option<JoinHandle<()>>,
}

impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/immutable.archive",
            listener.local_addr().unwrap()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let (events, received) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(15), async {
                let mut replies = replies.into_iter();
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        request.push(socket.read_u8().await.unwrap());
                        assert!(request.len() < 8192);
                    }
                    {
                        let mut captured = captured.lock().unwrap();
                        captured.push(request);
                        assert!(captured.len() <= 8, "unbounded fixture requests");
                    }
                    let _ = events.send(Event::Request);
                    // Unexpected retries encounter a valid body, making a
                    // forbidden retry observable as success as well as a count.
                    match replies.next().unwrap_or_else(|| response(200, "", EXPECTED)) {
                        Reply::Bytes(bytes) => { let _ = socket.write_all(&bytes).await; }
                        Reply::StallHeaders => std::future::pending::<()>().await,
                        Reply::StallBody => {
                            // Wait until the client cancels this body read before
                            // accepting the bounded retry on the same owned server.
                            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nveri", EXPECTED.len());
                            socket.write_all(header.as_bytes()).await.unwrap();
                            let mut byte = [0];
                            let _ = socket.read(&mut byte).await;
                        }
                        Reply::DropBody => {
                            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nveri", EXPECTED.len());
                            socket.write_all(header.as_bytes()).await.unwrap();
                        }
                        Reply::ObserveErrorDrop => {
                            // Never supply this unsuccessful response's body.
                            // The client must close it before waiting to retry.
                            socket.write_all(b"HTTP/1.1 500 Fixture\r\nContent-Length: 1024\r\nConnection: close\r\n\r\n").await.unwrap();
                            let mut byte = [0];
                            match socket.read(&mut byte).await {
                                Ok(0) => (),
                                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => (),
                                result => panic!("unsuccessful response was not closed: {result:?}"),
                            }
                            let _ = events.send(Event::ErrorDropped);
                        }
                    }
                }
            }).await.expect("bounded local HTTP fixture");
        });
        Self {
            url,
            requests,
            events: received,
            task: Some(task),
        }
    }

    async fn stop(&mut self) -> Vec<Vec<u8>> {
        let task = self.task.take().unwrap();
        task.abort();
        if let Err(error) = task.await {
            assert!(error.is_cancelled(), "HTTP fixture failed: {error}");
        }
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn setup(server: &Server) -> (tempfile::TempDir, PrepareOptions, Asset, FileIdentity) {
    let root = tempfile::tempdir().unwrap();
    let options = options(root.path());
    let directory = Directory::open(&options.bundle_dir, true, true).unwrap();
    let lock = directory.lock().await.unwrap();
    let identity = regular_file_info(&lock).unwrap().identity;
    drop(lock);
    let mut asset = asset(EXPECTED);
    asset.url = server.url.clone();
    (root, options, asset, identity)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

fn client_with_read_idle(read_idle: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(1))
        .read_timeout(read_idle)
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
}

fn retained_lock(options: &PrepareOptions, identity: FileIdentity) {
    let lock = File::options()
        .read(true)
        .write(true)
        .open(options.bundle_dir.join(LOCK_NAME))
        .unwrap();
    assert_eq!(regular_file_info(&lock).unwrap().identity, identity);
    lock.try_lock().unwrap();
}

fn identical_gets(requests: &[Vec<u8>], count: usize) {
    assert_eq!(requests.len(), count);
    assert!(requests[0].starts_with(b"GET /immutable.archive HTTP/1.1\r\n"));
    assert!(requests.iter().all(|request| request == &requests[0]));
}

#[tokio::test]
async fn eligible_statuses_recover_on_the_third_get_and_persistent_errors_stop() {
    for status in [500, 502, 503, 504] {
        let mut server = Server::start(vec![
            response(status, "", b"error one"),
            response(status, "", b"error two"),
            response(200, "", EXPECTED),
        ])
        .await;
        let (_root, options, asset, identity) = setup(&server).await;
        let result = tokio::time::timeout(
            Duration::from_secs(8),
            prepare_asset(&options, &asset, Some(&client())),
        )
        .await;
        identical_gets(&server.stop().await, 3);
        let published = result.unwrap().unwrap();
        assert_eq!(std::fs::read(&published).unwrap(), EXPECTED);
        assert_eq!(std::fs::read_dir(&options.bundle_dir).unwrap().count(), 2);
        retained_lock(&options, identity);
    }
    let mut server = Server::start(
        (0..3)
            .map(|_| response(503, "", b"still unavailable"))
            .collect(),
    )
    .await;
    let (_root, options, asset, identity) = setup(&server).await;
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        prepare_asset(&options, &asset, Some(&client())),
    )
    .await;
    identical_gets(&server.stop().await, 3);
    assert!(format!("{:#}", result.unwrap().unwrap_err()).contains("503"));
    assert_clean(&options.bundle_dir);
    retained_lock(&options, identity);
}

#[tokio::test]
async fn permanent_statuses_and_any_retry_after_are_not_retried() {
    for (status, advice) in [
        (400, ""),
        (401, ""),
        (403, ""),
        (404, ""),
        (408, ""),
        (429, ""),
        (501, ""),
        (505, ""),
        (500, "Retry-After: 0\r\n"),
        (502, "Retry-After: Wed, 21 Oct 2015 07:28:00 GMT\r\n"),
        (503, "rEtRy-AfTeR: unknown\r\n"),
        (504, "Retry-After:\r\n"),
    ] {
        let mut server = Server::start(vec![response(status, advice, EXPECTED)]).await;
        let (_root, options, asset, identity) = setup(&server).await;
        let result = tokio::time::timeout(
            Duration::from_secs(4),
            prepare_asset(&options, &asset, Some(&client())),
        )
        .await;
        identical_gets(&server.stop().await, 1);
        assert!(format!("{:#}", result.unwrap().unwrap_err()).contains(&status.to_string()));
        assert_clean(&options.bundle_dir);
        retained_lock(&options, identity);
    }
}

#[tokio::test]
async fn malformed_response_and_integrity_failures_never_start_another_get() {
    let mut corrupt = EXPECTED.to_vec();
    corrupt[0] ^= 1;
    let oversized = [EXPECTED, b"!"].concat();
    let mut chunked = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
        oversized.len()
    )
    .into_bytes();
    chunked.extend_from_slice(&oversized);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    for (label, reply) in [
        ("malformed response", Reply::Bytes(b"this is not an HTTP response\r\n\r\n".to_vec())),
        ("digest mismatch", response(200, "", &corrupt)),
        ("declared short payload", response(200, "", b"short")),
        ("oversized chunk", Reply::Bytes(chunked)),
        ("chunked short payload", Reply::Bytes(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nshort\r\n0\r\n\r\n".to_vec())),
    ] {
        let mut server = Server::start(vec![reply]).await;
        let (_root, options, asset, identity) = setup(&server).await;
        let result = tokio::time::timeout(Duration::from_secs(4), prepare_asset(&options, &asset, Some(&client()))).await;
        assert_eq!(server.stop().await.len(), 1, "unexpected retry for {label}");
        assert!(result.unwrap().is_err());
        assert_clean(&options.bundle_dir);
        retained_lock(&options, identity);
    }
}

#[tokio::test]
async fn timed_out_body_frame_is_typed() {
    let mut server = Server::start(vec![Reply::StallBody]).await;
    let mut response = client_with_read_idle(Duration::from_millis(500))
        .get(&server.url)
        .send()
        .await
        .unwrap();
    assert_eq!(response.chunk().await.unwrap().unwrap(), b"veri".as_slice());
    let error = response.chunk().await.unwrap_err();
    assert!(
        error.is_timeout(),
        "body error was not typed as a timeout: {error}"
    );
    assert!(
        error.is_decode(),
        "body error was not a nested decode error: {error}"
    );
    drop(response);
    identical_gets(&server.stop().await, 1);
}

#[tokio::test]
async fn timed_out_body_frame_retries_with_a_clean_stage() {
    let mut server = Server::start(vec![Reply::StallBody, response(200, "", EXPECTED)]).await;
    let (_root, options, asset, identity) = setup(&server).await;
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        prepare_asset_with_budget(
            &options,
            &asset,
            Some(&client_with_read_idle(Duration::from_millis(500))),
            Duration::from_secs(2),
        ),
    )
    .await;
    identical_gets(&server.stop().await, 2);
    let published = result.unwrap().unwrap();
    assert_eq!(std::fs::read(&published).unwrap(), EXPECTED);
    assert_eq!(std::fs::read_dir(&options.bundle_dir).unwrap().count(), 2);
    retained_lock(&options, identity);
}

#[tokio::test]
async fn interrupted_body_retries_only_three_times_and_releases_the_lock() {
    let mut server = Server::start(vec![
        response(503, "", b"unavailable"),
        Reply::DropBody,
        Reply::DropBody,
    ])
    .await;
    let (_root, options, asset, identity) = setup(&server).await;
    let result = tokio::time::timeout(
        Duration::from_secs(4),
        prepare_asset_with_budget(&options, &asset, Some(&client()), Duration::from_secs(3)),
    )
    .await;
    identical_gets(&server.stop().await, 3);
    let error = result.unwrap().unwrap_err();
    assert!(format!("{error:#}").contains("body read failed"));
    assert_clean(&options.bundle_dir);
    retained_lock(&options, identity);
}

#[tokio::test]
async fn one_download_deadline_covers_all_backoffs_headers_and_body() {
    for (replies, budget) in [
        (
            vec![
                response(500, "", b"first"),
                response(503, "", b"second"),
                response(200, "", EXPECTED),
            ],
            Duration::from_secs(1),
        ),
        (
            vec![response(500, "", b"first"), Reply::StallHeaders],
            Duration::from_secs(2),
        ),
        (
            vec![response(500, "", b"first"), Reply::StallBody],
            Duration::from_secs(2),
        ),
    ] {
        let mut server = Server::start(replies).await;
        let (_root, options, asset, identity) = setup(&server).await;
        // The client's ten-second per-request timer cannot satisfy this test.
        let result = tokio::time::timeout(
            Duration::from_secs(4),
            prepare_asset_with_budget(&options, &asset, Some(&client()), budget),
        )
        .await;
        identical_gets(&server.stop().await, 2);
        assert!(
            format!("{:#}", result.unwrap().unwrap_err())
                .contains("bundle archive download timed out")
        );
        assert_clean(&options.bundle_dir);
        retained_lock(&options, identity);
    }
}

#[tokio::test]
async fn cancellation_after_dropping_error_headers_releases_stage_and_stable_lock() {
    let mut server = Server::start(vec![Reply::ObserveErrorDrop]).await;
    let (_root, options, asset, identity) = setup(&server).await;
    let client = client();
    let mut preparation = Box::pin(prepare_asset(&options, &asset, Some(&client)));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut preparation => panic!("preparation completed before cancellation: {result:?}"),
                event = server.events.recv() => match event.unwrap() {
                    Event::Request => (),
                    Event::ErrorDropped => break,
                },
            }
        }
    }).await.unwrap();
    let lock = File::options()
        .read(true)
        .write(true)
        .open(options.bundle_dir.join(LOCK_NAME))
        .unwrap();
    assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
    assert!(
        std::fs::read_dir(&options.bundle_dir)
            .unwrap()
            .any(|entry| {
                let path = entry.unwrap().path();
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".bundle-")
                    && std::fs::metadata(path.join("archive")).unwrap().len() == 0
            })
    );
    drop(lock);
    drop(preparation);
    assert_clean(&options.bundle_dir);
    retained_lock(&options, identity);
    // A detached first-backoff retry would reconnect after 250 ms. Keep the
    // real listener available beyond it and reject any subsequent request.
    assert!(
        tokio::time::timeout(Duration::from_millis(600), server.events.recv())
            .await
            .is_err()
    );
    identical_gets(&server.stop().await, 1);
}
