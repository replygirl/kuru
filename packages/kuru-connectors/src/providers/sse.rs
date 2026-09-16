//! Bounded Responses SSE decoding. Only completed items become actor context;
//! deltas are presentation fragments, never a second copy of model output.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::MAX_BYTES;

use super::{
    ProviderEvent, ProviderFailureKind, ProviderSink, TextDeltaSource,
    diagnostics::{self, Operation},
};

// The retained result remains Kuru's 2 MiB protocol limit. SSE transport can
// contain independently discarded deltas and framing, but never grows without
// a bounded wire, event, or line budget.
const MAX_SSE_WIRE_BYTES: usize = MAX_BYTES * 32;
const MAX_SSE_EVENT_BYTES: usize = MAX_BYTES * 2;
const MAX_SSE_LINE_BYTES: usize = MAX_SSE_EVENT_BYTES;

pub(super) async fn response(
    response: reqwest::Response,
    idle: Duration,
    operation: Operation,
    allow_missing_content_type: bool,
    sink: &mut dyn ProviderSink,
) -> Result<Value> {
    let mut response = diagnostics::successful(response, operation).await?;
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_SSE_WIRE_BYTES as u64,
        "Responses stream exceeds wire size limit"
    );
    // The subscription endpoint can omit Content-Type. The bounded SSE
    // records and successful terminal event remain mandatory in that case.
    if let Some(header) = response.headers().get(reqwest::header::CONTENT_TYPE) {
        let content_type = header.to_str().unwrap_or("");
        ensure!(
            content_type
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream")),
            "ChatGPT response is not an event stream"
        );
    } else {
        ensure!(
            allow_missing_content_type,
            "Responses API response is not an event stream"
        );
    }
    let mut decoder = Decoder::default();
    loop {
        let chunk = tokio::time::timeout(idle, response.chunk())
            .await
            .context("Responses stream exceeded idle timeout")?
            .map_err(|error| diagnostics::transport(operation, error))?;
        let Some(chunk) = chunk else {
            sink.emit(ProviderEvent::Failed {
                kind: ProviderFailureKind::Incomplete,
            })
            .await?;
            bail!("Responses stream closed before response.completed");
        };
        let value = match decoder.push(&chunk) {
            Ok(value) => value,
            Err(error) => {
                for event in decoder.take_observations() {
                    sink.emit(event).await?;
                }
                return Err(error);
            }
        };
        for event in decoder.take_observations() {
            sink.emit(event).await?;
        }
        if let Some(value) = value {
            return Ok(value);
        }
    }
}

#[derive(Default)]
struct Decoder {
    wire_bytes: usize,
    retained_output_bytes: usize,
    line: Vec<u8>,
    data: String,
    after_cr: bool,
    started: bool,
    items: BTreeMap<usize, Value>,
    ids: BTreeSet<String>,
    observations: Vec<ProviderEvent>,
    text_fragments: BTreeMap<(u64, String, u64, TextDeltaSource), String>,
    summary_fragments: BTreeMap<(u64, String, u64), String>,
    tool_fragments: BTreeMap<(u64, String), String>,
    tool_done: BTreeMap<(u64, String), (String, String)>,
    settled: bool,
}

impl Decoder {
    fn take_observations(&mut self) -> Vec<ProviderEvent> {
        std::mem::take(&mut self.observations)
    }
    fn push(&mut self, bytes: &[u8]) -> Result<Option<Value>> {
        self.wire_bytes = self
            .wire_bytes
            .checked_add(bytes.len())
            .context("Responses stream size overflow")?;
        ensure!(
            self.wire_bytes <= MAX_SSE_WIRE_BYTES,
            "Responses stream exceeds wire size limit"
        );
        let mut completed = None;
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
                    ensure!(
                        completed.replace(value).is_none(),
                        "duplicate terminal stream event"
                    );
                }
            } else {
                self.line.push(byte);
                ensure!(
                    self.line.len() <= MAX_SSE_LINE_BYTES,
                    "Responses stream event line exceeds size limit"
                );
            }
        }
        Ok(completed)
    }

    fn end_line(&mut self) -> Result<Option<Value>> {
        let line = std::mem::take(&mut self.line);
        let line = std::str::from_utf8(&line).map_err(|_| diagnostics::stream_protocol())?;
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
                serde_json::from_str(&data).map_err(|_| diagnostics::stream_protocol())?;
            return self.event(event);
        }
        if let Some(value) = line.strip_prefix("data:") {
            let value = value.strip_prefix(' ').unwrap_or(value);
            let appended = value
                .len()
                .checked_add(1)
                .context("Responses stream event size overflow")?;
            let size = self
                .data
                .len()
                .checked_add(appended)
                .context("Responses stream event size overflow")?;
            ensure!(
                size <= MAX_SSE_EVENT_BYTES,
                "Responses stream event exceeds size limit"
            );
            self.data.push_str(value);
            self.data.push('\n');
        } else if line == "data" {
            ensure!(
                self.data.len() < MAX_SSE_EVENT_BYTES,
                "Responses stream event exceeds size limit"
            );
            self.data.push('\n');
        }
        // Comments, event labels, retry and id fields do not alter the JSON
        // protocol. Unknown JSON event types similarly grant no tool authority.
        Ok(None)
    }

    fn event(&mut self, event: Value) -> Result<Option<Value>> {
        ensure!(
            !self.settled,
            "Responses stream contains an event after its terminal outcome"
        );
        match event["type"]
            .as_str()
            .context("Responses event lacks type")?
        {
            "response.output_text.delta" | "response.refusal.delta" => {
                let item_id = event["item_id"]
                    .as_str()
                    .context("stream text delta lacks item ID")?;
                let output_index = event["output_index"]
                    .as_u64()
                    .context("stream text delta lacks output index")?;
                let content_index = event["content_index"]
                    .as_u64()
                    .context("stream text delta lacks content index")?;
                let text = event["delta"]
                    .as_str()
                    .context("stream text delta lacks text")?;
                ensure!(
                    text.len() <= MAX_BYTES,
                    "Responses stream delta exceeds retained output limit"
                );
                let source = if event["type"] == "response.refusal.delta" {
                    TextDeltaSource::Refusal
                } else {
                    TextDeltaSource::OutputText
                };
                append_fragment(
                    &mut self.text_fragments,
                    (output_index, item_id.into(), content_index, source),
                    text,
                )?;
                self.observations.push(ProviderEvent::TextDelta {
                    item_id: item_id.into(),
                    output_index,
                    content_index,
                    source,
                    text: text.into(),
                });
            }
            "response.reasoning_summary_text.delta" => {
                let item_id = event["item_id"]
                    .as_str()
                    .context("stream summary delta lacks item ID")?;
                let output_index = event["output_index"]
                    .as_u64()
                    .context("stream summary delta lacks output index")?;
                let summary_index = event["summary_index"]
                    .as_u64()
                    .context("stream summary delta lacks summary index")?;
                let text = event["delta"]
                    .as_str()
                    .context("stream summary delta lacks text")?;
                ensure!(
                    text.len() <= MAX_BYTES,
                    "Responses stream delta exceeds retained output limit"
                );
                append_fragment(
                    &mut self.summary_fragments,
                    (output_index, item_id.into(), summary_index),
                    text,
                )?;
                self.observations
                    .push(ProviderEvent::ReasoningSummaryDelta {
                        item_id: item_id.into(),
                        output_index,
                        summary_index,
                        text: text.into(),
                    });
            }
            "response.function_call_arguments.delta" => {
                let item_id = event["item_id"]
                    .as_str()
                    .context("stream function delta lacks item ID")?;
                let output_index = event["output_index"]
                    .as_u64()
                    .context("stream function delta lacks output index")?;
                let arguments_fragment = event["delta"]
                    .as_str()
                    .context("stream function delta lacks arguments")?;
                ensure!(
                    arguments_fragment.len() <= MAX_BYTES,
                    "Responses stream delta exceeds retained output limit"
                );
                append_fragment(
                    &mut self.tool_fragments,
                    (output_index, item_id.into()),
                    arguments_fragment,
                )?;
                self.observations.push(ProviderEvent::ToolCallDelta {
                    item_id: item_id.into(),
                    output_index,
                    arguments_fragment: arguments_fragment.into(),
                });
            }
            "response.function_call_arguments.done" => {
                let item_id = event["item_id"]
                    .as_str()
                    .context("completed function arguments lack item ID")?;
                let output_index = event["output_index"]
                    .as_u64()
                    .context("completed function arguments lack output index")?;
                let arguments = event["arguments"]
                    .as_str()
                    .context("completed function arguments lack arguments")?;
                let name = event["name"]
                    .as_str()
                    .context("completed function arguments lack name")?;
                ensure!(
                    arguments.len() <= MAX_BYTES && name.len() <= MAX_BYTES,
                    "Responses completed function arguments exceed retained output limit"
                );
                let _: Value =
                    serde_json::from_str(arguments).map_err(|_| diagnostics::stream_protocol())?;
                ensure!(
                    self.tool_done
                        .insert(
                            (output_index, item_id.into()),
                            (arguments.into(), name.into())
                        )
                        .is_none(),
                    "duplicate completed function arguments"
                );
            }
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
                let item_bytes = serde_json::to_vec(item)
                    .map_err(|_| diagnostics::stream_protocol())?
                    .len();
                let separator_bytes = if self.items.is_empty() { 2 } else { 1 };
                let retained = self
                    .retained_output_bytes
                    .checked_add(separator_bytes)
                    .and_then(|size| size.checked_add(item_bytes))
                    .context("Responses retained output size overflow")?;
                ensure!(
                    retained <= MAX_BYTES,
                    "Responses retained output exceeds size limit"
                );
                if let Some(id) = item["id"].as_str() {
                    ensure!(
                        self.ids.insert(id.to_owned()),
                        "duplicate completed output ID"
                    );
                }
                self.retained_output_bytes = retained;
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
                let items = std::mem::take(&mut self.items);
                if let Some(output) = response.get("output") {
                    let output = output
                        .as_array()
                        .context("completed response has invalid output")?;
                    let mut final_ids = BTreeSet::new();
                    for item in output {
                        if let Some(id) = item["id"].as_str() {
                            ensure!(final_ids.insert(id), "duplicate completed output ID");
                        }
                    }
                    for (index, item) in &items {
                        let final_item = output
                            .get(*index)
                            .context("completed response omits streamed output")?;
                        ensure!(
                            final_item == item,
                            "completed response disagrees with streamed output"
                        );
                    }
                } else {
                    ensure!(
                        !items.is_empty(),
                        "completed response lacks authoritative output"
                    );
                    ensure!(
                        items.keys().copied().eq(0..items.len()),
                        "streamed output indexes are incomplete"
                    );
                    response["output"] = json!(items.into_values().collect::<Vec<_>>());
                }
                self.reconcile_fragments(&response)?;
                response["status"] = json!("completed");
                let response_bytes = serde_json::to_vec(&response)
                    .map_err(|_| diagnostics::stream_protocol())?
                    .len();
                ensure!(
                    response_bytes <= MAX_BYTES,
                    "Responses retained response exceeds size limit"
                );
                self.settled = true;
                self.observations
                    .push(ProviderEvent::Usage(super::usage(&response)));
                return Ok(Some(response));
            }
            // Do not echo remote messages: they can contain tokens or actor
            // context. The event classification is sufficient for this error.
            "response.failed" => {
                if let Some(response) = event.get("response") {
                    self.observations
                        .push(ProviderEvent::Usage(super::usage(response)));
                }
                self.settled = true;
                self.observations.push(ProviderEvent::Failed {
                    kind: ProviderFailureKind::Failed,
                });
                return Err(diagnostics::stream_event(&event));
            }
            "response.incomplete" => {
                if let Some(response) = event.get("response") {
                    self.observations
                        .push(ProviderEvent::Usage(super::usage(response)));
                }
                self.settled = true;
                self.observations.push(ProviderEvent::Failed {
                    kind: ProviderFailureKind::Incomplete,
                });
                return Err(diagnostics::stream_failed());
            }
            "error" => {
                self.settled = true;
                self.observations.push(ProviderEvent::Failed {
                    kind: ProviderFailureKind::Error,
                });
                return Err(diagnostics::stream_failed());
            }
            _ => {}
        }
        Ok(None)
    }
}

impl Decoder {
    fn reconcile_fragments(&self, response: &Value) -> Result<()> {
        let output = response["output"]
            .as_array()
            .context("completed response lacks output")?;
        for ((index, item_id, content_index, source), fragment) in &self.text_fragments {
            let item = output
                .get(*index as usize)
                .context("stream text fragment lacks final item")?;
            ensure!(
                item["id"].as_str() == Some(item_id),
                "stream text fragment item ID disagrees with final output"
            );
            let part = item["content"]
                .as_array()
                .and_then(|content| content.get(*content_index as usize))
                .context("stream text fragment lacks final content")?;
            let final_text = match source {
                TextDeltaSource::OutputText => {
                    ensure!(
                        part["type"] == "output_text",
                        "stream text fragment has incompatible final content"
                    );
                    part["text"].as_str()
                }
                TextDeltaSource::Refusal => {
                    ensure!(
                        part["type"] == "refusal",
                        "stream text fragment has incompatible final content"
                    );
                    part["refusal"].as_str()
                }
            }
            .context("stream text fragment has incompatible final content")?;
            ensure!(
                final_text == fragment,
                "stream text fragment disagrees with final output"
            );
        }
        for ((index, item_id, summary_index), fragment) in &self.summary_fragments {
            let item = output
                .get(*index as usize)
                .context("stream summary fragment lacks final item")?;
            ensure!(
                item["id"].as_str() == Some(item_id),
                "stream summary fragment item ID disagrees with final output"
            );
            ensure!(
                item["type"] == "reasoning",
                "stream summary fragment has incompatible final item"
            );
            let summary = item["summary"]
                .as_array()
                .and_then(|summary| summary.get(*summary_index as usize))
                .context("stream summary fragment lacks final summary")?;
            ensure!(
                summary["type"] == "summary_text",
                "stream summary fragment has incompatible final summary"
            );
            let final_text = summary["text"]
                .as_str()
                .or_else(|| summary["summary_text"].as_str())
                .context("stream summary fragment has incompatible final summary")?;
            ensure!(
                final_text == fragment,
                "stream summary fragment disagrees with final output"
            );
        }
        for ((index, item_id), fragment) in &self.tool_fragments {
            let item = output
                .get(*index as usize)
                .context("stream function fragment lacks final item")?;
            ensure!(
                item["id"].as_str() == Some(item_id),
                "stream function fragment item ID disagrees with final output"
            );
            ensure!(
                item["type"] == "function_call",
                "stream function fragment has incompatible final item"
            );
            let final_arguments = item["arguments"]
                .as_str()
                .context("stream function fragment lacks final arguments")?;
            ensure!(
                final_arguments == fragment,
                "stream function fragment disagrees with final output"
            );
            let _: Value = serde_json::from_str(final_arguments)
                .map_err(|_| diagnostics::stream_protocol())?;
        }
        for ((index, item_id), (arguments, name)) in &self.tool_done {
            let item = output
                .get(*index as usize)
                .context("completed function arguments lack final item")?;
            ensure!(
                item["id"].as_str() == Some(item_id),
                "completed function item ID disagrees with final output"
            );
            ensure!(
                item["type"] == "function_call",
                "completed function has incompatible final item"
            );
            ensure!(
                item["name"].as_str() == Some(name),
                "completed function name disagrees with final output"
            );
            let final_arguments = item["arguments"]
                .as_str()
                .context("completed function arguments lack final arguments")?;
            let expected: Value =
                serde_json::from_str(arguments).map_err(|_| diagnostics::stream_protocol())?;
            let actual: Value = serde_json::from_str(final_arguments)
                .map_err(|_| diagnostics::stream_protocol())?;
            ensure!(
                expected == actual,
                "completed function arguments disagree with final output"
            );
        }
        Ok(())
    }
}

fn append_fragment<K: Ord>(fragments: &mut BTreeMap<K, String>, key: K, value: &str) -> Result<()> {
    let entry = fragments.entry(key).or_default();
    let size = entry
        .len()
        .checked_add(value.len())
        .context("Responses stream fragment size overflow")?;
    ensure!(
        size <= MAX_BYTES,
        "Responses stream fragment exceeds retained output limit"
    );
    entry.push_str(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_utf8_crlf_multiline_data_and_completed_items() {
        let bytes = concat!(
            "\u{feff}:keepalive\r\n\r\n",
            "event: ignored-label\r\ndata: {\"type\":\"response.output_item.done\",\r\ndata: \"output_index\":0,\"item\":{\"id\":\"m1\",\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"échec\"}]}}\r\n\r\n",
            "data: {\"type\":\"future.metadata\",\"ignored\":true}\r\r",
            "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"m1\",\"output_index\":0,\"content_index\":0,\"delta\":\"échec\"}\n\n",
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
    fn reconciles_sparse_raw_indexes_against_full_terminal_output() {
        let output = json!([
            {"type":"reasoning","id":"reasoning-0","summary":[]},
            {"type":"message","id":"message-1","content":[{"type":"output_text","text":"visible"}]},
            {"type":"function_call","id":"call-2","call_id":"call-id","name":"file_read","arguments":"{\"path\":\"a.txt\"}"}
        ]);
        let events = [
            json!({"type":"response.output_text.delta","item_id":"message-1","output_index":1,"content_index":0,"delta":"vis"}),
            json!({"type":"response.output_text.delta","item_id":"message-1","output_index":1,"content_index":0,"delta":"ible"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"call-2","output_index":2,"delta":"{\"path\":\""}),
            json!({"type":"response.function_call_arguments.delta","item_id":"call-2","output_index":2,"delta":"a.txt\"}"}),
            json!({"type":"response.function_call_arguments.done","item_id":"call-2","output_index":2,"name":"file_read","arguments":"{ \"path\" : \"a.txt\" }"}),
            json!({"type":"response.completed","response":{"id":"r1","output":output}}),
        ];
        let bytes: String = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let result = Decoder::default().push(bytes.as_bytes()).unwrap().unwrap();
        assert_eq!(result["output"], output);
    }

    #[test]
    fn reconciles_visible_summary_and_refusal_and_rejects_conflicts() {
        let output = json!([
            {"type":"reasoning","id":"reasoning-0","summary":[{"type":"summary_text","text":"careful"}]},
            {"type":"message","id":"message-1","content":[{"type":"refusal","refusal":"cannot"}]}
        ]);
        let valid = [
            json!({"type":"response.reasoning_summary_text.delta","item_id":"reasoning-0","output_index":0,"summary_index":0,"delta":"careful"}),
            json!({"type":"response.refusal.delta","item_id":"message-1","output_index":1,"content_index":0,"delta":"cannot"}),
            json!({"type":"response.completed","response":{"id":"r1","output":output}}),
        ];
        let bytes: String = valid
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        assert!(Decoder::default().push(bytes.as_bytes()).unwrap().is_some());

        let conflict = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type":"response.output_text.delta","item_id":"message-1","output_index":1,"content_index":0,"delta":"wrong"}),
            json!({"type":"response.completed","response":{"id":"r1","output":output}}),
        );
        assert!(Decoder::default().push(conflict.as_bytes()).is_err());

        let hidden_refusal = json!({"type":"response.completed","response":{"id":"r1","output":[
            {"type":"message","id":"message-1","content":[{"type":"output_text","text":"safe","refusal":"hidden"}]}
        ]}});
        let conflict = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type":"response.refusal.delta","item_id":"message-1","output_index":0,"content_index":0,"delta":"hidden"}),
            hidden_refusal,
        );
        assert!(Decoder::default().push(conflict.as_bytes()).is_err());
    }

    #[test]
    fn rejects_duplicate_buffered_terminal_and_accepts_terminal_only_success() {
        let terminal = json!({"type":"response.completed","response":{"id":"r1","output":[{"type":"message","content":[{"type":"output_text","text":"terminal"}]}]}});
        let terminal_only = format!("data: {terminal}\n\n");
        assert!(
            Decoder::default()
                .push(terminal_only.as_bytes())
                .unwrap()
                .is_some()
        );
        let duplicate = format!("{terminal_only}data: {terminal}\n\n");
        assert!(Decoder::default().push(duplicate.as_bytes()).is_err());
    }

    #[test]
    fn requires_authoritative_output_and_unique_terminal_item_ids() {
        let no_output = json!({"type":"response.completed","response":{"id":"r1"}});
        assert!(
            Decoder::default()
                .push(format!("data: {no_output}\n\n").as_bytes())
                .is_err()
        );

        let duplicates = json!({"type":"response.completed","response":{"id":"r1","output":[
            {"type":"message","id":"same","content":[]},
            {"type":"reasoning","id":"same","summary":[]}
        ]}});
        assert!(
            Decoder::default()
                .push(format!("data: {duplicates}\n\n").as_bytes())
                .is_err()
        );
    }

    #[test]
    fn rejects_tool_fragment_for_non_function_terminal_item() {
        let terminal = json!({"type":"response.completed","response":{"id":"r1","output":[{
            "type":"message","id":"message-1","name":"file_read","arguments":"{}","content":[]
        }]}});
        let data = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type":"response.function_call_arguments.delta","item_id":"message-1","output_index":0,"delta":"{}"}),
            terminal,
        );
        assert!(Decoder::default().push(data.as_bytes()).is_err());
    }

    #[test]
    fn discarded_fragmented_traffic_does_not_spend_retained_response_budget() {
        let discarded = ": ignored framing\n".repeat(MAX_BYTES / 17 + 1);
        let item = json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","content":[{"text":"kept"}]}});
        let completed = json!({"type":"response.completed","response":{"id":"r1"}});
        let bytes = format!("{discarded}\ndata: {item}\n\ndata: {completed}\n\n");
        let mut decoder = Decoder::default();
        let mut result = None;
        for chunk in bytes.as_bytes().chunks(997) {
            if let Some(value) = decoder.push(chunk).unwrap() {
                assert!(result.replace(value).is_none());
            }
        }
        assert_eq!(result.unwrap()["output"][0]["content"][0]["text"], "kept");
    }

    #[test]
    fn accepts_large_single_line_completed_item_and_reconciles_final_envelope() {
        let item =
            json!({"type":"message","id":"item-1","content":[{"text":"x".repeat(128 * 1024)}]});
        let completed_item =
            json!({"type":"response.output_item.done","output_index":0,"item":item});
        let completed = json!({"type":"response.completed","response":{"id":"r1","output":[item]}});
        let bytes = format!("data: {completed_item}\n\ndata: {completed}\n\n");
        let result = Decoder::default().push(bytes.as_bytes()).unwrap().unwrap();
        assert_eq!(result["output"].as_array().unwrap().len(), 1);
        assert!(serde_json::to_vec(&result).unwrap().len() < MAX_BYTES);
    }

    #[test]
    fn rejects_oversized_retained_wire_and_event_budgets() {
        let large_item = json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","content":[{"text":"x".repeat(MAX_BYTES)}]}});
        let error = Decoder::default()
            .push(format!("data: {large_item}\n\n").as_bytes())
            .unwrap_err();
        assert!(error.to_string().contains("retained output"));

        let large_final = json!({"type":"response.completed","response":{"id":"r1","output":[{"type":"message","content":[{"text":"x".repeat(MAX_BYTES)}]}]}});
        let error = Decoder::default()
            .push(format!("data: {large_final}\n\n").as_bytes())
            .unwrap_err();
        assert!(error.to_string().contains("retained response"));

        let error = Decoder::default()
            .push(&vec![b'x'; MAX_SSE_WIRE_BYTES + 1])
            .unwrap_err();
        assert!(error.to_string().contains("wire size"));

        let mut large_event = b"data: ".to_vec();
        large_event.extend(vec![b'x'; MAX_SSE_EVENT_BYTES + 1]);
        large_event.extend(b"\n\n");
        let error = Decoder::default().push(&large_event).unwrap_err();
        assert!(error.to_string().contains("event line"));
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
                .push(&vec![b'x'; MAX_SSE_LINE_BYTES + 1])
                .unwrap_err()
                .to_string()
                .contains("event line")
        );

        let error = Decoder::default()
            .push(b"data: {\"parser-secret\":\n\n")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ChatGPT completion stream contained invalid protocol data"
        );
        assert!(!format!("{error:#}").contains("parser-secret"));
        assert!(!format!("{error:?}").contains("parser-secret"));
    }
}
