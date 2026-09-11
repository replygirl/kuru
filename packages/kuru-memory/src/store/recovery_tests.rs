//! Real-server recovery faults. Packet fixtures never log authentication payloads.
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

const TEST_DEADLINE: Duration = Duration::from_secs(10);
const FRAME_LIMIT: usize = 128 * 1024;

#[tokio::test]
async fn live_original_session_blocks_receipt_reconciliation_even_after_commit() {
    let store = MemoryStore::temporary().await.unwrap();
    let operation = Uuid::new_v4().to_string();
    let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
    apply(
        &mut connection,
        &operation,
        "barrier",
        Mutation::State(vec![("settled".into(), "true".into())]),
    )
    .await
    .unwrap();
    let committed_revision = store.revision().await.unwrap();
    assert!(operation_exists(&store.pool, &operation).await.unwrap());
    *store.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Operation(operation),
    });
    // Receipt existence alone must not clear an original session that could
    // still finish another part of the accepted operation.
    assert!(
        tokio::time::timeout(Duration::from_millis(80), store.reconcile())
            .await
            .is_err()
    );
    assert!(store.shared.uncertain.lock().unwrap().is_some());
    assert!(
        await_session_end(&store.pool, id, Duration::from_millis(30))
            .await
            .is_err()
    );
    drop(connection);
    tokio::time::timeout(TEST_DEADLINE, store.reconcile())
        .await
        .unwrap()
        .unwrap();
    assert!(store.shared.uncertain.lock().unwrap().is_none());
    assert_eq!(store.get("settled").await.unwrap(), Some(json!(true)));
    store.reconcile().await.unwrap();
    assert_eq!(store.revision().await.unwrap(), committed_revision);
    store.close().await.unwrap();
}

#[tokio::test]
async fn dropped_uncommitted_session_resolves_absent_receipt_only_after_teardown() {
    let store = MemoryStore::temporary().await.unwrap();
    let revision = store.revision().await.unwrap();
    let operation = Uuid::new_v4().to_string();
    let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
    sqlx::query("START TRANSACTION")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO state (`key`, value) VALUES ('uncommitted', 'true')")
        .execute(&mut connection)
        .await
        .unwrap();
    *store.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Operation(operation),
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), store.reconcile())
            .await
            .is_err()
    );
    assert!(store.shared.uncertain.lock().unwrap().is_some());
    drop(connection);
    assert_eq!(
        tokio::time::timeout(TEST_DEADLINE, store.resolve_uncertain())
            .await
            .unwrap()
            .unwrap(),
        Some(false)
    );
    assert!(store.get("uncommitted").await.unwrap().is_none());
    assert_eq!(store.revision().await.unwrap(), revision);
    store.put("after rollback", &json!(1)).await.unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn in_flight_disconnect_waits_for_real_query_and_session_teardown() {
    let store = MemoryStore::temporary().await.unwrap();
    let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
    let running = tokio::spawn(async move {
        let _ = sqlx::query("SELECT SLEEP(5)")
            .execute(&mut connection)
            .await;
    });
    tokio::time::timeout(TEST_DEADLINE, async {
        loop {
            let query: Option<String> =
                sqlx::query_scalar("SELECT INFO FROM information_schema.processlist WHERE ID = ?")
                    .bind(id)
                    .fetch_optional(store.pool.as_ref())
                    .await
                    .unwrap();
            if query.is_some_and(|query| query.contains("SLEEP(5)")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        await_session_end(&store.pool, id, Duration::from_millis(30))
            .await
            .is_err()
    );
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    await_session_end(&store.pool, id, TEST_DEADLINE)
        .await
        .unwrap();
    // Independent authenticated readers must survive this owned socket close.
    assert_eq!(store.status().await.unwrap().engine, "dolt");
    store.close().await.unwrap();
}

#[tokio::test]
async fn lost_commit_reply_recovers_one_durable_update_and_reopens_without_replay() {
    let directory = crate::test_support::tempdir().unwrap();
    let options = crate::test_support::open_options(
        directory.path().to_path_buf(),
        format!("project/{}", "e".repeat(64)),
    )
    .unwrap();
    let store = MemoryStore::open(options.clone()).await.unwrap();
    let before = store.revision().await.unwrap();
    let proxy = AckDropProxy::start(store.pool.clone(), "COMMIT", before).await;
    let affected = proxy.view(&store).await;
    tokio::time::timeout(
        TEST_DEADLINE,
        affected.append("conversation", "user", "exactly once"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        proxy.discarded.load(Ordering::Acquire),
        "fixture must discard an actual durable COMMIT reply"
    );
    assert!(store.shared.uncertain.lock().unwrap().is_none());
    let revision = store.revision().await.unwrap();
    let operations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM operations")
        .fetch_one(store.pool.as_ref())
        .await
        .unwrap();
    assert_eq!(operations, 1);
    assert_eq!(store.history("conversation", 10).await.unwrap().len(), 1);
    affected.reconcile().await.unwrap();
    assert_eq!(store.revision().await.unwrap(), revision);
    affected.pool.close().await;
    proxy.close().await;
    store.close().await.unwrap();
    let reopened = MemoryStore::open(options).await.unwrap();
    assert_eq!(reopened.revision().await.unwrap(), revision);
    assert_eq!(
        reopened.history("conversation", 10).await.unwrap()[0].content,
        "exactly once"
    );
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn lost_promotion_reply_reconciles_target_once_and_keeps_later_writes() {
    let store = MemoryStore::temporary().await.unwrap();
    store.put("before", &json!(true)).await.unwrap();
    let candidate = store.begin_candidate("acknowledgement test").await.unwrap();
    candidate
        .view()
        .put("dream", &json!("accepted"))
        .await
        .unwrap();
    let target = candidate.view().revision().await.unwrap();
    let proxy = AckDropProxy::start(
        store.pool.clone(),
        "CALL DOLT_MERGE",
        candidate.base.clone(),
    )
    .await;
    let affected = proxy.view(&store).await;
    let candidate = Candidate {
        live: affected.clone(),
        view: candidate.view(),
        base: candidate.base,
    };
    assert_eq!(
        tokio::time::timeout(TEST_DEADLINE, candidate.promote())
            .await
            .unwrap()
            .unwrap(),
        target
    );
    assert!(
        proxy.discarded.load(Ordering::Acquire),
        "fixture must discard a real completed merge reply"
    );
    assert!(store.shared.uncertain.lock().unwrap().is_none());
    assert_eq!(store.get("dream").await.unwrap(), Some(json!("accepted")));
    assert_eq!(candidate.promote().await.unwrap(), target);
    store.put("later", &json!("retained")).await.unwrap();
    let after = store.revision().await.unwrap();
    affected.reconcile().await.unwrap();
    assert_eq!(store.revision().await.unwrap(), after);
    assert_eq!(store.get("later").await.unwrap(), Some(json!("retained")));
    affected.pool.close().await;
    proxy.close().await;
    store.close().await.unwrap();
}

#[tokio::test]
async fn promotion_receipt_keeps_base_target_and_refuses_divergent_history() {
    let store = MemoryStore::temporary().await.unwrap();
    let candidate = store.begin_candidate("receipt barrier").await.unwrap();
    candidate.view().put("candidate", &json!(1)).await.unwrap();
    let target = candidate.view().revision().await.unwrap();
    let (connection, id) = owned_connection(&store.pool).await.unwrap();
    *store.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Promotion {
            base: candidate.base.clone(),
            target: target.clone(),
        },
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), store.reconcile())
            .await
            .is_err()
    );
    assert!(
        matches!(&store.shared.uncertain.lock().unwrap().as_ref().unwrap().receipt, Receipt::Promotion { base, target: pending } if base == candidate.base() && pending == &target)
    );
    drop(connection);
    assert_eq!(store.resolve_uncertain().await.unwrap(), Some(false));
    assert!(store.get("candidate").await.unwrap().is_none());
    store.put("live", &json!(2)).await.unwrap();
    *store.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Promotion {
            base: candidate.base,
            target,
        },
    });
    assert!(
        store
            .reconcile()
            .await
            .unwrap_err()
            .to_string()
            .contains("diverged")
    );
    assert!(store.shared.uncertain.lock().unwrap().is_some());
    assert!(store.put("must not write", &json!(3)).await.is_err());
    assert!(store.get("must not write").await.unwrap().is_none());
    store.close().await.unwrap();
}

struct AckDropProxy {
    port: u16,
    discarded: Arc<AtomicBool>,
    task: JoinHandle<Result<()>>,
}

impl AckDropProxy {
    async fn start(observer: Arc<MySqlPool>, statement: &'static str, base: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let upstream = observer.connect_options();
        let upstream = (upstream.get_host().to_owned(), upstream.get_port());
        let discarded = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(Fault {
            observer,
            statement,
            base,
            discarded: discarded.clone(),
        });
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    incoming = listener.accept() => {
                        let (client, _) = incoming?;
                        let server = TcpStream::connect((upstream.0.as_str(), upstream.1)).await?;
                        let fault = fault.clone();
                        connections.spawn(async move { proxy_connection(client, server, fault).await });
                    }
                    Some(finished) = connections.join_next(), if !connections.is_empty() => { finished??; }
                }
            }
        });
        Self {
            port,
            discarded,
            task,
        }
    }

    async fn view(&self, store: &MemoryStore) -> MemoryStore {
        let options = store
            .pool
            .connect_options()
            .as_ref()
            .clone()
            .host("127.0.0.1")
            .port(self.port);
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(3))
            .connect_with(options)
            .await
            .unwrap();
        MemoryStore {
            shared: store.shared.clone(),
            pool: Arc::new(pool),
            branch: store.branch.clone(),
        }
    }

    async fn close(self) {
        self.task.abort();
        let result = self.task.await;
        assert!(
            result.as_ref().is_ok_and(|value| value.is_ok())
                || result.as_ref().is_err_and(|error| error.is_cancelled()),
            "packet proxy failed before expected shutdown"
        );
    }
}

struct Fault {
    observer: Arc<MySqlPool>,
    statement: &'static str,
    base: String,
    discarded: Arc<AtomicBool>,
}

#[derive(Default)]
struct WireState {
    authenticated: bool,
    preparing: bool,
    prepared: Option<u32>,
    discard_next: bool,
}

async fn proxy_connection(client: TcpStream, server: TcpStream, fault: Arc<Fault>) -> Result<()> {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();
    let state = Arc::new(StdMutex::new(WireState::default()));
    let requests = state.clone();
    let statement = fault.statement;
    let client_to_server = async move {
        while let Some(packet) = read_packet(&mut client_read).await? {
            {
                let mut state = requests.lock().unwrap();
                let payload = &packet[4..];
                if state.authenticated && !payload.is_empty() {
                    match payload[0] {
                        0x16 => state.preparing = payload[1..].starts_with(statement.as_bytes()),
                        0x03 => state.discard_next = payload[1..].starts_with(statement.as_bytes()),
                        0x17 if payload.len() >= 5 => {
                            let id = u32::from_le_bytes(payload[1..5].try_into().unwrap());
                            state.discard_next = state.prepared == Some(id);
                        }
                        _ => {}
                    }
                }
            }
            server_write.write_all(&packet).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let server_to_client = async move {
        while let Some(packet) = read_packet(&mut server_read).await? {
            let discard = {
                let mut state = state.lock().unwrap();
                let payload = &packet[4..];
                if !state.authenticated && packet[3] >= 2 && payload.first() == Some(&0) {
                    state.authenticated = true;
                }
                if state.preparing && payload.len() >= 5 && payload[0] == 0 {
                    state.prepared = Some(u32::from_le_bytes(payload[1..5].try_into().unwrap()));
                    state.preparing = false;
                }
                std::mem::take(&mut state.discard_next)
                    && fault
                        .discarded
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
            };
            if discard {
                // Receiving a packet alone may only mean result metadata is
                // ready. Observe the durable branch revision directly before
                // discarding the reply, so the injected fault is a lost ack.
                tokio::time::timeout(TEST_DEADLINE, async {
                    loop {
                        if revision(&fault.observer).await? != fault.base {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .context("intercepted SQL did not become durable")??;
                client_write.shutdown().await?;
                return Ok::<_, anyhow::Error>(());
            }
            client_write.write_all(&packet).await?;
        }
        Ok(())
    };
    tokio::select! {
        result = client_to_server => result,
        result = server_to_client => result,
    }
}

async fn read_packet(reader: &mut (impl AsyncRead + Unpin)) -> Result<Option<Vec<u8>>> {
    let mut header = [0_u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    }
    let length =
        usize::from(header[0]) | usize::from(header[1]) << 8 | usize::from(header[2]) << 16;
    ensure!(length <= FRAME_LIMIT, "fixture MySQL packet exceeds bound");
    let mut packet = vec![0; 4 + length];
    packet[..4].copy_from_slice(&header);
    reader.read_exact(&mut packet[4..]).await?;
    Ok(Some(packet))
}
