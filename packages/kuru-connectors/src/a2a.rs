use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::{MAX_BYTES, http};

/// Nonstreaming A2A 1.0 JSON-RPC SendMessage. The caller supplies an explicitly
/// configured agent endpoint and permitted message context, never actor memory.
pub async fn a2a_send(url: &str, message: &str, context_id: &str) -> Result<String> {
    http::endpoint(url)?;
    ensure!(
        message.len() <= MAX_BYTES / 2,
        "A2A message exceeds size limit"
    );
    let id = json!(uuid::Uuid::new_v4().to_string());
    let request = json!({"jsonrpc":"2.0","id":id,"method":"SendMessage","params":{"message":{"messageId":uuid::Uuid::new_v4().to_string(),"contextId":context_id,"role":"ROLE_USER","parts":[{"text":message}]},"configuration":{"acceptedOutputModes":["text/plain"],"returnImmediately":false}}});
    let response = http::json(
        http::client()?
            .post(url)
            .header("A2A-Version", "1.0")
            .json(&request)
            .send()
            .await?,
    )
    .await?;
    extract(http::rpc_result(response, &id)?)
}

fn parts(value: &Value) -> Vec<String> {
    value["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn extract(result: Value) -> Result<String> {
    ensure!(
        result.get("message").is_some() != result.get("task").is_some(),
        "A2A SendMessage must return exactly one message or task"
    );
    if let Some(message) = result.get("message") {
        let text = parts(message);
        ensure!(!text.is_empty(), "A2A response has no text parts");
        return Ok(text.join("\n"));
    }
    let task = &result["task"];
    let state = task["status"]["state"]
        .as_str()
        .context("A2A task lacks status state")?;
    ensure!(
        matches!(
            state,
            "TASK_STATE_COMPLETED" | "TASK_STATE_INPUT_REQUIRED" | "TASK_STATE_AUTH_REQUIRED"
        ),
        "A2A task ended in {state}"
    );
    let mut text = parts(&task["status"]["message"]);
    if let Some(artifacts) = task["artifacts"].as_array() {
        for artifact in artifacts {
            text.extend(parts(artifact));
        }
    }
    ensure!(
        !text.is_empty(),
        "A2A task returned no text; state={state}, task={}",
        task["id"]
    );
    Ok(text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{HttpFixture, Reply};

    #[tokio::test]
    async fn sends_v1_jsonrpc_and_reads_message_or_task() {
        let peer = HttpFixture::new(vec![Reply::rpc(json!({"message":{"parts":[{"text":"hello"},{"text":"peer"}]}})),Reply::rpc(json!({"task":{"id":"task-1","status":{"state":"TASK_STATE_COMPLETED"},"artifacts":[{"parts":[{"text":"result"}]}]}}))]).await;
        assert_eq!(
            a2a_send(&peer.url, "request", "context-1").await.unwrap(),
            "hello\npeer"
        );
        assert_eq!(
            a2a_send(&peer.url, "again", "context-1").await.unwrap(),
            "result"
        );
        let requests = peer.requests.lock().await;
        assert_eq!(requests[0].headers["a2a-version"], "1.0");
        assert_eq!(requests[0].body["method"], "SendMessage");
        assert_eq!(requests[0].body["params"]["message"]["role"], "ROLE_USER");
        assert_eq!(
            requests[0].body["params"]["message"]["contextId"],
            "context-1"
        );
        assert_eq!(
            requests[0].body["params"]["message"]["parts"][0],
            json!({"text":"request"})
        );
        assert_ne!(
            requests[0].body["params"]["message"]["messageId"],
            requests[1].body["params"]["message"]["messageId"]
        );
    }

    #[test]
    fn task_failures_and_nontext_output_are_explicit() {
        for result in [
            json!({}),
            json!({"message":{},"task":{}}),
            json!({"message":{"parts":[{"raw":"abc"}]}}),
            json!({"task":{}}),
            json!({"task":{"status":{"state":"TASK_STATE_FAILED"}}}),
            json!({"task":{"status":{"state":"TASK_STATE_COMPLETED"}}}),
        ] {
            assert!(extract(result).is_err());
        }
        assert_eq!(extract(json!({"task":{"status":{"state":"TASK_STATE_INPUT_REQUIRED","message":{"parts":[{"text":"Need details"}]}}}})).unwrap(),"Need details");
    }

    #[tokio::test]
    async fn validates_endpoints_and_outbound_size() {
        assert!(a2a_send("file:///tmp/a", "hello", "c").await.is_err());
        assert!(
            a2a_send("http://localhost:1", &"a".repeat(MAX_BYTES), "c")
                .await
                .is_err()
        );
    }
}
