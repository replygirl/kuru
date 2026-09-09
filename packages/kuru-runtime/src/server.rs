//! Deliberately small A2A 1.0 JSON-RPC binding: direct messages, no task streaming.
use std::{sync::Arc, time::Duration};

use anyhow::{Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::Harness;

pub async fn serve(listener: tokio::net::TcpListener, app: Router) -> Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("signal handler failed: {error}");
            }
        })
        .await?;
    Ok(())
}

#[derive(Clone)]
struct Service {
    harness: Arc<Mutex<Harness>>,
    base: String,
    token: String,
}

pub fn router(harness: Arc<Mutex<Harness>>, base: &str, token: &str) -> Result<Router> {
    ensure!(
        token.len() >= 16,
        "A2A bearer token must contain at least 16 characters"
    );
    let service = Service {
        harness,
        base: base.trim_end_matches('/').into(),
        token: token.into(),
    };
    Ok(Router::new()
        .route("/.well-known/agent-card.json", get(card))
        .route(
            "/agents/{identity}/.well-known/agent-card.json",
            get(part_card),
        )
        .route("/", post(send))
        .route("/agents/{identity}", post(send_part))
        .layer(DefaultBodyLimit::max(131_072))
        .with_state(service))
}

async fn card(State(service): State<Service>) -> Json<Value> {
    Json(agent_card(
        "Kuru",
        "A pool of persistent peers",
        &service.base,
    ))
}
async fn part_card(State(service): State<Service>, Path(identity): Path<String>) -> Response {
    let harness = service.harness.lock().await;
    match harness.resolve(&identity) {
        Ok(id) => Json(agent_card(
            &id,
            "An independently addressable Kuru peer",
            &format!("{}/agents/{id}", service.base),
        ))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn agent_card(name: &str, description: &str, url: &str) -> Value {
    json!({"name":name,"description":description,"version":env!("CARGO_PKG_VERSION"),
        "supportedInterfaces":[{"url":url,"protocolBinding":"JSONRPC","protocolVersion":"1.0"}],
        "capabilities":{"streaming":false,"pushNotifications":false},
        "defaultInputModes":["text/plain"],"defaultOutputModes":["text/plain"],
        "securitySchemes":{"bearer":{"httpAuthSecurityScheme":{"scheme":"Bearer"}}},"securityRequirements":[{"schemes":{"bearer":{"list":[]}}}],
        "skills":[{"id":"conversation","name":"Conversation and workspace tasks","description":description,"tags":["chat","peers"]}]})
}

async fn send(
    State(service): State<Service>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    dispatch(service, headers, request, None).await
}
async fn send_part(
    State(service): State<Service>,
    Path(identity): Path<String>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    dispatch(service, headers, request, Some(identity)).await
}

async fn dispatch(
    service: Service,
    headers: HeaderMap,
    request: Value,
    identity: Option<String>,
) -> Response {
    if headers.get("authorization").and_then(|h| h.to_str().ok())
        != Some(&format!("Bearer {}", service.token))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if headers.get("a2a-version").is_some_and(|v| v != "1.0") {
        return error(id, -32009, "unsupported A2A version; supported: 1.0");
    }
    if request["jsonrpc"] != "2.0" || !(id.is_string() || id.is_number()) {
        return error(id, -32600, "invalid JSON-RPC request");
    }
    if request["method"] != "SendMessage" {
        return error(id, -32601, "only SendMessage is supported");
    }
    let message = &request["params"]["message"];
    if !matches!(message["role"].as_str(), Some("ROLE_USER" | "ROLE_AGENT"))
        || message["messageId"].as_str().is_none_or(|s| s.is_empty())
    {
        return error(id, -32602, "messageId and valid role are required");
    }
    let Some(parts) = message["parts"].as_array().filter(|p| !p.is_empty()) else {
        return error(id, -32602, "text parts required");
    };
    let Some(texts) = parts
        .iter()
        .map(|p| p["text"].as_str())
        .collect::<Option<Vec<_>>>()
    else {
        return error(id, -32602, "only text parts are supported");
    };
    let prompt = texts.join("\n");
    if prompt.trim().is_empty() {
        return error(id, -32602, "message cannot be empty");
    }
    if message.get("taskId").is_some() {
        return error(
            id,
            -32001,
            "this direct-message agent does not persist A2A tasks",
        );
    }
    if request["params"]["configuration"]
        .get("pushNotificationConfig")
        .is_some()
    {
        return error(id, -32003, "push notifications unsupported");
    }
    let context = message["contextId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if context.len() > 256 {
        return error(id, -32602, "contextId too long");
    }
    let run = async {
        let mut harness = service.harness.lock().await;
        harness.run_for(&prompt, identity.as_deref()).await
    };
    match tokio::time::timeout(Duration::from_secs(600), run).await {
        Ok(Ok(output)) => Json(json!({"jsonrpc":"2.0","id":id,"result":{"message":{
            "messageId":Uuid::new_v4().to_string(),"contextId":context,"role":"ROLE_AGENT","parts":[{"text":output.text}],"metadata":{"speaker":output.speaker}
        }}})).into_response(),
        Ok(Err(error_value)) => error(id, -32603, &format!("{error_value:#}")),
        Err(_) => error(id, -32603, "agent request timed out"),
    }
}

fn error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})).into_response()
}
