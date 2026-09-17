use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// One ordered piece of model-visible content. Unknown tags are refused rather
/// than silently omitted from provider requests or persistent history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        call_id: String,
        output: Value,
        is_error: bool,
    },
    ReasoningSummary {
        text: String,
    },
    Image {
        media_type: String,
        data_base64: String,
    },
    CacheBoundary {
        kind: String,
    },
}

/// A persisted conversation entry. Roles remain extensible for tool protocols.
/// New serialization has exactly one canonical ordered content representation.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub blocks: Vec<ContentBlock>,
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Typed {
            role: String,
            blocks: Vec<ContentBlock>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Legacy {
            role: String,
            content: String,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Typed(Typed),
            Legacy(Legacy),
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Typed(Typed { role, blocks }) => Self { role, blocks },
            Wire::Legacy(Legacy { role, content }) => Self::text(role, content),
        })
    }
}

impl Message {
    pub fn text(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            blocks: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn tool_result(call_id: impl Into<String>, output: Value, is_error: bool) -> Self {
        Self {
            role: "tool".into(),
            blocks: vec![ContentBlock::ToolResult {
                call_id: call_id.into(),
                output,
                is_error,
            }],
        }
    }

    pub fn plain_text(&self) -> Option<&str> {
        match self.blocks.as_slice() {
            [ContentBlock::Text { text }] => Some(text),
            _ => None,
        }
    }

    /// Keep the established prompt envelope for text while exposing structured
    /// records as blocks rather than flattening them into apparent prose.
    pub fn prompt_projection(&self) -> Value {
        if let Some(content) = self.plain_text() {
            serde_json::json!({"role": self.role, "content": content})
        } else {
            serde_json::json!({"role": self.role, "blocks": self.blocks})
        }
    }

    /// Deliberate display projection; never use this to encode provider input.
    pub fn text_projection(&self) -> String {
        self.blocks
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => text.clone(),
                ContentBlock::ToolUse {
                    id,
                    name,
                    arguments,
                } => format!("[tool use: {name} id={id}] {}", bounded_json(arguments)),
                ContentBlock::ToolResult {
                    call_id,
                    output,
                    is_error,
                } => format!(
                    "[tool result: id={call_id} error={is_error}] {}",
                    bounded_json(output)
                ),
                ContentBlock::ReasoningSummary { text } => format!("[reasoning summary: {text}]"),
                ContentBlock::Image { media_type, .. } => format!("[image: {media_type}]"),
                ContentBlock::CacheBoundary { kind } => format!("[cache boundary: {kind}]"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn bounded_json(value: &Value) -> String {
    const LIMIT: usize = 4096;
    let encoded = value.to_string();
    if encoded.len() <= LIMIT {
        return encoded;
    }
    let mut end = LIMIT;
    while !encoded.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{} [truncated {} bytes]",
        &encoded[..end],
        encoded.len() - end
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    /// Provider-supplied strings, rather than an enum, preserve future efforts.
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionRequest {
    pub actor: String,
    pub instructions: String,
    pub messages: Vec<Message>,
    /// Number of messages at the end of `messages` retained from this actor
    /// invocation's inputs. `None` is a legacy/direct caller: native provider
    /// continuation is disabled rather than inferred from durable history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_message_count: Option<usize>,
    pub model: String,
    pub effort: Option<String>,
    pub tools: Vec<ToolSpec>,
}

/// Provider observations, with absence distinct from an observed zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Completion {
    pub blocks: Vec<ContentBlock>,
    pub usage: Usage,
    pub stop_reason: Option<String>,
}

impl Completion {
    pub fn from_legacy(
        text: impl Into<String>,
        calls: Vec<ToolCall>,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Self {
        let text = text.into();
        let mut blocks = if text.is_empty() {
            Vec::new()
        } else {
            vec![ContentBlock::Text { text }]
        };
        blocks.extend(calls.into_iter().map(|call| ContentBlock::ToolUse {
            id: call.id,
            name: call.name,
            arguments: call.arguments,
        }));
        Self {
            blocks,
            usage: Usage {
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
                ..Usage::default()
            },
            stop_reason: None,
        }
    }

    pub fn text_projection(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn calls(&self) -> Vec<ToolCall> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse {
                    id,
                    name,
                    arguments,
                } => Some(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    pub fn with_calls(mut self, calls: Vec<ToolCall>) -> Self {
        for call in calls {
            self.push_call(call);
        }
        self
    }

    pub fn push_call(&mut self, call: ToolCall) {
        self.blocks.push(ContentBlock::ToolUse {
            id: call.id,
            name: call.name,
            arguments: call.arguments,
        });
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.blocks
            .retain(|block| !matches!(block, ContentBlock::Text { .. }));
        self.blocks
            .insert(0, ContentBlock::Text { text: text.into() });
    }

    pub fn input_tokens(&self) -> u64 {
        self.usage.input_tokens.unwrap_or(0)
    }

    pub fn output_tokens(&self) -> u64 {
        self.usage.output_tokens.unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typed_blocks_keep_order_and_legacy_json_looking_text_is_exact() {
        let message = Message {
            role: "future-role".into(),
            blocks: vec![
                ContentBlock::Text {
                    text: "before".into(),
                },
                ContentBlock::ToolUse {
                    id: "c1".into(),
                    name: "file_read".into(),
                    arguments: json!({"path":"東京.txt"}),
                },
                ContentBlock::ToolResult {
                    call_id: "c1".into(),
                    output: json!({"ok": true}),
                    is_error: false,
                },
                ContentBlock::ReasoningSummary {
                    text: "visible".into(),
                },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data_base64: "AAEC".into(),
                },
                ContentBlock::CacheBoundary {
                    kind: "ephemeral".into(),
                },
            ],
        };
        assert_eq!(
            serde_json::from_value::<Message>(serde_json::to_value(&message).unwrap()).unwrap(),
            message
        );
        let legacy = json!({"role":"tool","content":"{\"blocks\":[{\"type\":\"tool_result\"}]}"});
        let decoded: Message = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(decoded.plain_text(), legacy["content"].as_str());
        assert_eq!(decoded.role, "tool");
        assert!(
            serde_json::to_value(decoded)
                .unwrap()
                .get("content")
                .is_none()
        );
        assert!(
            serde_json::from_value::<Message>(json!({"role":"user","blocks":[{"type":"unknown"}]}))
                .is_err()
        );
    }

    #[test]
    fn completion_keeps_absent_usage_distinct_from_zero_and_preserves_call_order() {
        let completion = Completion {
            blocks: vec![
                ContentBlock::Text { text: "a".into() },
                ContentBlock::ToolUse {
                    id: "c1".into(),
                    name: "one".into(),
                    arguments: json!({}),
                },
                ContentBlock::Text { text: "b".into() },
                ContentBlock::ToolUse {
                    id: "c2".into(),
                    name: "two".into(),
                    arguments: json!({"x":1}),
                },
            ],
            usage: Usage {
                input_tokens: Some(0),
                ..Usage::default()
            },
            stop_reason: Some("future_stop".into()),
        };
        assert_eq!(completion.text_projection(), "a\nb");
        assert_eq!(
            completion
                .calls()
                .iter()
                .map(|call| call.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c1", "c2"]
        );
        assert_eq!(completion.usage.input_tokens, Some(0));
        assert_eq!(completion.usage.output_tokens, None);
        assert_eq!(
            serde_json::from_value::<Completion>(serde_json::to_value(&completion).unwrap())
                .unwrap(),
            completion
        );
    }
}
