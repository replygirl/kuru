use std::{fmt, time::Duration};

use anyhow::{Error, Result, ensure};
use reqwest::{Response, StatusCode};
use serde_json::Value;

use crate::MAX_BYTES;

const DIAGNOSTIC_BYTES: usize = 8 * 1024;
const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub(super) enum Operation {
    ResponsesCompletion,
    ResponsesCatalog,
    ChatgptCompletion,
    ChatgptCatalog,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResponsesCompletion => "Responses completion",
            Self::ResponsesCatalog => "Responses model catalog",
            Self::ChatgptCompletion => "ChatGPT completion",
            Self::ChatgptCatalog => "ChatGPT model catalog",
        })
    }
}

#[derive(Debug)]
enum TransportKind {
    Timeout,
    Connect,
    Other,
}

#[derive(Debug)]
struct TransportFailure {
    operation: Operation,
    kind: TransportKind,
}

impl fmt::Display for TransportFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            TransportKind::Timeout => "transport timed out",
            TransportKind::Connect => "transport connection failed",
            TransportKind::Other => "transport failed",
        };
        write!(formatter, "{} {kind}", self.operation)
    }
}

impl std::error::Error for TransportFailure {}

pub(super) fn transport(operation: Operation, error: reqwest::Error) -> Error {
    // A connection classification records only the transport symptom. It says
    // nothing about dispatch and must never make a request safe to replay.
    let kind = if error.is_timeout() {
        TransportKind::Timeout
    } else if error.is_connect() {
        TransportKind::Connect
    } else {
        TransportKind::Other
    };
    Error::new(TransportFailure { operation, kind })
}

pub(super) async fn successful(response: Response, operation: Operation) -> Result<Response> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(failed(response, operation).await)
    }
}

pub(super) struct Rejected {
    pub error: Error,
    pub retryable: bool,
}

pub(super) async fn rejected(mut response: Response, operation: Operation) -> Rejected {
    let status = response.status();
    let body = diagnostic_body(&mut response).await;
    let retryable = match status {
        StatusCode::INTERNAL_SERVER_ERROR | StatusCode::SERVICE_UNAVAILABLE => true,
        StatusCode::TOO_MANY_REQUESTS => {
            !matches!(
                operation,
                Operation::ResponsesCompletion | Operation::ResponsesCatalog
            ) || !is_responses_quota(body.as_ref())
        }
        _ => false,
    };
    Rejected {
        error: Error::msg(status_message(operation, status, body.as_ref())),
        retryable,
    }
}

pub(super) async fn json(response: Response, operation: Operation) -> Result<Value> {
    let mut response = successful(response, operation).await?;
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_BYTES as u64,
        "{operation} response exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| transport(operation, error))?
    {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "{operation} response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| protocol(operation))
}

pub(super) fn envelope(value: &Value, operation: Operation) -> Result<()> {
    ensure!(
        value.get("error").is_none_or(Value::is_null),
        "{}",
        protocol(operation)
    );
    ensure!(
        value["status"]
            .as_str()
            .is_none_or(|status| status == "completed"),
        "{}",
        protocol(operation)
    );
    Ok(())
}

pub(super) fn protocol(operation: Operation) -> Error {
    Error::msg(format!("{operation} returned a provider error response"))
}

pub(super) fn invalid_endpoint() -> Error {
    Error::msg("Responses provider API endpoint is invalid")
}

pub(super) fn client() -> Error {
    Error::msg("Provider HTTP client initialization failed")
}

pub(super) fn stream_failed() -> Error {
    Error::msg("ChatGPT completion stream failed before completion")
}

pub(super) fn stream_protocol() -> Error {
    Error::msg("ChatGPT completion stream contained invalid protocol data")
}

pub(super) fn function_arguments(operation: Operation) -> Error {
    Error::msg(format!("{operation} contained invalid function arguments"))
}

pub(super) fn stream_event(event: &Value) -> Error {
    match event
        .pointer("/response/error/code")
        .or_else(|| event.pointer("/error/code"))
        .and_then(Value::as_str)
    {
        Some("context_length_exceeded") => {
            Error::msg("ChatGPT completion exceeded the model context limit")
        }
        Some("insufficient_quota") => {
            Error::msg("ChatGPT completion is blocked by a subscription quota limit")
        }
        Some("usage_not_included") => {
            Error::msg("ChatGPT subscription does not include this usage")
        }
        Some("cyber_policy" | "misalignment_policy_violation" | "bio_policy") => {
            Error::msg("ChatGPT completion was blocked by policy")
        }
        Some("invalid_prompt") => Error::msg("ChatGPT completion request was rejected"),
        Some("server_is_overloaded") => Error::msg("ChatGPT service is overloaded"),
        Some("rate_limit_exceeded") => Error::msg("ChatGPT completion is rate limited"),
        _ => stream_failed(),
    }
}

fn status_message(operation: Operation, status: StatusCode, body: Option<&Value>) -> String {
    let code = body
        .and_then(|body| body.pointer("/error/code"))
        .and_then(Value::as_str);
    let status = status.as_u16();
    let responses = matches!(
        operation,
        Operation::ResponsesCompletion | Operation::ResponsesCatalog
    );
    if responses && matches!(status, 400 | 403 | 404) && code == Some("model_not_found") {
        return format!(
            "{operation} selected model is unavailable or access is denied (HTTP {status})"
        );
    }
    if responses && status == 429 && is_responses_quota(body) {
        return format!("{operation} is blocked by an API quota or billing limit (HTTP 429)");
    }
    match status {
        400 | 422 => format!("{operation} request was rejected (HTTP {status})"),
        401 => format!("{operation} authentication was rejected (HTTP 401)"),
        403 => format!("{operation} access was denied (HTTP 403)"),
        404 => format!("{operation} resource was not found (HTTP 404)"),
        429 => format!("{operation} is rate limited (HTTP 429)"),
        500..=599 => format!("{operation} service failed (HTTP {status})"),
        _ => format!("{operation} failed (HTTP {status})"),
    }
}

async fn failed(mut response: Response, operation: Operation) -> Error {
    let status = response.status();
    let body = diagnostic_body(&mut response).await;
    Error::msg(status_message(operation, status, body.as_ref()))
}

fn is_responses_quota(body: Option<&Value>) -> bool {
    let code = body
        .and_then(|body| body.pointer("/error/code"))
        .and_then(Value::as_str);
    let kind = body
        .and_then(|body| body.pointer("/error/type"))
        .and_then(Value::as_str);
    kind == Some("insufficient_quota")
        || matches!(
            code,
            Some(
                "credit_balance_exhausted"
                    | "organization_usage_limit_exceeded"
                    | "organization_spend_limit_exceeded"
                    | "project_spend_limit_exceeded"
            )
        )
}

async fn diagnostic_body(response: &mut Response) -> Option<Value> {
    if response.content_length().unwrap_or(0) <= DIAGNOSTIC_BYTES as u64 {
        match tokio::time::timeout(DIAGNOSTIC_TIMEOUT, read_diagnostic(response)).await {
            Ok(Ok(Some(bytes))) => serde_json::from_slice(&bytes).ok(),
            Ok(Ok(None) | Err(_)) | Err(_) => None,
        }
    } else {
        None
    }
}

async fn read_diagnostic(
    response: &mut Response,
) -> std::result::Result<Option<Vec<u8>>, reqwest::Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes
            .len()
            .checked_add(chunk.len())
            .is_none_or(|size| size > DIAGNOSTIC_BYTES)
        {
            return Ok(None);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn api_body_mappings_do_not_cross_into_native_statuses() {
        let body = json!({"error":{"code":"model_not_found","type":"insufficient_quota"}});
        assert!(
            status_message(
                Operation::ResponsesCompletion,
                StatusCode::FORBIDDEN,
                Some(&body)
            )
            .contains("selected model")
        );
        assert!(
            status_message(
                Operation::ResponsesCompletion,
                StatusCode::TOO_MANY_REQUESTS,
                Some(&body)
            )
            .contains("quota or billing")
        );
        assert_eq!(
            status_message(
                Operation::ChatgptCompletion,
                StatusCode::FORBIDDEN,
                Some(&body)
            ),
            "ChatGPT completion access was denied (HTTP 403)"
        );
        assert_eq!(
            status_message(
                Operation::ChatgptCompletion,
                StatusCode::TOO_MANY_REQUESTS,
                Some(&body)
            ),
            "ChatGPT completion is rate limited (HTTP 429)"
        );
    }
}
