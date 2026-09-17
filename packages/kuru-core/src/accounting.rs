//! Bounded facts shared by the runtime and the memory-owned usage ledger.
//!
//! These values describe one provider invocation. They contain no prompt,
//! request body, native continuation, tool arguments or credentials.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{PriceSchedule, Usage};

const MAX_ID_BYTES: usize = 256;
const MAX_ROUTE_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsagePhase {
    Deliberate,
    Speak,
    Consult,
    Dream,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// Immutable identity and historical price terms, committed before inference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvocationStart {
    pub session_id: String,
    pub invocation_id: String,
    pub operation_id: String,
    pub phase: UsagePhase,
    pub actor_id: String,
    pub route: String,
    pub model: String,
    pub price_at_invocation: Option<PriceSchedule>,
}

impl InvocationStart {
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("session_id", &self.session_id),
            ("invocation_id", &self.invocation_id),
            ("operation_id", &self.operation_id),
            ("actor_id", &self.actor_id),
            ("model", &self.model),
        ] {
            ensure!(
                !value.is_empty()
                    && value.len() <= MAX_ID_BYTES
                    && !value.chars().any(char::is_control),
                "usage {name} must be nonempty, bounded and contain no controls"
            );
        }
        ensure!(
            !self.route.is_empty()
                && self.route.len() <= MAX_ROUTE_BYTES
                && !self.route.chars().any(char::is_control),
            "usage route must be nonempty, bounded and contain no controls"
        );
        Ok(())
    }
}

/// Local sequence assigned by the actor's fallible P5 observer, not a remote
/// provider event ID. A terminal report may have all components absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UsageObservation {
    pub sequence: u64,
    pub terminal: bool,
    pub usage: Usage,
}

impl UsageObservation {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.sequence > 0, "usage sequence must start at one");
        Ok(())
    }
}

/// A durable per-invocation record. `terminal_usage = None` differs from a
/// terminal report whose four components are all absent. `usage` retains
/// earlier evidence for components the terminal did not report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvocationUsage {
    pub start: InvocationStart,
    pub usage: Usage,
    pub last_usage_sequence: Option<u64>,
    pub terminal_usage: Option<Usage>,
    pub outcome: Option<InvocationOutcome>,
    pub incomplete: bool,
}

/// Whether every provider invocation in this session reported a component.
/// A pre-ledger session is incomplete for all four components regardless of
/// how many current records happen to have reports.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UsageCompleteness {
    pub input_tokens: bool,
    pub output_tokens: bool,
    pub cached_input_tokens: bool,
    pub reasoning_output_tokens: bool,
}

/// A priced term the fold did not apply to a subtotal. The ledger keeps the raw
/// per-kind token components and the frozen price basis, so naming the term is
/// enough for a later reprice; a partly modelled term is never half-applied.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum UnappliedPriceTerm {
    /// A schedule's long-context tier could not be decided or parsed, so no
    /// tier multiplier was applied to that invocation.
    LongContextTier,
    /// The schedule prices cache writes, which this fold does not model.
    CacheWriteRate,
    /// Cached input tokens were reported without a usable cached-input rate.
    CachedInputRate,
    /// An invocation carried no frozen price, or its rates were unusable.
    InvocationPrice,
    /// A token component the subtotal needs was never reported.
    TokenComponents,
}

impl UnappliedPriceTerm {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LongContextTier => "long-context tier",
            Self::CacheWriteRate => "cache-write rate",
            Self::CachedInputRate => "cached-input rate",
            Self::InvocationPrice => "invocation price",
            Self::TokenComponents => "token components",
        }
    }
}

/// A known subtotal and its uncertainty. The decimal string is computed from
/// frozen invocation prices; `None` never means a known zero charge.
/// `unapplied` names what an incomplete subtotal leaves out.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MoneyEstimate {
    pub known_usd: Option<String>,
    pub incomplete: bool,
    /// Sorted and deduplicated; empty whenever nothing priced was skipped.
    #[serde(default)]
    pub unapplied: Vec<UnappliedPriceTerm>,
}

/// Bounded result of folding one session's operational records on read. It
/// does not expose a mutable aggregate or an unbounded record list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionUsage {
    pub session_id: String,
    pub historical_complete: bool,
    pub invocation_count: u64,
    pub incomplete_invocations: u64,
    /// Each component is summed independently; cached/reasoning are subsets.
    pub known_usage: Usage,
    pub component_complete: UsageCompleteness,
    pub api_standard: MoneyEstimate,
    pub api_equivalent: MoneyEstimate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_identity_is_bounded_without_conflating_zero_and_absent_usage() {
        let start = InvocationStart {
            session_id: "s".into(),
            invocation_id: "i".into(),
            operation_id: "turn".into(),
            phase: UsagePhase::Speak,
            actor_id: "peer".into(),
            route: "responses".into(),
            model: "future-model".into(),
            price_at_invocation: None,
        };
        start.validate().unwrap();
        let observed = UsageObservation {
            sequence: 1,
            terminal: false,
            usage: Usage {
                input_tokens: Some(0),
                ..Usage::default()
            },
        };
        observed.validate().unwrap();
        assert_eq!(
            serde_json::from_str::<UsageObservation>(&serde_json::to_string(&observed).unwrap())
                .unwrap(),
            observed
        );
        assert!(
            UsageObservation {
                sequence: 0,
                ..observed.clone()
            }
            .validate()
            .is_err()
        );
        assert_ne!(
            observed.usage.input_tokens,
            observed.usage.cached_input_tokens
        );
        assert!(
            InvocationStart {
                actor_id: "x".repeat(MAX_ID_BYTES + 1),
                ..start
            }
            .validate()
            .is_err()
        );
    }
}
