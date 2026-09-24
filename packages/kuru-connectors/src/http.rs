use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Response};
use serde_json::Value;

use crate::{IO_TIMEOUT, MAX_BYTES};

pub(crate) fn client() -> Result<Client> {
    let builder = Client::builder()
        .timeout(IO_TIMEOUT)
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    #[cfg(feature = "test-support")]
    let builder = with_fixture_root(builder)?;
    Ok(builder.build()?)
}

// Only owned test children may add a synthetic CA. This keeps the normal
// platform verifier, HTTPS requirement and hostname checks in place.
#[cfg(feature = "test-support")]
fn with_fixture_root(builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder> {
    use std::io::Read;

    let Some(path) = std::env::var_os("KURU_TEST_MCP_CA_PEM") else {
        return Ok(builder);
    };
    const MAX_FIXTURE_PEM_BYTES: u64 = 16 * 1024;
    let mut pem = Vec::new();
    std::fs::File::open(path)
        .context("open synthetic MCP test CA")?
        .take(MAX_FIXTURE_PEM_BYTES + 1)
        .read_to_end(&mut pem)
        .context("read synthetic MCP test CA")?;
    ensure!(
        !pem.is_empty() && pem.len() as u64 <= MAX_FIXTURE_PEM_BYTES,
        "synthetic MCP test CA is empty or too large"
    );
    let root = reqwest::Certificate::from_pem(&pem).context("invalid synthetic MCP test CA")?;
    Ok(builder.tls_certs_merge([root]))
}

pub(crate) fn endpoint(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).context("invalid endpoint URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "endpoint must use HTTP(S)"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URL credentials are unsupported; use explicit authentication configuration"
    );
    ensure!(url.host_str().is_some(), "endpoint requires a host");
    Ok(url)
}

pub(crate) async fn body(mut response: Response) -> Result<Vec<u8>> {
    ensure!(
        response.status().is_success(),
        "HTTP request failed: {}",
        response.status()
    );
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_BYTES as u64,
        "HTTP response exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "HTTP response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) async fn json(response: Response) -> Result<Value> {
    serde_json::from_slice(&body(response).await?).context("invalid JSON response")
}

pub(crate) fn rpc_result(value: Value, id: &Value) -> Result<Value> {
    ensure!(value.get("id") == Some(id), "JSON-RPC response ID mismatch");
    if let Some(error) = value.get("error") {
        // Error data may contain credentials/headers echoed by a remote server.
        bail!(
            "JSON-RPC error {}: {}",
            error["code"],
            error["message"].as_str().unwrap_or("unknown error")
        );
    }
    value
        .get("result")
        .cloned()
        .context("JSON-RPC response lacks result")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{HttpFixture, Reply};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn urls_and_rpc_envelopes_are_validated() {
        for url in [
            "not a URL",
            "file:///tmp/a",
            "http://user:password@localhost/",
        ] {
            assert!(endpoint(url).is_err());
        }
        assert!(endpoint("https://example.com/rpc").is_ok());
        assert!(rpc_result(json!({"id":2,"result":{}}), &json!(1)).is_err());
        assert!(rpc_result(json!({"id":1}), &json!(1)).is_err());
        assert!(
            rpc_result(
                json!({"id":1,"error":{"code":-1,"message":"failure","data":{"token":"private"}}}),
                &json!(1)
            )
            .unwrap_err()
            .to_string()
            .contains("failure")
        );
        assert_eq!(
            rpc_result(json!({"id":1,"result":{"ok":true}}), &json!(1)).unwrap()["ok"],
            true
        );
    }

    #[tokio::test]
    async fn body_rejects_http_failure_invalid_json_and_size_limit() {
        let mut error = Reply::json(json!({"secret":"must not appear in HTTP error"}));
        error.status = axum::http::StatusCode::UNAUTHORIZED;
        let mut invalid = Reply::json(json!({}));
        invalid.body = "not JSON".into();
        let mut large = Reply::json(json!({}));
        large.body = "x".repeat(MAX_BYTES + 1);
        let peer = HttpFixture::new(vec![error, invalid, large]).await;
        let client = client().unwrap();
        let failure = body(client.get(&peer.url).send().await.unwrap())
            .await
            .unwrap_err()
            .to_string();
        assert!(failure.contains("401") && !failure.contains("secret"));
        assert!(
            json(client.get(&peer.url).send().await.unwrap())
                .await
                .is_err()
        );
        assert!(
            body(client.get(&peer.url).send().await.unwrap())
                .await
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
    }

    #[tokio::test]
    async fn chunked_response_is_bounded_without_content_length() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            for _ in 0..3 {
                if socket.write_all(b"100000\r\n").await.is_err() {
                    return;
                }
                if socket.write_all(&vec![b'x'; 1024 * 1024]).await.is_err() {
                    return;
                }
                if socket.write_all(b"\r\n").await.is_err() {
                    return;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        });
        assert!(
            body(client().unwrap().get(url).send().await.unwrap())
                .await
                .unwrap_err()
                .to_string()
                .contains("size limit")
        );
        task.await.unwrap();
    }
}
