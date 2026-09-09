use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

/// A2A 1.0 text messages use the same shape on local mailboxes and HTTP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerMessage {
    pub message_id: String,
    pub context_id: String,
    pub role: String,
    pub parts: Vec<TextPart>,
    pub metadata: Routing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPart {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routing {
    pub sender: String,
    pub recipient: String,
}

impl PeerMessage {
    pub fn new(sender: &str, recipient: &str, context: &str, text: &str) -> Result<Self> {
        ensure!(
            !sender.is_empty() && !recipient.is_empty(),
            "peer identities cannot be empty"
        );
        ensure!(
            !text.trim().is_empty() && text.len() <= 32_768,
            "peer message must contain 1–32768 bytes"
        );
        Ok(Self {
            message_id: Uuid::new_v4().to_string(),
            context_id: context.into(),
            role: "ROLE_AGENT".into(),
            parts: vec![TextPart { text: text.into() }],
            metadata: Routing {
                sender: sender.into(),
                recipient: recipient.into(),
            },
        })
    }

    pub fn rpc(&self) -> Value {
        json!({"jsonrpc":"2.0", "id":self.message_id, "method":"SendMessage", "params":{"message":self}})
    }

    pub fn text(&self) -> String {
        self.parts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
