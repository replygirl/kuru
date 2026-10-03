//! A local HTTP/1.1 fixture whose response pacing is controlled, for release
//! download timeout tests. Each server answers at most one connection and is
//! bounded so a wrong client timeout fails the test instead of hanging it.

use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use url::Url;

pub(crate) const CHUNK: &[u8] = b"paced release bytes\n";
const FIXTURE_BOUND: Duration = Duration::from_secs(15);

pub(crate) enum Pace {
    /// Send the complete body as `chunks` writes separated by `gap`.
    Trickle { chunks: usize, gap: Duration },
    /// Send the headers and one chunk, then never send the rest.
    StallBody,
}

pub(crate) struct PacedServer {
    pub(crate) url: Url,
    // A listener that is held without accepting: the kernel backlog completes
    // the TCP handshake, so the client waits for a response that never comes.
    _held: Option<TcpListener>,
    task: Option<JoinHandle<()>>,
}

impl PacedServer {
    pub(crate) async fn start(pace: Pace) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = url(&listener);
        let task = tokio::spawn(async move {
            tokio::time::timeout(FIXTURE_BOUND, serve(listener, pace))
                .await
                .expect("bounded paced HTTP fixture");
        });
        Self {
            url,
            _held: None,
            task: Some(task),
        }
    }

    pub(crate) async fn never_accepting() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Self {
            url: url(&listener),
            _held: Some(listener),
            task: None,
        }
    }

    /// A URL whose port had a listener that is now closed, so connecting is
    /// refused.
    pub(crate) async fn refused() -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        url(&listener)
    }

    pub(crate) fn body(chunks: usize) -> Vec<u8> {
        CHUNK.repeat(chunks)
    }
}

impl Drop for PacedServer {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

fn url(listener: &TcpListener) -> Url {
    Url::parse(&format!(
        "http://{}/release.asset",
        listener.local_addr().unwrap()
    ))
    .unwrap()
}

async fn serve(listener: TcpListener, pace: Pace) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        request.push(socket.read_u8().await.unwrap());
        assert!(request.len() < 8192, "unbounded fixture request");
    }
    let chunks = match pace {
        Pace::Trickle { chunks, .. } => chunks,
        Pace::StallBody => 4,
    };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        CHUNK.len() * chunks
    );
    socket.write_all(header.as_bytes()).await.unwrap();
    match pace {
        Pace::Trickle { chunks, gap } => {
            for _ in 0..chunks {
                // The client may legitimately give up (the flat-total client
                // does); stop pacing once it has gone.
                if socket.write_all(CHUNK).await.is_err() {
                    return;
                }
                tokio::time::sleep(gap).await;
            }
        }
        Pace::StallBody => {
            socket.write_all(CHUNK).await.unwrap();
            // Hold the connection until the client abandons the body.
            let mut byte = [0];
            let _ = socket.read(&mut byte).await;
        }
    }
}
