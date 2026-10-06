use std::{path::Path, time::Duration};

use kuru_core::{CompletionRequest, ContentBlock, Message};
use serde::Serialize;

use crate::{AuthManager, CompatibilityCode, ConnectorIncompatibility, incompatibility};

use super::{Provider, ResponsesProvider};

const CANARY_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryState {
    Verified,
    Incompatible,
    Unverified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryStage {
    Credentials,
    Catalog,
    Completion,
    Usage,
    Refresh,
    ToolCall,
    Reasoning,
}

impl CanaryStage {
    const ALL: [Self; 7] = [
        Self::Credentials,
        Self::Catalog,
        Self::Completion,
        Self::Usage,
        Self::Refresh,
        Self::ToolCall,
        Self::Reasoning,
    ];
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CanaryReport {
    pub schema_version: u8,
    pub state: CanaryState,
    pub reason: Option<&'static str>,
    pub incompatibility: Option<CompatibilityCode>,
    pub action: Option<&'static str>,
    pub observed: Vec<CanaryStage>,
    pub unobserved: Vec<CanaryStage>,
}

#[derive(Clone, Copy)]
enum Failure {
    Incompatible(CompatibilityCode),
    Unverified(&'static str),
}

pub async fn subscription_canary(data_dir: &Path, tool_root: &Path, model: &str) -> CanaryReport {
    let manager = match AuthManager::new(data_dir.to_owned(), tool_root.to_owned(), None) {
        Ok(manager) => manager,
        Err(_) => {
            return report(
                CanaryState::Unverified,
                Some("credentials_unavailable"),
                None,
                Vec::new(),
            );
        }
    };
    with_manager(manager, model, None).await
}

/// Test-only endpoint override used by fresh-process loopback fixtures.
#[cfg(feature = "test-support")]
pub async fn subscription_canary_test_endpoint(
    data_dir: &Path,
    tool_root: &Path,
    model: &str,
    endpoint: &str,
) -> CanaryReport {
    if !is_numeric_loopback_endpoint(endpoint) {
        return report(
            CanaryState::Unverified,
            Some("test_endpoint_rejected"),
            None,
            Vec::new(),
        );
    }
    let manager = match AuthManager::new(data_dir.to_owned(), tool_root.to_owned(), None) {
        Ok(manager) => manager,
        Err(_) => {
            return report(
                CanaryState::Unverified,
                Some("credentials_unavailable"),
                None,
                Vec::new(),
            );
        }
    };
    with_manager(manager, model, Some(endpoint)).await
}

#[cfg(feature = "test-support")]
fn is_numeric_loopback_endpoint(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    let Some(host) = url
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
    else {
        return false;
    };
    url.scheme() == "http"
        && host.is_loopback()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
}

pub(super) async fn with_manager(
    manager: AuthManager,
    model: &str,
    test_endpoint: Option<&str>,
) -> CanaryReport {
    let mut observed = Vec::new();
    let result = tokio::time::timeout(CANARY_TIMEOUT, async {
        let initial = manager
            .credentials_snapshot()
            .await
            .map_err(|_| Failure::Unverified("credentials_unavailable"))?;
        observed.push(CanaryStage::Credentials);
        let provider = ResponsesProvider::subscription(manager, initial)
            .map_err(|_| Failure::Unverified("provider_unavailable"))?;
        #[cfg(any(test, feature = "test-support"))]
        let mut provider = provider;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(endpoint) = test_endpoint {
            provider.base = endpoint.to_owned();
        }
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = test_endpoint;
        let models = provider.models().await.map_err(classify_error)?;
        observed.push(CanaryStage::Catalog);
        if !models.iter().any(|available| available.id == model) {
            return Err(Failure::Unverified("model_unavailable"));
        }
        let completion = provider
            .complete(CompletionRequest {
                actor: "kuru-canary".into(),
                instructions: "Reply with the single word OK.".into(),
                messages: vec![Message::text("user", "Reply with OK.")],
                current_message_count: Some(1),
                context_budget: None,
                model: model.to_owned(),
                effort: None,
                tools: Vec::new(),
            })
            .await
            .map_err(classify_error)?;
        observed.push(CanaryStage::Completion);
        if completion.usage.input_tokens.is_some()
            || completion.usage.output_tokens.is_some()
            || completion.usage.cached_input_tokens.is_some()
            || completion.usage.reasoning_output_tokens.is_some()
        {
            observed.push(CanaryStage::Usage);
        }
        if completion
            .blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
        {
            observed.push(CanaryStage::ToolCall);
            return Err(Failure::Unverified("unexpected_tool_call"));
        }
        if completion.blocks.is_empty() {
            return Err(Failure::Unverified("empty_completion"));
        }
        Ok::<(), Failure>(())
    })
    .await;

    let (state, reason, incompatible) = match result {
        Ok(Ok(())) => (CanaryState::Verified, None, None),
        Ok(Err(Failure::Incompatible(code))) => (
            CanaryState::Incompatible,
            Some("completed_incompatibility"),
            Some(code),
        ),
        Ok(Err(Failure::Unverified(reason))) => (CanaryState::Unverified, Some(reason), None),
        Err(_) => (CanaryState::Unverified, Some("deadline_exceeded"), None),
    };
    report(state, reason, incompatible, observed)
}

fn report(
    state: CanaryState,
    reason: Option<&'static str>,
    incompatibility: Option<CompatibilityCode>,
    observed: Vec<CanaryStage>,
) -> CanaryReport {
    let action = incompatibility.map(CompatibilityCode::action).or_else(|| {
        reason.map(|reason| match reason {
            "credentials_unavailable" => {
                "Run kuru login if you choose to retry the subscription canary."
            }
            "model_unavailable" => "Choose a model listed by the ChatGPT catalog.",
            "unexpected_tool_call" => "No tool was run; retry later or choose another model.",
            _ => "Retry later if you choose; this observation is inconclusive.",
        })
    });
    let unobserved = CanaryStage::ALL
        .into_iter()
        .filter(|stage| !observed.contains(stage))
        .collect();
    CanaryReport {
        schema_version: 1,
        state,
        reason,
        incompatibility,
        action,
        observed,
        unobserved,
    }
}

fn classify_error(error: anyhow::Error) -> Failure {
    incompatibility(&error)
        .map(ConnectorIncompatibility::code)
        .map(Failure::Incompatible)
        .unwrap_or(Failure::Unverified("provider_operation_unavailable"))
}

#[cfg(all(test, feature = "test-support"))]
mod test_endpoint_tests {
    use super::*;

    #[test]
    fn fixture_endpoint_requires_numeric_http_loopback() {
        for endpoint in [
            "https://127.0.0.1:1234",
            "http://localhost:1234",
            "http://192.0.2.1:1234",
            "http://user@127.0.0.1:1234",
            "http://127.0.0.1:1234/path",
        ] {
            assert!(!is_numeric_loopback_endpoint(endpoint), "{endpoint}");
        }
        assert!(is_numeric_loopback_endpoint("http://127.0.0.1:1234"));
    }
}
