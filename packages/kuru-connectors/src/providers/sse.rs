//! Bounded Responses SSE decoding. Only completed items become actor context;
//! deltas are presentation fragments, never a second copy of model output.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::MAX_BYTES;

pub(super) async fn response(mut response: reqwest::Response, idle: Duration) -> Result<Value> {
    ensure!(
        response.status().is_success(),
        "HTTP request failed: {}",
        response.status()
    );
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_BYTES as u64,
        "Responses stream exceeds size limit"
    );
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    ensure!(
        content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream")),
        "ChatGPT response is not an event stream"
    );
    let mut decoder = Decoder::default();
    loop {
        let chunk = tokio::time::timeout(idle, response.chunk())
            .await
            .context("Responses stream exceeded idle timeout")??;
        let Some(chunk) = chunk else {
            bail!("Responses stream closed before response.completed");
        };
        if let Some(value) = decoder.push(&chunk)? {
            return Ok(value);
        }
    }
}

#[derive(Default)]
struct Decoder {
    bytes: usize,
    line: Vec<u8>,
    data: String,
    after_cr: bool,
    started: bool,
    items: BTreeMap<usize, Value>,
    ids: BTreeSet<String>,
}

impl Decoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Option<Value>> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .context("Responses stream size overflow")?;
        ensure!(
            self.bytes <= MAX_BYTES,
            "Responses stream exceeds size limit"
        );
        for &byte in bytes {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if matches!(byte, b'\n' | b'\r') {
                self.after_cr = byte == b'\r';
                if let Some(value) = self.end_line()? {
                    return Ok(Some(value));
                }
            } else {
                self.line.push(byte);
            }
        }
        Ok(None)
    }

    fn end_line(&mut self) -> Result<Option<Value>> {
        let line = std::mem::take(&mut self.line);
        let line = std::str::from_utf8(&line).context("Responses event contains invalid UTF-8")?;
        let line = if self.started {
            line
        } else {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        };
        self.started = true;
        if line.is_empty() {
            if self.data.is_empty() {
                return Ok(None);
            }
            let data = std::mem::take(&mut self.data);
            let event: Value =
                serde_json::from_str(&data).context("invalid Responses event JSON")?;
            return self.event(event);
        }
        if let Some(value) = line.strip_prefix("data:") {
            self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
            self.data.push('\n');
        } else if line == "data" {
            self.data.push('\n');
        }
        // Comments, event labels, retry and id fields do not alter the JSON
        // protocol. Unknown JSON event types similarly grant no tool authority.
        Ok(None)
    }

    fn event(&mut self, event: Value) -> Result<Option<Value>> {
        match event["type"]
            .as_str()
            .context("Responses event lacks type")?
        {
            "response.output_item.done" => {
                let index = event["output_index"]
                    .as_u64()
                    .context("completed output lacks index")?;
                let index = usize::try_from(index).context("invalid output index")?;
                let item = event
                    .get("item")
                    .filter(|value| value.is_object())
                    .context("completed output lacks item")?;
                ensure!(item["type"].is_string(), "completed output lacks type");
                ensure!(
                    !self.items.contains_key(&index),
                    "duplicate completed output index"
                );
                if let Some(id) = item["id"].as_str() {
                    ensure!(
                        self.ids.insert(id.to_owned()),
                        "duplicate completed output ID"
                    );
                }
                self.items.insert(index, item.clone());
            }
            "response.completed" => {
                let mut response = event
                    .get("response")
                    .filter(|value| value.is_object())
                    .context("completed event lacks response")?
                    .clone();
                ensure!(
                    response["id"].as_str().is_some_and(|id| !id.is_empty()),
                    "completed response lacks ID"
                );
                ensure!(
                    response["status"].is_null() || response["status"] == "completed",
                    "response did not complete successfully"
                );
                ensure!(
                    response["error"].is_null(),
                    "Responses completed event contains an error"
                );
                ensure!(
                    self.items.keys().copied().eq(0..self.items.len()),
                    "streamed output indexes are incomplete"
                );
                let items: Vec<_> = std::mem::take(&mut self.items).into_values().collect();
                if let Some(output) = response.get("output") {
                    let output = output
                        .as_array()
                        .context("completed response has invalid output")?;
                    if !items.is_empty() && !output.is_empty() {
                        ensure!(
                            output == &items,
                            "completed response disagrees with streamed output"
                        );
                    }
                }
                if !items.is_empty() || response.get("output").is_none() {
                    response["output"] = json!(items);
                }
                response["status"] = json!("completed");
                return Ok(Some(response));
            }
            // Do not echo remote messages: they can contain tokens or actor
            // context. The event classification is sufficient for this error.
            "response.failed" => bail!("Responses stream reported response.failed"),
            "response.incomplete" => bail!("Responses stream reported response.incomplete"),
            "error" => bail!("Responses stream reported an error"),
            _ => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_utf8_crlf_multiline_data_and_completed_items() {
        let bytes = concat!(
            "\u{feff}:keepalive\r\n\r\n",
            "event: ignored-label\r\ndata: {\"type\":\"response.output_item.done\",\r\ndata: \"output_index\":0,\"item\":{\"type\":\"message\",\"content\":[{\"text\":\"échec\"}]}}\r\n\r\n",
            "data: {\"type\":\"future.metadata\",\"ignored\":true}\r\r",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"échec\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"usage\":{\"input_tokens\":7,\"output_tokens\":2}}}\n\n"
        ).as_bytes();
        for width in [1, 2, 3, 11, bytes.len()] {
            let mut decoder = Decoder::default();
            let mut result = None;
            for part in bytes.chunks(width) {
                if let Some(value) = decoder.push(part).unwrap() {
                    assert!(result.replace(value).is_none());
                }
            }
            let result = result.unwrap();
            assert_eq!(result["output"].as_array().unwrap().len(), 1);
            assert_eq!(result["output"][0]["content"][0]["text"], "échec");
            assert_eq!(result["usage"]["input_tokens"], 7);
        }
    }

    #[test]
    fn rejects_malformed_duplicate_and_failed_events_without_echoing_secrets() {
        for data in [
            "not-json".to_owned(),
            json!({"item":{}}).to_string(),
            json!({"type":"response.output_item.done","output_index":-1,"item":{}}).to_string(),
            json!({"type":"response.completed","response":{"id":"r","status":"incomplete"}}).to_string(),
            json!({"type":"response.completed","response":{"id":"r","output":"invalid"}}).to_string(),
            json!({"type":"response.failed","response":{"error":{"message":"secret-sentinel"}}}).to_string(),
            json!({"type":"response.incomplete","response":{"error":{"message":"secret-sentinel"}}}).to_string(),
            json!({"type":"error","message":"secret-sentinel"}).to_string(),
        ] {
            let error = Decoder::default().push(format!("data: {data}\n\n").as_bytes()).unwrap_err();
            assert!(!format!("{error:#}").contains("secret-sentinel"));
        }
        let item = json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"i","encrypted_content":"opaque"}});
        let mut decoder = Decoder::default();
        let event = format!("data: {item}\n\n");
        decoder.push(event.as_bytes()).unwrap();
        assert!(
            decoder
                .push(event.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
        assert!(Decoder::default().push(b"data: \xff\n\n").is_err());
        assert!(
            Decoder::default()
                .push(&vec![b'x'; MAX_BYTES + 1])
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
    }
}
