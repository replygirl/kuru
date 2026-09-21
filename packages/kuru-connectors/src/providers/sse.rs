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
    tool_done: BTreeMap<(u64, String), (String, Option<String>)>,
    // Function-call identity as the backend announces it, keyed by item ID.
    // The ChatGPT subscription route states a call's name and call ID once, on
    // `response.output_item.added`, and never repeats them on the argument
    // events that follow.
    tool_calls: BTreeMap<String, (Option<String>, Option<String>)>,
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
                trace_function_event(
                    "function_call_arguments.delta",
                    item_id,
                    event["call_id"].as_str(),
                    event["name"].as_str(),
                    arguments_fragment.len(),
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
                trace_function_event(
                    "function_call_arguments.done",
                    item_id,
                    event["call_id"].as_str(),
                    event["name"].as_str(),
                    arguments.len(),
                );
                // The name is not part of this event on every backend. It is
                // resolved from the call's own announced item at dispatch
                // time; a call that never announced one still fails the turn.
                let name = event["name"].as_str();
                ensure!(
                    arguments.len() <= MAX_BYTES && name.unwrap_or_default().len() <= MAX_BYTES,
                    "Responses completed function arguments exceed retained output limit"
                );
                let _: Value =
                    serde_json::from_str(arguments).map_err(|_| diagnostics::stream_protocol())?;
                ensure!(
                    self.tool_done
                        .insert(
                            (output_index, item_id.into()),
                            (arguments.into(), name.map(str::to_owned))
                        )
                        .is_none(),
                    "duplicate completed function arguments"
                );
            }
            // Announced but not yet complete. Its arguments are still empty,
            // but this is where the subscription route states the call's name
            // and call ID — the only place it ever does.
            "response.output_item.added" => {
                if let Some(item) = event.get("item").filter(|value| value.is_object())
                    && item["type"] == "function_call"
                {
                    trace_function_event(
                        "output_item.added",
                        item["id"].as_str().unwrap_or(""),
                        item["call_id"].as_str(),
                        item["name"].as_str(),
                        item["arguments"].as_str().map_or(0, str::len),
                    );
                    self.announce_function_call(item)?;
                }
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
                if item["type"] == "function_call" {
                    trace_function_event(
                        "output_item.done",
                        item["id"].as_str().unwrap_or(""),
                        item["call_id"].as_str(),
                        item["name"].as_str(),
                        item["arguments"].as_str().map_or(0, str::len),
                    );
                    self.announce_function_call(item)?;
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
                for (index, item) in &items {
                    trace_item("streamed", *index, item);
                }
                // An empty terminal array alongside items already delivered as
                // `response.output_item.done` carries no reconciliation
                // information: the ChatGPT subscription route closes with an
                // envelope that restates usage but not the output it already
                // sent. Treat that exactly as an absent array. An empty array
                // with nothing streamed still means an empty completion and
                // stays authoritative.
                let authoritative = match response.get("output") {
                    Some(output) => {
                        let output = output
                            .as_array()
                            .context("completed response has invalid output")?;
                        for (position, item) in output.iter().enumerate() {
                            trace_item("final", position, item);
                        }
                        if output.is_empty() && !items.is_empty() {
                            false
                        } else {
                            reconcile_items(output, &items)?;
                            true
                        }
                    }
                    None => false,
                };
                if !authoritative {
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
                self.settle_function_calls(&mut response)?;
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
    /// Retain the name and call ID a `function_call` item announces, keyed by
    /// its item ID. A second announcement of the same item may repeat those
    /// values but never change them.
    fn announce_function_call(&mut self, item: &Value) -> Result<()> {
        let Some(id) = item["id"].as_str().filter(|id| !id.is_empty()) else {
            return Ok(());
        };
        let name = item["name"].as_str().filter(|name| !name.is_empty());
        let call_id = item["call_id"]
            .as_str()
            .filter(|call_id| !call_id.is_empty());
        ensure!(
            name.unwrap_or_default().len() <= MAX_BYTES
                && call_id.unwrap_or_default().len() <= MAX_BYTES,
            "Responses function call identity exceeds retained output limit"
        );
        let announced = self.tool_calls.entry(id.to_owned()).or_insert((None, None));
        for (announced, observed) in [(&mut announced.0, name), (&mut announced.1, call_id)] {
            match (announced.as_deref(), observed) {
                (Some(known), Some(observed)) => ensure!(
                    known == observed,
                    "streamed function call identity disagrees with itself"
                ),
                (None, Some(observed)) => *announced = Some(observed.to_owned()),
                _ => {}
            }
        }
        Ok(())
    }

    /// Complete every function call in the authoritative output from what the
    /// backend announced for that exact item ID, then refuse to hand the
    /// runtime anything it could not honestly dispatch.
    ///
    /// The subscription route states a call's name and call ID once, on
    /// `response.output_item.added`, and its argument events carry arguments
    /// alone. Filling those fields back in from the same item's own
    /// announcement is reconciliation, not invention: a name that was never
    /// announced, a name the terminal output contradicts, or arguments that
    /// are absent or not complete JSON still fail the turn rather than
    /// reaching the tool host as an unnamed or truncated call.
    fn settle_function_calls(&self, response: &mut Value) -> Result<()> {
        let arguments_by_id: BTreeMap<&str, &str> = self
            .tool_done
            .iter()
            .map(|((_, item_id), (arguments, _))| (item_id.as_str(), arguments.as_str()))
            .collect();
        let output = response["output"]
            .as_array_mut()
            .context("completed response lacks output")?;
        for item in output {
            if item["type"] != "function_call" {
                continue;
            }
            let announced = item["id"]
                .as_str()
                .and_then(|id| self.tool_calls.get(id))
                .cloned()
                .unwrap_or((None, None));
            let streamed_arguments = item["id"].as_str().and_then(|id| arguments_by_id.get(id));
            for (field, announced) in [("name", announced.0), ("call_id", announced.1)] {
                let present = item[field].as_str().filter(|value| !value.is_empty());
                match (present, announced.as_deref()) {
                    (Some(present), Some(announced)) => ensure!(
                        present == announced,
                        "completed function call identity disagrees with streamed output"
                    ),
                    (None, Some(announced)) => item[field] = json!(announced),
                    (Some(_), None) => {}
                    (None, None) => bail!("completed function call lacks {field}"),
                }
            }
            if item["arguments"].as_str().is_none() {
                let arguments =
                    streamed_arguments.context("completed function call lacks arguments")?;
                item["arguments"] = json!(arguments);
            }
            let arguments = item["arguments"]
                .as_str()
                .context("completed function call lacks arguments")?;
            let _: Value =
                serde_json::from_str(arguments).map_err(|_| diagnostics::stream_protocol())?;
        }
        Ok(())
    }

    fn reconcile_fragments(&self, response: &Value) -> Result<()> {
        let output = response["output"]
            .as_array()
            .context("completed response lacks output")?;
        // Fragments are located by item ID, never by stream position: the
        // terminal output may reorder or drop items. Text the user was shown
        // must still be found and still agree; a reasoning summary or tool-call
        // scaffold the terminal output no longer carries has nothing to check.
        for ((_, item_id, content_index, source), fragment) in &self.text_fragments {
            let item =
                final_item(output, item_id).context("stream text fragment lacks final item")?;
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
        for ((_, item_id, summary_index), fragment) in &self.summary_fragments {
            let Some(item) = final_item(output, item_id) else {
                continue;
            };
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
        for ((_, item_id), fragment) in &self.tool_fragments {
            let Some(item) = final_item(output, item_id) else {
                continue;
            };
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
        for ((_, item_id), (arguments, name)) in &self.tool_done {
            let Some(item) = final_item(output, item_id) else {
                continue;
            };
            ensure!(
                item["type"] == "function_call",
                "completed function has incompatible final item"
            );
            // The name is checked only when the argument event carried one.
            // When it did not, `settle_function_calls` has already resolved it
            // from the call's own announced item and failed the turn if none
            // was ever announced.
            if let Some(name) = name {
                ensure!(
                    item["name"].as_str() == Some(name.as_str()),
                    "completed function name disagrees with final output"
                );
            }
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

/// Reconcile the items streamed as `response.output_item.done` against a
/// non-empty terminal `response.output`.
///
/// The terminal array is authoritative for ordering and presence: a reasoning
/// item or tool-call scaffold that the backend drops, replaces or reorders
/// there is a legitimate completion, not a client error. Text the user was
/// already shown is the exception — it must still be present, matched by item
/// ID, and still say the same thing, or the turn fails rather than quietly
/// contradicting the terminal.
fn reconcile_items(output: &[Value], items: &BTreeMap<usize, Value>) -> Result<()> {
    let mut final_ids = BTreeSet::new();
    for item in output {
        if let Some(id) = item["id"].as_str() {
            ensure!(final_ids.insert(id), "duplicate completed output ID");
        }
    }
    for item in items.values() {
        let streamed = visible_text(item);
        if streamed.is_empty() {
            continue;
        }
        let final_item = item["id"]
            .as_str()
            .and_then(|id| final_item(output, id))
            .context("completed response omits streamed output")?;
        ensure!(
            visible_text(final_item) == streamed,
            "completed response disagrees with streamed output"
        );
    }
    Ok(())
}

/// The terminal item carrying one streamed item ID, if the terminal output
/// kept it. Position in the stream never identifies it.
fn final_item<'a>(output: &'a [Value], item_id: &str) -> Option<&'a Value> {
    output
        .iter()
        .find(|item| item["id"].as_str() == Some(item_id))
}

/// The typed text parts a user would have been shown for one item, in order.
/// Untyped or non-text parts are not user-visible output and are not compared.
fn visible_text(item: &Value) -> Vec<(&str, &str)> {
    item["content"]
        .as_array()
        .map(|content| {
            content
                .iter()
                .filter_map(|part| match part["type"].as_str() {
                    Some("output_text") => part["text"].as_str().map(|text| ("output_text", text)),
                    Some("refusal") => part["refusal"].as_str().map(|text| ("refusal", text)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Record the *shape* of one reconciled output item in the bounded diagnostics
/// ring: identity, kind, position and text length only. Model text, reasoning
/// summaries, tool arguments and encrypted reasoning payloads are never
/// recorded, so this trace stays safe to read and attach to a bug report.
fn trace_item(stage: &'static str, index: usize, item: &Value) {
    tracing::debug!(
        target: "kuru.provider",
        operation = "responses-stream-reconcile",
        stage,
        item_index = index as u64,
        item_id = item["id"].as_str().unwrap_or(""),
        item_type = item["type"].as_str().unwrap_or(""),
        text_len = text_len(item),
        "reconciled output item"
    );
}

/// Record the *shape* of one function-call stream event in the bounded
/// diagnostics ring: the event kind, the item identity, whether that event
/// carried a call ID and a name, and how long its arguments were. The
/// arguments themselves, which can contain actor context, are never recorded.
fn trace_function_event(
    event_type: &'static str,
    item_id: &str,
    call_id: Option<&str>,
    name: Option<&str>,
    arguments_len: usize,
) {
    tracing::debug!(
        target: "kuru.provider",
        operation = "responses-stream-reconcile",
        stage = "function-call",
        event_type,
        item_id,
        has_call_id = call_id.is_some_and(|value| !value.is_empty()),
        has_name = name.is_some_and(|value| !value.is_empty()),
        arguments_len = arguments_len as u64,
        "observed function call event"
    );
}

/// Total length in bytes of the user-visible text an item carries. Never the
/// text itself.
fn text_len(item: &Value) -> u64 {
    item["content"]
        .as_array()
        .map(|content| {
            content
                .iter()
                .filter_map(|part| {
                    part["text"]
                        .as_str()
                        .or_else(|| part["refusal"].as_str())
                        .map(|text| text.len() as u64)
                })
                .sum()
        })
        .unwrap_or(0)
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

    /// The ids, types, indexes and event order below mirror one real turn on
    /// the ChatGPT subscription route, captured from the `--debug` diagnostics
    /// ring: a reasoning item at stream index 0, a message at index 1 whose
    /// visible text is five bytes, and a terminal envelope whose `output` is a
    /// present but EMPTY array. The pre-fix decoder rejected this with
    /// "completed response omits streamed output" and failed every turn.
    #[test]
    fn accepts_subscription_terminal_envelope_with_empty_output() {
        let reasoning = "rs_0068e14ccefb15ec016ab0bc43f01087d1ae431a51ff386b29";
        let message = "msg_0068e14ccefb15ec016ab0bc44515087d1aa3862980c0b266d";
        let events = [
            json!({"type":"response.output_item.done","output_index":0,"item":{
                "id":reasoning,"type":"reasoning","summary":[],"encrypted_content":"opaque"
            }}),
            json!({"type":"response.output_text.delta","item_id":message,"output_index":1,"content_index":0,"delta":"read"}),
            json!({"type":"response.output_text.delta","item_id":message,"output_index":1,"content_index":0,"delta":"y"}),
            json!({"type":"response.output_item.done","output_index":1,"item":{
                "id":message,"type":"message","role":"assistant",
                "content":[{"type":"output_text","text":"ready"}]
            }}),
            json!({"type":"response.completed","response":{
                "id":"resp_0068e14ccefb15ec016ab0bc43a2b487d1","status":"completed","output":[],
                "usage":{"input_tokens":1287,"output_tokens":9}
            }}),
        ];
        let bytes: String = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let result = Decoder::default().push(bytes.as_bytes()).unwrap().unwrap();
        let output = result["output"].as_array().unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["id"], json!(reasoning));
        assert_eq!(output[1]["id"], json!(message));
        assert_eq!(output[1]["content"][0]["text"], "ready");
        assert_eq!(result["usage"]["input_tokens"], 1287);
        assert_eq!(result["usage"]["output_tokens"], 9);
    }

    #[test]
    fn terminal_output_owns_order_and_presence_but_never_drops_visible_text() {
        let reasoning =
            json!({"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"});
        let message = json!({"type":"message","id":"msg_1","content":[{"type":"output_text","text":"ready"}]});
        let streamed = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            json!({"type":"response.output_item.done","output_index":1,"item":message}),
        );

        // Reordered, and the reasoning item replaced by the terminal output:
        // the terminal array decides, so this succeeds and is retained as sent.
        let reordered = json!([
            message,
            {"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"weighed it"}]}
        ]);
        let completed =
            json!({"type":"response.completed","response":{"id":"r1","output":reordered}});
        let result = Decoder::default()
            .push(format!("{streamed}data: {completed}\n\n").as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(result["output"], reordered);

        // A streamed reasoning item the terminal output drops is not an error.
        let without_reasoning =
            json!({"type":"response.completed","response":{"id":"r1","output":[message]}});
        assert_eq!(
            Decoder::default()
                .push(format!("{streamed}data: {without_reasoning}\n\n").as_bytes())
                .unwrap()
                .unwrap()["output"],
            json!([message])
        );

        // A streamed item that carried visible text and has no counterpart by
        // ID in the terminal output still fails the turn.
        let without_message =
            json!({"type":"response.completed","response":{"id":"r1","output":[reasoning]}});
        let error = Decoder::default()
            .push(format!("{streamed}data: {without_message}\n\n").as_bytes())
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("completed response omits streamed output"),
            "{error:#}"
        );

        // So does a counterpart whose visible text disagrees.
        let rewritten = json!({"type":"response.completed","response":{"id":"r1","output":[
            {"type":"message","id":"msg_1","content":[{"type":"output_text","text":"not ready"}]}
        ]}});
        let error = Decoder::default()
            .push(format!("{streamed}data: {rewritten}\n\n").as_bytes())
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("completed response disagrees with streamed output"),
            "{error:#}"
        );
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

    /// The event order, ids and field presence below mirror one real
    /// tool-calling turn on the ChatGPT subscription route, captured from the
    /// `--debug` diagnostics ring: `response.output_item.added` announces the
    /// call's name and call ID with empty arguments, every
    /// `function_call_arguments.delta` and the `function_call_arguments.done`
    /// that follows carry arguments alone (no name, no call ID), and the
    /// terminal envelope's `output` is empty. The pre-fix decoder rejected
    /// this with "completed function arguments lack name" and failed every
    /// tool-calling turn.
    #[test]
    fn accepts_subscription_function_call_announced_only_on_output_item_added() {
        let call = "fc_0239ab12e7bb1ad8016ab0d46525fc87d1977648d70a0ed18f";
        let arguments = "{\"path\":\"README.md\"}";
        let events = [
            json!({"type":"response.output_item.added","output_index":0,"item":{
                "id":call,"type":"function_call","name":"file_read",
                "call_id":"call_HcZ1sB6hCkG0","arguments":""
            }}),
            json!({"type":"response.function_call_arguments.delta","item_id":call,"output_index":0,"delta":"{\"path\":"}),
            json!({"type":"response.function_call_arguments.delta","item_id":call,"output_index":0,"delta":"\"README.md\"}"}),
            json!({"type":"response.function_call_arguments.done","item_id":call,"output_index":0,"arguments":arguments}),
            json!({"type":"response.output_item.done","output_index":0,"item":{
                "id":call,"type":"function_call","name":"file_read",
                "call_id":"call_HcZ1sB6hCkG0","arguments":arguments
            }}),
            json!({"type":"response.completed","response":{
                "id":"resp_0239ab12e7bb1ad8016ab0d4649c8887d1","status":"completed","output":[],
                "usage":{"input_tokens":2041,"output_tokens":24}
            }}),
        ];
        let bytes: String = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let result = Decoder::default().push(bytes.as_bytes()).unwrap().unwrap();
        let output = result["output"].as_array().unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["id"], json!(call));
        assert_eq!(output[0]["name"], "file_read");
        assert_eq!(output[0]["call_id"], "call_HcZ1sB6hCkG0");
        assert_eq!(output[0]["arguments"], json!(arguments));
    }

    /// `function_call_arguments.done` is arguments only; the call's identity
    /// is resolved by item ID from whichever announcement carried it, in
    /// either order, and a call whose name is never announced fails the turn
    /// instead of reaching the tool host unnamed.
    #[test]
    fn resolves_function_call_identity_by_item_id_or_fails_the_turn() {
        let arguments = "{\"path\":\"README.md\"}";
        let done = json!({"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":0,"arguments":arguments});
        let item = json!({
            "id":"fc_1","type":"function_call","name":"file_read",
            "call_id":"call_1","arguments":arguments
        });
        let completed = json!({"type":"response.completed","response":{"id":"r1","status":"completed","output":[]}});

        // Arguments settled before the item's own completed announcement.
        let early = format!(
            "data: {}\n\ndata: {done}\n\ndata: {}\n\ndata: {completed}\n\n",
            json!({"type":"response.output_item.added","output_index":0,"item":{
                "id":"fc_1","type":"function_call","arguments":""
            }}),
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
        );
        let result = Decoder::default().push(early.as_bytes()).unwrap().unwrap();
        assert_eq!(result["output"][0]["name"], "file_read");
        assert_eq!(result["output"][0]["call_id"], "call_1");

        // A name announced only on `added`, with a terminal item that omits
        // it, is still dispatchable.
        let late = format!(
            "data: {}\n\ndata: {done}\n\ndata: {}\n\ndata: {completed}\n\n",
            json!({"type":"response.output_item.added","output_index":0,"item":{
                "id":"fc_1","type":"function_call","name":"file_read","call_id":"call_1","arguments":""
            }}),
            json!({"type":"response.output_item.done","output_index":0,"item":{
                "id":"fc_1","type":"function_call","arguments":arguments
            }}),
        );
        let result = Decoder::default().push(late.as_bytes()).unwrap().unwrap();
        assert_eq!(result["output"][0]["name"], "file_read");
        assert_eq!(result["output"][0]["call_id"], "call_1");

        // A call whose name is never announced anywhere fails the turn.
        let unnamed = format!(
            "data: {done}\n\ndata: {}\n\ndata: {completed}\n\n",
            json!({"type":"response.output_item.done","output_index":0,"item":{
                "id":"fc_1","type":"function_call","call_id":"call_1","arguments":arguments
            }}),
        );
        let error = Decoder::default().push(unnamed.as_bytes()).unwrap_err();
        assert!(
            format!("{error:#}").contains("completed function call lacks name"),
            "{error:#}"
        );

        // So does one whose announced name the terminal output contradicts.
        let contradicted = format!(
            "data: {}\n\ndata: {done}\n\ndata: {}\n\ndata: {completed}\n\n",
            json!({"type":"response.output_item.added","output_index":0,"item":{
                "id":"fc_1","type":"function_call","name":"file_read","call_id":"call_1","arguments":""
            }}),
            json!({"type":"response.output_item.done","output_index":0,"item":{
                "id":"fc_1","type":"function_call","name":"shell","call_id":"call_1","arguments":arguments
            }}),
        );
        let error = Decoder::default()
            .push(contradicted.as_bytes())
            .unwrap_err();
        assert!(format!("{error:#}").contains("disagrees"), "{error:#}");

        // And so does one whose arguments never became complete JSON.
        let truncated = format!(
            "data: {}\n\ndata: {completed}\n\n",
            json!({"type":"response.output_item.done","output_index":0,"item":{
                "id":"fc_1","type":"function_call","name":"file_read","call_id":"call_1",
                "arguments":"{\"path\":"
            }}),
        );
        let error = Decoder::default().push(truncated.as_bytes()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "ChatGPT completion stream contained invalid protocol data"
        );
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

        // An empty terminal array with nothing streamed is a genuinely empty
        // completion, not a missing one.
        let empty_output = json!({"type":"response.completed","response":{"id":"r1","output":[]}});
        assert_eq!(
            Decoder::default()
                .push(format!("data: {empty_output}\n\n").as_bytes())
                .unwrap()
                .unwrap()["output"],
            json!([])
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
