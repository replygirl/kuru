use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use kuru_core::{Completion, CompletionRequest, Config, Message, ModelInfo, ToolCall};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{CodexProvider, http};

#[async_trait]
pub trait Provider: Send + Sync {
    async fn models(&self) -> Result<Vec<ModelInfo>>;
    async fn complete(&self, request: CompletionRequest) -> Result<Completion>;
}

pub fn provider(config: &Config, _cwd: &Path) -> Result<Arc<dyn Provider>> {
    match config.provider.as_str() {
        "demo" => Ok(Arc::new(DemoProvider)),
        "codex" => Ok(Arc::new(CodexProvider::new(&config.codex_command))),
        "responses" => Ok(Arc::new(ResponsesProvider::new(
            &config.api_base,
            &config.api_key_env,
        )?)),
        other => bail!("unknown provider: {other}"),
    }
}

/// Offline transport for installation checks; never masquerades as inference.
pub struct DemoProvider;

#[async_trait]
impl Provider for DemoProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "demo".into(),
            name: "Deterministic demo (offline)".into(),
            efforts: vec![],
            default_effort: None,
        }])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let latest = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .unwrap_or("Ready");
        let original = latest
            .strip_prefix("User request: ")
            .unwrap_or(latest)
            .split("\nExplicit contributions to this speaking identity:")
            .next()
            .unwrap_or(latest);
        let concise: String = original.chars().take(240).collect();
        let text = if request.instructions.contains("Phase: dream:") {
            "[demo] Offline demo memory consolidation.".into()
        } else if request.instructions.contains("Phase: speak and act:") {
            let mode = request
                .actor
                .split('/')
                .rev()
                .nth(2)
                .unwrap_or("configured mode");
            let count = request
                .instructions
                .lines()
                .find_map(|line| line.strip_prefix("Active peers: "))
                .and_then(|line| serde_json::from_str::<Vec<Value>>(line).ok())
                .map(|roster| roster.len())
                .unwrap_or(0);
            format!(
                "[demo] Kuru is running in {mode} with {count} active peers. No model request was sent. Your message: {concise}"
            )
        } else {
            format!("[demo] Offline contribution: {concise}")
        };
        Ok(Completion {
            text,
            calls: vec![],
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

#[derive(Clone)]
struct Pending {
    input: Vec<Value>,
    output: Vec<Value>,
    calls: Vec<String>,
}

/// Standard Responses API transport. Pending native output (including encrypted
/// reasoning) is transient and scoped to exactly one actor, solely to complete
/// function-call protocol handshakes. Kuru remains the durable memory owner.
pub struct ResponsesProvider {
    base: String,
    key_env: String,
    client: reqwest::Client,
    actors: Mutex<BTreeMap<String, Arc<Mutex<Option<Pending>>>>>,
}

impl ResponsesProvider {
    pub fn new(base: &str, key_env: &str) -> Result<Self> {
        http::endpoint(base)?;
        Ok(Self {
            base: base.trim_end_matches('/').into(),
            key_env: key_env.into(),
            client: http::client()?,
            actors: Mutex::new(BTreeMap::new()),
        })
    }

    fn authenticated(&self, builder: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        if self.key_env.is_empty() {
            return Ok(builder);
        }
        let key = std::env::var(&self.key_env)
            .with_context(|| format!("set {} for Responses API authentication", self.key_env))?;
        ensure!(!key.trim().is_empty(), "{} is empty", self.key_env);
        Ok(builder.bearer_auth(key))
    }

    async fn actor(&self, name: &str) -> Result<Arc<Mutex<Option<Pending>>>> {
        let mut actors = self.actors.lock().await;
        ensure!(
            actors.contains_key(name) || actors.len() < 1024,
            "provider actor transport limit reached; start a new session"
        );
        Ok(actors
            .entry(name.into())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone())
    }
}

fn text_message(message: &Message) -> Value {
    match message.role.as_str() {
        "assistant" | "system" | "developer" | "user" => {
            json!({"role":message.role,"content":message.content})
        }
        _ => {
            json!({"role":"user","content":format!("[{} record]\n{}", message.role, message.content)})
        }
    }
}

fn input_items(messages: &[Message], pending: Option<&Pending>) -> Result<Vec<Value>> {
    // Runtime tool continuations end with a tool receipt. A trailing user
    // message explicitly starts a new conversational/model phase.
    if messages
        .last()
        .is_some_and(|message| message.role == "user")
    {
        return Ok(messages.iter().map(text_message).collect());
    }
    let Some(pending) = pending else {
        return Ok(messages.iter().map(text_message).collect());
    };
    let mut found = BTreeMap::new();
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role != "tool" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&message.content)
            && let Some(id) = value["call_id"].as_str()
            && pending.calls.iter().any(|call| call == id)
        {
            found.entry(id.to_owned()).or_insert((index, value));
        }
    }
    // New non-tool user input starts a fresh protocol context. Historical tool
    // records after a restart are deliberately represented as ordinary text.
    if found.is_empty() {
        return Ok(messages.iter().map(text_message).collect());
    }
    if found.len() != pending.calls.len() {
        let latest_output = found.values().map(|(index, _)| *index).max().unwrap_or(0);
        if messages
            .iter()
            .skip(latest_output + 1)
            .any(|message| message.role == "user")
        {
            // A canceled operation can leave partial receipts. An explicit new
            // user turn starts fresh instead of trapping this actor forever.
            return Ok(messages.iter().map(text_message).collect());
        }
        bail!("missing function outputs for pending Responses calls");
    }
    let mut input = pending.input.clone();
    input.extend(pending.output.clone());
    let mut final_index = 0;
    for call in &pending.calls {
        let (index, record) = &found[call];
        final_index = final_index.max(*index);
        let output = record.get("output").context("tool result lacks output")?;
        input.push(json!({"type":"function_call_output","call_id":call,"output":output.as_str().map(str::to_owned).unwrap_or_else(|| output.to_string())}));
    }
    input.extend(messages.iter().skip(final_index + 1).map(text_message));
    Ok(input)
}

fn completion(value: &Value) -> Result<Completion> {
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        bail!(
            "Responses API error: {}",
            error["message"].as_str().unwrap_or("request failed")
        );
    }
    if let Some(status) = value["status"].as_str() {
        ensure!(
            status == "completed",
            "Responses API returned {status}; response was not completed"
        );
    }
    let output = value["output"]
        .as_array()
        .context("Responses response lacks output array")?;
    let mut text = Vec::new();
    let mut calls = Vec::new();
    let mut ids = BTreeSet::new();
    for item in output {
        match item["type"].as_str() {
            Some("message") => {
                if let Some(content) = item["content"].as_array() {
                    for part in content {
                        if let Some(value) =
                            part["text"].as_str().or_else(|| part["refusal"].as_str())
                        {
                            text.push(value.to_owned());
                        }
                    }
                }
            }
            Some("function_call") => {
                let id = item["call_id"]
                    .as_str()
                    .context("function call lacks call_id")?;
                ensure!(ids.insert(id), "duplicate function call ID");
                let name = item["name"].as_str().context("function call lacks name")?;
                let arguments = serde_json::from_str(
                    item["arguments"]
                        .as_str()
                        .context("function call lacks arguments")?,
                )
                .context("invalid function arguments JSON")?;
                calls.push(ToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments,
                });
            }
            _ => {}
        }
    }
    Ok(Completion {
        text: text.join("\n"),
        calls,
        input_tokens: value["usage"]["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: value["usage"]["output_tokens"].as_u64().unwrap_or(0),
    })
}

#[async_trait]
impl Provider for ResponsesProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        let value = http::json(
            self.authenticated(self.client.get(format!("{}/models", self.base)))?
                .send()
                .await?,
        )
        .await?;
        let data = value["data"]
            .as_array()
            .context("models response lacks data array")?;
        data.iter()
            .map(|model| {
                let id = model["id"].as_str().context("model lacks ID")?;
                // The public /models API does not advertise reasoning effort. Never
                // infer a static effort catalog or filter away future model names.
                Ok(ModelInfo {
                    id: id.into(),
                    name: id.into(),
                    efforts: vec![],
                    default_effort: None,
                })
            })
            .collect()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        ensure!(
            request.model != "auto",
            "select an explicit model for the Responses provider"
        );
        let actor = self.actor(&request.actor).await?;
        let mut pending = actor.lock().await;
        let input = input_items(&request.messages, pending.as_ref())?;
        let mut body = json!({"model":request.model,"instructions":request.instructions,"input":input,"store":false,"include":["reasoning.encrypted_content"],"tools":request.tools.iter().map(|tool| json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.parameters,"strict":false})).collect::<Vec<_>>()});
        if let Some(effort) = request.effort {
            body["reasoning"] = json!({"effort":effort});
        }
        ensure!(
            body.to_string().len() <= crate::MAX_BYTES,
            "Responses request exceeds 2 MiB transport limit; shorten actor context"
        );
        let value = http::json(
            self.authenticated(
                self.client
                    .post(format!("{}/responses", self.base))
                    .json(&body),
            )?
            .send()
            .await?,
        )
        .await?;
        let result = completion(&value)?;
        *pending = if result.calls.is_empty() {
            None
        } else {
            Some(Pending {
                input,
                output: value["output"]
                    .as_array()
                    .context("missing output")?
                    .clone(),
                calls: result.calls.iter().map(|call| call.id.clone()).collect(),
            })
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{HttpFixture, Reply, request};

    #[tokio::test]
    async fn native_function_round_trip_keeps_reasoning_and_actor_isolation() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"status":"completed","output":[{"type":"reasoning","encrypted_content":"opaque-private-reasoning","summary":[]},{"type":"function_call","call_id":"c1","name":"file_read","arguments":"{\"path\":\"a.txt\"}"},{"type":"function_call","call_id":"c2","name":"file_read","arguments":"{\"path\":\"b.txt\"}"}],"usage":{"input_tokens":8,"output_tokens":5}})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"other actor"}]}]})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"files compared"}]}]})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"refusal","refusal":"declined"}]}]})),
        ]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut first = request();
        let completion = provider.complete(first.clone()).await.unwrap();
        assert_eq!(completion.calls.len(), 2);
        assert_eq!((completion.input_tokens, completion.output_tokens), (8, 5));
        let mut other = first.clone();
        other.actor = "other-project/part".into();
        assert_eq!(provider.complete(other).await.unwrap().text, "other actor");
        first.messages.push(Message {
            role: "assistant".into(),
            content: "requesting file reads".into(),
        });
        for (call, text) in [("c2", "second file"), ("c1", "first file")] {
            first.messages.push(Message {
                role: "tool".into(),
                content: json!({"call_id":call,"output":text}).to_string(),
            });
        }
        assert_eq!(
            provider.complete(first.clone()).await.unwrap().text,
            "files compared"
        );
        first.messages.push(Message {
            role: "user".into(),
            content: "new turn".into(),
        });
        assert_eq!(provider.complete(first).await.unwrap().text, "declined");
        let sent = peer.requests.lock().await;
        assert_eq!(sent[0].body["reasoning"]["effort"], "future-effort");
        assert_eq!(sent[0].body["tools"][0]["name"], "file_read");
        assert_eq!(sent[0].body["store"], false);
        assert!(
            !sent[1]
                .body
                .to_string()
                .contains("opaque-private-reasoning")
        );
        let input = sent[2].body["input"].as_array().unwrap();
        assert!(
            input
                .iter()
                .any(|item| item["encrypted_content"] == "opaque-private-reasoning")
        );
        let outputs: Vec<_> = input
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .collect();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0]["call_id"], "c1");
        assert_eq!(outputs[0]["output"], "first file");
        assert!(
            !sent[3]
                .body
                .to_string()
                .contains("opaque-private-reasoning")
        );
    }

    #[tokio::test]
    async fn models_preserve_future_names_and_do_not_invent_efforts() {
        let peer = HttpFixture::new(vec![Reply::json(
            json!({"data":[{"id":"future-2099"},{"id":"special-model"}]}),
        )])
        .await;
        let models = ResponsesProvider::new(&peer.url, "")
            .unwrap()
            .models()
            .await
            .unwrap();
        assert_eq!(models[0].id, "future-2099");
        assert!(models[0].efforts.is_empty());
    }

    #[tokio::test]
    async fn provider_factory_and_demo_remain_offline() {
        let config = Config {
            provider: "demo".into(),
            ..Default::default()
        };
        let demo = provider(&config, Path::new(".")).unwrap();
        assert_eq!(demo.models().await.unwrap()[0].id, "demo");
        assert!(
            demo.complete(request())
                .await
                .unwrap()
                .text
                .contains("hello")
        );
        let mut empty = request();
        empty.messages.clear();
        assert!(demo.complete(empty).await.unwrap().text.contains("Ready"));
        for name in ["codex", "responses"] {
            assert!(
                provider(
                    &Config {
                        provider: name.into(),
                        ..Default::default()
                    },
                    Path::new(".")
                )
                .is_ok()
            );
        }
        assert!(
            provider(
                &Config {
                    provider: "unknown".into(),
                    ..Default::default()
                },
                Path::new(".")
            )
            .is_err()
        );
        let api =
            ResponsesProvider::new("http://localhost:1", "KURU_NONEXISTENT_AUTH_VARIABLE").unwrap();
        assert!(
            api.models()
                .await
                .unwrap_err()
                .to_string()
                .contains("set KURU_")
        );
        let mut auto = request();
        auto.model = "auto".into();
        assert!(
            api.complete(auto)
                .await
                .unwrap_err()
                .to_string()
                .contains("explicit model")
        );
        assert!(ResponsesProvider::new("file:///tmp/key", "").is_err());
    }

    #[test]
    fn validates_native_output_and_partial_tool_handshakes() {
        for value in [
            json!({"error":{"message":"unavailable"}}),
            json!({"status":"incomplete"}),
            json!({}),
            json!({"output":[{"type":"function_call","name":"x","arguments":"{}"}]}),
            json!({"output":[{"type":"function_call","call_id":"a","name":"x","arguments":"not-json"}]}),
            json!({"output":[{"type":"function_call","call_id":"a","name":"x","arguments":"{}"},{"type":"function_call","call_id":"a","name":"x","arguments":"{}"}]}),
        ] {
            assert!(completion(&value).is_err(), "{value}");
        }
        let pending = Pending {
            input: vec![],
            output: vec![],
            calls: vec!["a".into(), "b".into()],
        };
        let messages = vec![Message {
            role: "tool".into(),
            content: json!({"call_id":"a","output":"ok"}).to_string(),
        }];
        assert!(input_items(&messages, Some(&pending)).is_err());
        assert_eq!(input_items(&messages, None).unwrap()[0]["role"], "user");
        assert_eq!(
            input_items(&request().messages, Some(&pending))
                .unwrap()
                .len(),
            1
        );
        let malformed = vec![Message {
            role: "tool".into(),
            content: "not JSON".into(),
        }];
        assert_eq!(
            input_items(&malformed, Some(&pending)).unwrap()[0]["role"],
            "user"
        );
    }

    #[tokio::test]
    async fn failed_round_trip_can_be_retried_without_losing_pending_output() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"output":[{"type":"function_call","call_id":"a","name":"file_read","arguments":"{}"}]})),
            Reply::json(json!({"error":{"message":"temporary unavailable"}})),
            Reply::json(json!({"output":[]})),
        ]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut input = request();
        input.effort = None;
        provider.complete(input.clone()).await.unwrap();
        input.messages.push(Message {
            role: "tool".into(),
            content: json!({"call_id":"a","output":{"answer":4}}).to_string(),
        });
        assert!(provider.complete(input.clone()).await.is_err());
        provider.complete(input).await.unwrap();
        let sent = peer.requests.lock().await;
        assert_eq!(sent[1].body, sent[2].body);
        assert_eq!(
            sent[2].body["input"].as_array().unwrap().last().unwrap()["type"],
            "function_call_output"
        );
    }
}
