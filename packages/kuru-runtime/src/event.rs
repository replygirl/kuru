use std::time::Duration;

use kuru_connectors::{project_json, project_text};
use kuru_core::Relationship;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use sha2::Digest;

const EVENT_WITHHELD: &str = "[event detail withheld]";
const RESPONSE_COMPLETED: &str = "[response completed]";
const MAX_TOOL_ARGUMENT_BYTES: usize = 8 * 1024;
const MAX_EVENT_PAYLOAD_BYTES: usize = 8 * 1024;
/// An 8 KiB UTF-8 receipt string may expand to six JSON bytes per control byte,
/// plus its enclosing quotes.
const MAX_TOOL_RESULT_SERIALIZED_BYTES: u64 = (MAX_TOOL_ARGUMENT_BYTES as u64 * 6) + 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateReport {
    pub activation: f64,
    pub note: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
/// A concrete resource limit reached while producing a turn.
pub enum TurnLimitReason {
    ToolCalls,
    PeerRounds,
    LegacyUnspecified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Ok,
    Error,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolObservation {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    pub outcome: ToolOutcome,
    pub argument_bytes: u64,
    pub result_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HookObservation {
    pub event: String,
    pub hook_index: u16,
    pub invocation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    pub outcome: String,
}

impl HookObservation {
    fn valid(&self) -> bool {
        matches!(
            self.event.as_str(),
            "pre_turn" | "post_turn" | "pre_tool" | "post_tool" | "speaker_selected"
        ) && self.hook_index > 0
            && !self.invocation_id.is_empty()
            && self.invocation_id.len() <= 256
            && self
                .turn_id
                .as_ref()
                .is_none_or(|id| !id.is_empty() && id.len() <= 256)
            && self
                .call_id
                .as_ref()
                .is_none_or(|call_id| !call_id.is_empty() && call_id.len() <= 256)
            && matches!(
                self.outcome.as_str(),
                "allowed"
                    | "rewritten"
                    | "denied"
                    | "observed"
                    | "stopped"
                    | "annotated"
                    | "failed"
                    | "suppressed"
            )
    }
}

impl ToolObservation {
    pub fn projected(
        call_id: &str,
        name: &str,
        arguments: Value,
        outcome: ToolOutcome,
        result: Option<Value>,
        elapsed: Duration,
    ) -> Self {
        let result =
            result.map(|result| project_value(result, MAX_TOOL_RESULT_SERIALIZED_BYTES as usize));
        Self::from_projected_receipt(call_id, name, arguments, outcome, result, elapsed)
    }

    /// Construct a receipt from the same projected value that is supplied to
    /// the actor. Its byte count and digest include JSON string escaping.
    pub fn from_projected_receipt(
        call_id: &str,
        name: &str,
        arguments: Value,
        outcome: ToolOutcome,
        projected_result: Option<Value>,
        elapsed: Duration,
    ) -> Self {
        let call_id = project_text(call_id).unwrap_or_else(|_| EVENT_WITHHELD.into());
        let name = project_text(name).unwrap_or_else(|_| EVENT_WITHHELD.into());
        let arguments = project_value(arguments, MAX_TOOL_ARGUMENT_BYTES);
        let argument_bytes = serialized_len(&arguments);
        let (result_bytes, result_sha256) = projected_result.map_or((0, None), |result| {
            let bytes = serde_json::to_vec(&result).unwrap_or_else(|_| b"null".to_vec());
            let digest = sha2::Sha256::digest(&bytes);
            (
                bytes.len().min(u64::MAX as usize) as u64,
                Some(digest.iter().map(|byte| format!("{byte:02x}")).collect()),
            )
        });
        Self {
            call_id,
            name,
            arguments,
            outcome,
            argument_bytes,
            result_bytes,
            result_sha256,
            elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
        }
    }

    fn valid(&self) -> bool {
        self.argument_bytes == serialized_len(&self.arguments)
            && self.argument_bytes <= MAX_TOOL_ARGUMENT_BYTES as u64
            && self.result_bytes <= MAX_TOOL_RESULT_SERIALIZED_BYTES
            && self.result_sha256.as_ref().is_none_or(|digest| {
                digest.len() == 64
                    && digest.bytes().all(|byte| {
                        byte.is_ascii_digit()
                            || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
                    })
            })
            && ((self.result_bytes == 0) == self.result_sha256.is_none())
            && match self.outcome {
                ToolOutcome::Ok => self.result_bytes > 0 && self.result_sha256.is_some(),
                ToolOutcome::Denied | ToolOutcome::Cancelled => {
                    self.result_bytes == 0 && self.result_sha256.is_none()
                }
                ToolOutcome::Error => true,
            }
    }
}

/// A projected semantic runtime observation. Serialize it only through the
/// compatible `{kind, actor, detail}` wire adapter.
#[derive(Debug, Clone)]
pub enum Event {
    Active {
        actor: String,
        detail: String,
    },
    Idle {
        actor: String,
        detail: String,
    },
    SpeakerSelection {
        actor: String,
        reason: String,
    },
    Speaker {
        actor: String,
        identity_kind: String,
    },
    ToolStarted {
        actor: String,
        call_id: String,
        name: String,
    },
    ToolSettled {
        actor: String,
        observation: ToolObservation,
    },
    Hook {
        actor: String,
        observation: HookObservation,
    },
    Mcp {
        actor: String,
        detail: String,
    },
    Peer {
        actor: String,
        envelope: Value,
    },
    Relationship {
        actor: String,
        relationship: Relationship,
    },
    State {
        actor: String,
        report: StateReport,
    },
    Budget {
        actor: String,
        reason: TurnLimitReason,
        detail: Option<String>,
    },
    Dream {
        actor: String,
        detail: String,
    },
    Error {
        actor: String,
        detail: String,
    },
    Response {
        actor: String,
    },
    Legacy {
        kind: String,
        actor: String,
        detail: String,
    },
    Withheld {
        kind: String,
        actor: String,
    },
}

impl Event {
    pub fn kind(&self) -> &str {
        match self {
            Self::Active { .. } => "active",
            Self::Idle { .. } => "idle",
            Self::SpeakerSelection { .. } => "speaker-selection",
            Self::Speaker { .. } => "speaker",
            Self::ToolStarted { .. } => "tool",
            Self::ToolSettled { .. } => "tool-observation",
            Self::Hook { .. } => "hook",
            Self::Mcp { .. } => "mcp",
            Self::Peer { .. } => "peer",
            Self::Relationship { .. } => "relationship",
            Self::State { .. } => "state",
            Self::Budget { .. } => "budget",
            Self::Dream { .. } => "dream",
            Self::Error { .. } => "error",
            Self::Response { .. } => "response",
            Self::Legacy { kind, .. } | Self::Withheld { kind, .. } => kind,
        }
    }

    pub fn actor(&self) -> &str {
        match self {
            Self::Active { actor, .. }
            | Self::Idle { actor, .. }
            | Self::SpeakerSelection { actor, .. }
            | Self::Speaker { actor, .. }
            | Self::ToolStarted { actor, .. }
            | Self::ToolSettled { actor, .. }
            | Self::Hook { actor, .. }
            | Self::Mcp { actor, .. }
            | Self::Peer { actor, .. }
            | Self::Relationship { actor, .. }
            | Self::State { actor, .. }
            | Self::Budget { actor, .. }
            | Self::Dream { actor, .. }
            | Self::Error { actor, .. }
            | Self::Response { actor }
            | Self::Legacy { actor, .. }
            | Self::Withheld { actor, .. } => actor,
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Active { detail, .. }
            | Self::Idle { detail, .. }
            | Self::Mcp { detail, .. }
            | Self::Dream { detail, .. }
            | Self::Error { detail, .. }
            | Self::Legacy { detail, .. } => detail.clone(),
            Self::SpeakerSelection { reason, .. } => reason.clone(),
            Self::Speaker { identity_kind, .. } => identity_kind.clone(),
            Self::ToolStarted { name, .. } => name.clone(),
            Self::ToolSettled { observation, .. } => wire_json(observation),
            Self::Hook { observation, .. } => wire_json(observation),
            Self::Peer { envelope, .. } => wire_json(envelope),
            Self::Relationship { relationship, .. } => wire_json(relationship),
            Self::State { report, .. } => wire_json(report),
            Self::Budget { reason, detail, .. } => {
                let mut value = json!({"reason": reason});
                if let Some(detail) = detail {
                    value["detail"] = Value::String(detail.clone());
                }
                wire_json(&value)
            }
            Self::Response { .. } => RESPONSE_COMPLETED.into(),
            Self::Withheld { .. } => EVENT_WITHHELD.into(),
        }
    }

    pub fn projected(self) -> Self {
        let actor = project_detail(self.actor().into());
        match self {
            Self::Active { detail, .. } => Self::Active {
                actor,
                detail: project_detail(detail),
            },
            Self::Idle { detail, .. } => Self::Idle {
                actor,
                detail: project_detail(detail),
            },
            Self::SpeakerSelection { reason, .. } => Self::SpeakerSelection {
                actor,
                reason: project_detail(reason),
            },
            Self::Speaker { identity_kind, .. } => Self::Speaker {
                actor,
                identity_kind: project_detail(identity_kind),
            },
            Self::ToolStarted { call_id, name, .. } => Self::ToolStarted {
                actor,
                call_id: project_detail(call_id),
                name: project_detail(name),
            },
            Self::ToolSettled { observation, .. } => Self::ToolSettled {
                actor,
                observation: project_observation(observation),
            },
            Self::Hook {
                mut observation, ..
            } => {
                observation.event = project_detail(observation.event);
                observation.invocation_id = project_detail(observation.invocation_id);
                observation.turn_id = observation.turn_id.map(project_detail);
                observation.call_id = observation.call_id.map(project_detail);
                observation.outcome = project_detail(observation.outcome);
                Self::Hook { actor, observation }
            }
            Self::Mcp { detail, .. } => Self::Mcp {
                actor,
                detail: project_detail(detail),
            },
            Self::Peer { envelope, .. } => {
                let envelope = project_value(envelope, MAX_EVENT_PAYLOAD_BYTES);
                if envelope == Value::String(EVENT_WITHHELD.into()) {
                    Self::Withheld {
                        kind: "peer".into(),
                        actor,
                    }
                } else {
                    Self::Peer { actor, envelope }
                }
            }
            Self::Relationship { relationship, .. } => {
                let relationship = serde_json::to_value(relationship)
                    .ok()
                    .map(|value| project_value(value, MAX_EVENT_PAYLOAD_BYTES))
                    .and_then(|value| serde_json::from_value(value).ok());
                match relationship {
                    Some(relationship) => Self::Relationship {
                        actor,
                        relationship,
                    },
                    None => Self::Withheld {
                        kind: "relationship".into(),
                        actor,
                    },
                }
            }
            Self::State { report, .. } => {
                let report = serde_json::to_value(report)
                    .ok()
                    .map(|value| project_value(value, MAX_EVENT_PAYLOAD_BYTES))
                    .and_then(|value| serde_json::from_value(value).ok());
                match report {
                    Some(report) => Self::State { actor, report },
                    None => Self::Withheld {
                        kind: "state".into(),
                        actor,
                    },
                }
            }
            Self::Budget { reason, detail, .. } => Self::Budget {
                actor,
                reason,
                detail: detail.map(project_detail),
            },
            Self::Dream { detail, .. } => Self::Dream {
                actor,
                detail: project_detail(detail),
            },
            Self::Error { detail, .. } => Self::Error {
                actor,
                detail: project_detail(detail),
            },
            Self::Response { .. } => Self::Response { actor },
            Self::Legacy { kind, detail, .. } => Self::Legacy {
                kind: project_detail(kind),
                actor,
                detail: project_detail(detail),
            },
            Self::Withheld { kind, .. } => Self::Withheld {
                kind: project_detail(kind),
                actor,
            },
        }
    }

    pub fn from_wire_v1(kind: String, actor: String, detail: String) -> Self {
        let event = match kind.as_str() {
            "active" => Self::Active { actor, detail },
            "idle" => Self::Idle { actor, detail },
            "speaker-selection" => Self::SpeakerSelection {
                actor,
                reason: detail,
            },
            "speaker" => Self::Speaker {
                actor,
                identity_kind: detail,
            },
            "tool" => Self::ToolStarted {
                actor,
                call_id: String::new(),
                name: detail,
            },
            "mcp" => Self::Mcp { actor, detail },
            "dream" => Self::Dream { actor, detail },
            "error" => Self::Error { actor, detail },
            "response" => Self::Response { actor },
            "peer" => serde_json::from_str(&detail)
                .map(|envelope| Self::Peer {
                    actor: actor.clone(),
                    envelope,
                })
                .unwrap_or_else(|_| Self::Withheld { kind, actor }),
            "relationship" => serde_json::from_str(&detail)
                .map(|relationship| Self::Relationship {
                    actor: actor.clone(),
                    relationship,
                })
                .unwrap_or_else(|_| Self::Withheld { kind, actor }),
            "state" => serde_json::from_str(&detail)
                .map(|report| Self::State {
                    actor: actor.clone(),
                    report,
                })
                .unwrap_or_else(|_| Self::Withheld { kind, actor }),
            _ => Self::Legacy {
                kind,
                actor,
                detail,
            },
        };
        event.projected()
    }

    pub fn from_wire_v2(kind: String, actor: String, detail: String) -> Self {
        let event = match kind.as_str() {
            "tool-observation" => serde_json::from_str::<ToolObservation>(&detail)
                .ok()
                .filter(ToolObservation::valid)
                .map(|observation| Self::ToolSettled {
                    actor: actor.clone(),
                    observation,
                })
                .unwrap_or_else(|| Self::Withheld { kind, actor }),
            "hook" => serde_json::from_str::<HookObservation>(&detail)
                .ok()
                .filter(HookObservation::valid)
                .map(|observation| Self::Hook {
                    actor: actor.clone(),
                    observation,
                })
                .unwrap_or_else(|| Self::Withheld { kind, actor }),
            "peer" => strict_peer(&actor, &detail).unwrap_or(Self::Withheld { kind, actor }),
            "relationship" => {
                strict_relationship(&actor, &detail).unwrap_or(Self::Withheld { kind, actor })
            }
            "state" => strict_state(&actor, &detail).unwrap_or(Self::Withheld { kind, actor }),
            "budget" => strict_budget(&actor, &detail).unwrap_or(Self::Withheld { kind, actor }),
            "active" | "idle" | "speaker-selection" | "speaker" | "tool" | "mcp" | "dream"
            | "error" | "response" => Self::from_wire_v1(kind, actor, detail),
            _ => Self::Legacy {
                kind,
                actor,
                detail,
            }
            .projected(),
        };
        event.projected()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvent {
    kind: String,
    actor: String,
    detail: String,
}

impl Serialize for Event {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        WireEvent {
            kind: self.kind().into(),
            actor: self.actor().into(),
            detail: self.detail(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireEvent::deserialize(deserializer)?;
        Ok(Self::from_wire_v2(wire.kind, wire.actor, wire.detail))
    }
}

fn project_detail(value: String) -> String {
    project_text(&value)
        .and_then(|value| {
            (value.len() <= MAX_EVENT_PAYLOAD_BYTES)
                .then_some(value)
                .ok_or(kuru_connectors::ProjectionError::SizeBound)
        })
        .unwrap_or_else(|_| EVENT_WITHHELD.into())
}

fn project_value(value: Value, bound: usize) -> Value {
    let value = project_json(value)
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_else(|| Value::String(EVENT_WITHHELD.into()));
    if serde_json::to_vec(&value).map_or(true, |bytes| bytes.len() > bound) {
        Value::String(EVENT_WITHHELD.into())
    } else {
        value
    }
}

fn project_observation(mut observation: ToolObservation) -> ToolObservation {
    observation.call_id = project_detail(observation.call_id);
    observation.name = project_detail(observation.name);
    observation.arguments = project_value(observation.arguments, MAX_TOOL_ARGUMENT_BYTES);
    observation.argument_bytes = serialized_len(&observation.arguments);
    observation
}

fn serialized_len(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len().min(u64::MAX as usize) as u64)
        .unwrap_or(0)
}

fn wire_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| EVENT_WITHHELD.into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictBudget {
    reason: TurnLimitReason,
    #[serde(default)]
    detail: Option<String>,
}

fn strict_budget(actor: &str, detail: &str) -> Option<Event> {
    let budget: StrictBudget = serde_json::from_str(detail).ok()?;
    Some(Event::Budget {
        actor: actor.into(),
        reason: budget.reason,
        detail: budget.detail,
    })
}

fn strict_state(actor: &str, detail: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(detail).ok()?;
    let object = value.as_object()?;
    (object.len() == 2 && object.contains_key("activation") && object.contains_key("note"))
        .then_some(())?;
    let report: StateReport = serde_json::from_value(value).ok()?;
    (report.activation.is_finite()
        && (0.0..=1.0).contains(&report.activation)
        && report.note.len() <= MAX_EVENT_PAYLOAD_BYTES)
        .then_some(Event::State {
            actor: actor.into(),
            report,
        })
}

fn strict_relationship(actor: &str, detail: &str) -> Option<Event> {
    let relationship: Relationship = serde_json::from_str(detail).ok()?;
    let canonical = Relationship::new(relationship.kind, relationship.members.clone()).ok()?;
    (canonical.id == relationship.id).then_some(Event::Relationship {
        actor: actor.into(),
        relationship,
    })
}

fn strict_peer(actor: &str, detail: &str) -> Option<Event> {
    let envelope: Value = serde_json::from_str(detail).ok()?;
    let object = envelope.as_object()?;
    (object.len() == 4
        && object.get("jsonrpc")?.as_str() == Some("2.0")
        && object.get("method")?.as_str() == Some("SendMessage"))
    .then_some(())?;
    let id = object.get("id")?.as_str()?;
    let message: crate::bus::PeerMessage =
        serde_json::from_value(object.get("params")?.get("message")?.clone()).ok()?;
    (id == message.message_id
        && message.metadata.sender == actor
        && !message.metadata.sender.is_empty()
        && !message.metadata.recipient.is_empty()
        && !message.parts.is_empty()
        && message
            .parts
            .iter()
            .all(|part| !part.text.trim().is_empty() && part.text.len() <= 32_768))
    .then_some(Event::Peer {
        actor: actor.into(),
        envelope,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sha2::{Digest, Sha256};

    use super::{Event, ToolObservation, ToolOutcome};

    #[test]
    fn wire_adapter_stays_a_three_key_object() {
        let event = Event::SpeakerSelection {
            actor: "part".into(),
            reason: "caller-target".into(),
        }
        .projected();
        let wire = serde_json::to_value(event).unwrap();
        assert_eq!(wire.as_object().unwrap().len(), 3);
        assert_eq!(wire["kind"], "speaker-selection");
        assert_eq!(wire["actor"], "part");
        assert_eq!(wire["detail"], "caller-target");

        let started = Event::ToolStarted {
            actor: "part".into(),
            call_id: "provider-call-7".into(),
            name: "file_read".into(),
        }
        .projected();
        assert!(matches!(
            &started,
            Event::ToolStarted { call_id, .. } if call_id == "provider-call-7"
        ));
        let wire = serde_json::to_value(started).unwrap();
        assert_eq!(wire.as_object().unwrap().len(), 3);
        assert_eq!(wire["kind"], "tool");
        assert_eq!(wire["actor"], "part");
        assert_eq!(wire["detail"], "file_read");
        assert!(!wire.to_string().contains("provider-call-7"));
    }

    #[test]
    fn receipt_counts_serialized_projected_bytes_including_escaping() {
        let result = json!("control:\u{0000}\n雪");
        let bytes = serde_json::to_vec(&result).unwrap();
        let observation = ToolObservation::from_projected_receipt(
            "call-1",
            "file_read",
            json!({"path":"notes.txt"}),
            ToolOutcome::Ok,
            Some(result),
            std::time::Duration::from_millis(12),
        );
        assert_eq!(observation.result_bytes, bytes.len() as u64);
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(observation.result_sha256.as_deref(), Some(digest.as_str()));
    }

    #[test]
    fn versioned_wire_normalizers_keep_response_and_unknown_history_safe() {
        let response = Event::from_wire_v1("response".into(), "actor".into(), "secret".into());
        assert_eq!(response.detail(), "[response completed]");
        let malformed =
            Event::from_wire_v2("tool-observation".into(), "actor".into(), "not-json".into());
        assert_eq!(malformed.detail(), "[event detail withheld]");
        let unknown = Event::from_wire_v1("historical-kind".into(), "actor".into(), "value".into());
        assert_eq!(unknown.kind(), "historical-kind");
        assert_eq!(unknown.detail(), "value");
    }

    #[test]
    fn malformed_v2_structured_payloads_and_oversized_peer_are_withheld() {
        let invalid_state = Event::from_wire_v2(
            "state".into(),
            "actor".into(),
            r#"{"activation":4.0,"note":"x"}"#.into(),
        );
        assert_eq!(invalid_state.detail(), "[event detail withheld]");
        let invalid_budget = Event::from_wire_v2(
            "budget".into(),
            "actor".into(),
            r#"{"reason":"tool-calls","extra":true}"#.into(),
        );
        assert_eq!(invalid_budget.detail(), "[event detail withheld]");
        let denied_with_receipt = Event::from_wire_v2(
            "tool-observation".into(),
            "actor".into(),
            r#"{"call_id":"c","name":"n","arguments":{},"outcome":"denied","argument_bytes":2,"result_bytes":2,"result_sha256":"0000000000000000000000000000000000000000000000000000000000000000","elapsed_ms":0}"#.into(),
        );
        assert_eq!(denied_with_receipt.detail(), "[event detail withheld]");
        let oversized = Event::Peer {
            actor: "actor".into(),
            envelope: json!({"body":"x".repeat(9000)}),
        }
        .projected();
        let wire = serde_json::to_value(oversized).unwrap();
        assert_eq!(wire["detail"], "[event detail withheld]");
    }
}
