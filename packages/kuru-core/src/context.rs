//! Non-content measurements for an estimated per-request context fit.
//!
//! The byte-derived estimate is deliberately labelled: it is not a tokenizer
//! or an upper bound on provider tokens. Serialized transport limits remain a
//! separate connector check.

use std::fmt;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::Sourced;

pub const DEFAULT_OUTPUT_RESERVE_TOKENS: u64 = 8_192;
pub const MAX_CONFIGURED_OUTPUT_RESERVE_TOKENS: u64 = 2_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBudget {
    pub window: Sourced<u64>,
    pub output_reserve_tokens: u64,
}

impl ContextBudget {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.window.value > 0, "context window must be positive");
        ensure!(
            (1..=MAX_CONFIGURED_OUTPUT_RESERVE_TOKENS).contains(&self.output_reserve_tokens),
            "context output reserve must be between 1 and 2000000 tokens"
        );
        Ok(())
    }

    /// Direct callers without an explicit runtime budget still receive a
    /// documented assumed window; they never bypass native preflight.
    pub fn legacy_default() -> Self {
        Self::resolve(Sourced::built_in(128_000), None, None)
            .expect("fixed built-in context budget is valid")
    }

    /// Reserve at most a quarter of a tiny window by default. Provider output
    /// metadata narrows this reserve; it does not become a new wire parameter.
    pub fn resolve(
        window: Sourced<u64>,
        model_max_output_tokens: Option<u64>,
        configured_reserve_tokens: Option<u64>,
    ) -> Result<Self> {
        ensure!(window.value > 0, "context window must be positive");
        let default = DEFAULT_OUTPUT_RESERVE_TOKENS
            .min(
                model_max_output_tokens
                    .filter(|value| *value > 0)
                    .unwrap_or(u64::MAX),
            )
            .min((window.value / 4).max(1));
        let output_reserve_tokens = configured_reserve_tokens.unwrap_or(default);
        ensure!(
            (1..=MAX_CONFIGURED_OUTPUT_RESERVE_TOKENS).contains(&output_reserve_tokens),
            "context output reserve must be between 1 and 2000000 tokens"
        );
        let budget = Self {
            window,
            output_reserve_tokens,
        };
        budget.validate()?;
        Ok(budget)
    }

    pub fn check_fit(
        &self,
        estimated_input_tokens: u64,
        native_continuation_mandatory: bool,
    ) -> std::result::Result<(), ContextTooLarge> {
        if estimated_input_tokens
            .checked_add(self.output_reserve_tokens)
            .is_some_and(|total| total <= self.window.value)
        {
            Ok(())
        } else {
            Err(ContextTooLarge {
                estimated_input_tokens,
                output_reserve_tokens: self.output_reserve_tokens,
                window_tokens: self.window.value,
                native_continuation_mandatory,
            })
        }
    }
}

/// Fixed categories only: no prompt text, actor names or native continuation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ContextSourceKind {
    Instructions,
    /// Connector-level aggregate; runtime classifies embedded optional rows.
    SelectedInstructions,
    ToolSchemas,
    PublicTranscript,
    PrivateHistory,
    Notes,
    /// Connector-level aggregate; runtime classifies its constituent rows.
    SelectedInput,
    CurrentInput,
    RequiredReceipts,
    NativeContinuation,
    WireOverhead,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextSourceSize {
    pub kind: ContextSourceKind,
    pub serialized_bytes: u64,
    pub estimated_tokens: u64,
    pub units: u64,
    pub mandatory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextEstimate {
    pub budget: ContextBudget,
    pub final_body_bytes: u64,
    pub estimated_input_tokens: u64,
    /// Native pending input/output and matching receipts cannot be sliced to
    /// make this request fit.
    pub native_continuation_mandatory: bool,
    pub sources: Vec<ContextSourceSize>,
}

impl ContextEstimate {
    /// A deliberately conservative *estimate* from final serialized request
    /// bytes. Punctuation-rich inputs may still tokenize more densely.
    pub fn for_final_body(
        budget: ContextBudget,
        final_body_bytes: u64,
        native_continuation_mandatory: bool,
        sources: Vec<ContextSourceSize>,
    ) -> Self {
        Self {
            budget,
            final_body_bytes,
            estimated_input_tokens: estimated_tokens_for_bytes(final_body_bytes),
            native_continuation_mandatory,
            sources,
        }
    }

    pub fn ensure_fits(&self) -> std::result::Result<(), ContextTooLarge> {
        self.budget.check_fit(
            self.estimated_input_tokens,
            self.native_continuation_mandatory,
        )
    }
}

pub const fn estimated_tokens_for_bytes(bytes: u64) -> u64 {
    bytes / 2 + bytes % 2
}

/// Fixed text keeps private request content out of diagnostics and journals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextTooLarge {
    pub estimated_input_tokens: u64,
    pub output_reserve_tokens: u64,
    pub window_tokens: u64,
    pub native_continuation_mandatory: bool,
}

impl fmt::Display for ContextTooLarge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "estimated provider request exceeds context window before dispatch (input estimate {}, output reserve {}, window {} tokens)",
            self.estimated_input_tokens, self.output_reserve_tokens, self.window_tokens
        )
    }
}

impl std::error::Error for ContextTooLarge {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_window_reserve_is_bounded_and_failure_is_typed() {
        let budget =
            ContextBudget::resolve(Sourced::configured_assumption(120), Some(80), None).unwrap();
        assert_eq!(budget.output_reserve_tokens, 30);
        assert!(budget.check_fit(90, false).is_ok());
        assert_eq!(budget.check_fit(91, false).unwrap_err().window_tokens, 120);
        let configured =
            ContextBudget::resolve(Sourced::configured_assumption(120), None, Some(121)).unwrap();
        assert!(configured.check_fit(0, false).is_err());
        assert!(
            ContextBudget {
                window: Sourced::configured_assumption(0),
                output_reserve_tokens: 1,
            }
            .validate()
            .is_err()
        );
        assert!(
            ContextBudget {
                window: Sourced::configured_assumption(120),
                output_reserve_tokens: 0,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn byte_derived_estimate_is_labelled_and_keeps_no_request_content() {
        let budget = ContextBudget::resolve(Sourced::built_in(128_000), None, None).unwrap();
        let estimate = ContextEstimate::for_final_body(budget, 9, false, vec![]);
        assert_eq!(estimate.estimated_input_tokens, 5);
        assert!(estimate.ensure_fits().is_ok());
    }
}
