use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

/// A finite reason for a positively observed contradiction in the native
/// ChatGPT wire contract. These values never contain provider-supplied text.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityCode {
    MissingTokenField,
    MissingCatalogField,
    MissingCompletionField,
    TerminalIdentityContradiction,
    TerminalOutputContradiction,
}

impl CompatibilityCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingTokenField => "missing_token_field",
            Self::MissingCatalogField => "missing_catalog_field",
            Self::MissingCompletionField => "missing_completion_field",
            Self::TerminalIdentityContradiction => "terminal_identity_contradiction",
            Self::TerminalOutputContradiction => "terminal_output_contradiction",
        }
    }

    pub const fn action(self) -> &'static str {
        match self {
            Self::MissingTokenField => "Update Kuru before retrying ChatGPT sign-in.",
            Self::MissingCatalogField => "Update Kuru before using the ChatGPT model catalog.",
            Self::MissingCompletionField => "Update Kuru before retrying this ChatGPT operation.",
            Self::TerminalIdentityContradiction | Self::TerminalOutputContradiction => {
                "Update Kuru before retrying this ChatGPT operation."
            }
        }
    }
}

/// Sanitized typed evidence that a complete native response contradicted a
/// required contract semantic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectorIncompatibility {
    code: CompatibilityCode,
}

impl ConnectorIncompatibility {
    pub(crate) const fn new(code: CompatibilityCode) -> Self {
        Self { code }
    }

    pub const fn code(self) -> CompatibilityCode {
        self.code
    }

    pub const fn action(self) -> &'static str {
        self.code.action()
    }
}

impl fmt::Display for ConnectorIncompatibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "ChatGPT protocol incompatibility: {}. {}",
            self.code.as_str(),
            self.code.action()
        )
    }
}

impl Error for ConnectorIncompatibility {}

/// Find typed compatibility evidence without formatting or returning the
/// original provider error chain.
pub fn incompatibility(error: &anyhow::Error) -> Option<ConnectorIncompatibility> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ConnectorIncompatibility>().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_actions_are_finite_and_do_not_retain_remote_sentinels() {
        let sentinel = "provider-body-token-sentinel";
        let error = anyhow::Error::new(ConnectorIncompatibility::new(
            CompatibilityCode::TerminalIdentityContradiction,
        ))
        .context(sentinel);
        let found = incompatibility(&error).unwrap();
        let rendered = format!("{found} {}", found.action());
        assert_eq!(found.code().as_str(), "terminal_identity_contradiction");
        assert!(!rendered.contains(sentinel));
        assert!(rendered.len() <= 256);
    }
}
