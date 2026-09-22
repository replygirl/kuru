//! Typed storage operations carried only after an authenticated generation handshake.
//! A failed response or broken connection is never permission to replay a write.

use anyhow::{Context, Result, bail, ensure};
use kuru_core::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use super::{EndpointAuthority, read_frame, write_frame};
use crate::{HistoryWindow, StoredNote, store::MemoryStore};

// JSON can escape a valid 16 MiB typed message by up to six times. Keep the
// frame bounded while leaving the existing message limit representable.
const OPERATION_FRAME_LIMIT: usize = 100 * 1024 * 1024;
const OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(35);
pub(super) const FRAME_BUDGET_MIB: usize = 128;
const MIB: usize = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRequest {
    pub id: Uuid,
    pub generation: String,
    pub call: ServiceCall,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceCall {
    AppendMessage { namespace: String, message: Message },
    HistoryWindow { namespace: String, limit: usize },
    Notes { namespace: String, limit: usize },
    PutMany { values: Vec<(String, Value)> },
    Get { key: String },
    Reconcile,
    Revision,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceReply {
    pub id: Uuid,
    pub generation: String,
    pub response: ServiceResponse,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ServiceResponse {
    Success(ServiceValue),
    Rejected(ServiceFault),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ServiceValue {
    Unit,
    HistoryWindow(HistoryWindow),
    Notes(Vec<StoredNote>),
    StoredValue(Option<Value>),
    Reconciled(Option<bool>),
    Revision(String),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceFault {
    GenerationChanged,
    StorageFailed,
}

impl ServiceRequest {
    pub fn new(generation: &str, call: ServiceCall) -> Self {
        Self {
            id: Uuid::new_v4(),
            generation: generation.to_owned(),
            call,
        }
    }
}

/// One request at a time per authenticated connection keeps reply ownership
/// unambiguous. The service can concurrently serve independent connections;
/// `MemoryStore` itself serializes short mutations.
#[cfg(test)]
pub async fn serve_one<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
) -> Result<()> {
    ensure!(
        super::accept_handshake(stream, authority).await?.is_ok(),
        "memory service handshake rejected"
    );
    let request: ServiceRequest =
        read_frame(stream, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    respond(stream, authority, store, request).await
}

/// An open connection is an attachment. Waiting for its next complete frame
/// does not impose an idle timeout on an inference turn; partial frames remain
/// bounded, and EOF releases the attachment.
pub async fn serve_attached<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
    budget: Arc<Semaphore>,
) -> Result<()> {
    ensure!(
        super::accept_handshake(stream, authority).await?.is_ok(),
        "memory service handshake rejected"
    );
    while let Some((request, _bytes)) = read_next(stream, budget.clone()).await? {
        respond(stream, authority, store, request).await?;
    }
    Ok(())
}

async fn read_next<R: AsyncRead + Unpin>(
    reader: &mut R,
    budget: Arc<Semaphore>,
) -> Result<Option<(ServiceRequest, OwnedSemaphorePermit)>> {
    let mut first = [0u8; 1];
    if reader.read(&mut first).await? == 0 {
        return Ok(None);
    }
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        let mut rest = [0u8; 3];
        reader.read_exact(&mut rest).await?;
        let length = u32::from_be_bytes([first[0], rest[0], rest[1], rest[2]]) as usize;
        ensure!(
            length > 0 && length <= OPERATION_FRAME_LIMIT,
            "memory service frame exceeds its limit"
        );
        let units = u32::try_from(length.div_ceil(MIB))?;
        let bytes = budget
            .acquire_many_owned(units)
            .await
            .context("memory service frame budget closed")?;
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload).await?;
        let request = serde_json::from_slice::<ServiceRequest>(&payload)
            .context("decode typed memory service request")?;
        Ok::<_, anyhow::Error>((request, bytes))
    })
    .await
    .context("memory service request frame deadline exceeded")?
    .map(Some)
}

async fn respond<S: AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
    request: ServiceRequest,
) -> Result<()> {
    let response = if request.generation == authority.service_generation {
        match dispatch(store, request.call).await {
            Ok(value) => ServiceResponse::Success(value),
            Err(error) => {
                tracing::warn!(error = %error, "memory service operation failed");
                ServiceResponse::Rejected(ServiceFault::StorageFailed)
            }
        }
    } else {
        ServiceResponse::Rejected(ServiceFault::GenerationChanged)
    };
    write_frame(
        stream,
        &ServiceReply {
            id: request.id,
            generation: authority.service_generation.clone(),
            response,
        },
        OPERATION_FRAME_LIMIT,
        OPERATION_TIMEOUT,
    )
    .await
}

#[cfg(test)]
pub async fn request_one<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    call: ServiceCall,
) -> Result<ServiceValue> {
    super::connect_handshake(stream, authority).await?;
    request_attached(stream, authority, call).await
}

pub async fn request_attached<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    call: ServiceCall,
) -> Result<ServiceValue> {
    let request = ServiceRequest::new(&authority.service_generation, call);
    write_frame(stream, &request, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    let reply: ServiceReply = read_frame(stream, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    ensure!(reply.id == request.id, "memory service reply ID changed");
    ensure!(
        reply.generation == authority.service_generation,
        "memory service reply generation changed"
    );
    match reply.response {
        ServiceResponse::Success(value) => Ok(value),
        ServiceResponse::Rejected(ServiceFault::GenerationChanged) => {
            bail!("memory service generation changed during the operation")
        }
        ServiceResponse::Rejected(ServiceFault::StorageFailed) => {
            bail!("memory service storage operation failed")
        }
    }
}

async fn dispatch(store: &MemoryStore, call: ServiceCall) -> Result<ServiceValue> {
    let value = match call {
        ServiceCall::AppendMessage { namespace, message } => {
            store.append_message(&namespace, &message).await?;
            ServiceValue::Unit
        }
        ServiceCall::HistoryWindow { namespace, limit } => {
            ServiceValue::HistoryWindow(store.history_window(&namespace, limit).await?)
        }
        ServiceCall::Notes { namespace, limit } => {
            ServiceValue::Notes(store.notes(&namespace, limit).await?)
        }
        ServiceCall::PutMany { values } => {
            store.put_many(&values).await?;
            ServiceValue::Unit
        }
        ServiceCall::Get { key } => ServiceValue::StoredValue(store.get(&key).await?),
        ServiceCall::Reconcile => ServiceValue::Reconciled(store.reconcile().await?),
        ServiceCall::Revision => ServiceValue::Revision(store.revision().await?),
    };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, duplex};

    #[tokio::test(start_paused = true)]
    async fn partial_request_header_expires_without_waiting_for_peer_eof() {
        let (mut client, mut server) = duplex(64);
        client.write_all(&[0]).await.unwrap();
        let error = read_next(&mut server, Arc::new(Semaphore::new(FRAME_BUDGET_MIB)))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("deadline exceeded"));
    }
}
