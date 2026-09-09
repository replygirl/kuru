use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A persisted conversation entry. Roles remain extensible for tool protocols.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub content: String,
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
    pub model: String,
    pub effort: Option<String>,
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}
