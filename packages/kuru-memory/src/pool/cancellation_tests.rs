use super::*;
use anyhow::{Context, ensure};
use futures::FutureExt;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};

use crate as kuru_memory;
use crate::{store::MemoryStore, store::QUERY_TIMEOUT};

/// One fixture-owned connection, relayed without decoding or logging any
/// protocol bytes. Arming after BEGIN holds the next real query response.
struct ResponseGate {
    port: u16,
    armed: Arc<std::sync::atomic::AtomicBool>,
    reached: oneshot::Receiver<()>,
    release: Option<oneshot::Sender<()>>,
    eof: oneshot::Receiver<()>,
    task: Option<JoinHandle<Result<()>>>,
}

impl ResponseGate {
    async fn start(options: &MySqlConnectOptions) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let upstream = (options.get_host().to_owned(), options.get_port());
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let arm = armed.clone();
        let (reached_tx, reached) = oneshot::channel();
        let (release, release_rx) = oneshot::channel();
        let (eof_tx, eof) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut client, _) = listener.accept().await?;
            let mut server = TcpStream::connect(upstream).await?;
            let (mut client_read, mut client_write) = client.split();
            let (mut server_read, mut server_write) = server.split();
            let from_client = async {
                let mut bytes = [0; 8192];
                loop {
                    let count = client_read.read(&mut bytes).await?;
                    if count == 0 {
                        return Ok::<_, anyhow::Error>(());
                    }
                    server_write.write_all(&bytes[..count]).await?;
                }
            };
            let from_server = async {
                let mut bytes = [0; 8192];
                let mut reached_tx = Some(reached_tx);
                let mut release_rx = Some(release_rx);
                loop {
                    let count = server_read.read(&mut bytes).await?;
                    if count == 0 {
                        return Ok::<_, anyhow::Error>(());
                    }
                    if arm.load(Ordering::SeqCst)
                        && let Some(reached) = reached_tx.take()
                    {
                        let _ = reached.send(());
                        // Dropping the fixture's sender also releases the gate.
                        let _ = release_rx.take().expect("one response gate").await;
                    }
                    client_write.write_all(&bytes[..count]).await?;
                }
            };
            tokio::select! {
                result = from_client => {
                    result?;
                    let _ = eof_tx.send(());
                }
                result = from_server => { result?; }
            }
            Ok(())
        });
        Ok(Self {
            port,
            armed,
            reached,
            release: Some(release),
            eof,
            task: Some(task),
        })
    }

    async fn finish(&mut self) -> Result<()> {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        let mut task = self.task.take().expect("relay is owned until joined");
        match timeout(QUERY_TIMEOUT, &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                // This await already joined the task, including a panic or
                // cancellation. A completed JoinHandle cannot be awaited twice.
                task.abort();
                Err(error).context("relay task failed")
            }
            Err(error) => {
                task.abort();
                let _ = task.await;
                Err(error).context("relay did not finish after pool close")
            }
        }
    }
}

impl Drop for ResponseGate {
    fn drop(&mut self) {
        // finish owns awaited cleanup; abort is a fallback on unexpected unwind.
        self.release.take();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn fixture_pool(
    options: &MySqlConnectOptions,
    gate: &ResponseGate,
) -> (MemoryPool, oneshot::Receiver<()>) {
    let (returned, release_started) = oneshot::channel();
    let returned = Arc::new(StdMutex::new(Some(returned)));
    let armed = gate.armed.clone();
    let pool = MySqlPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(QUERY_TIMEOUT)
        .after_release(move |_, _| {
            if armed.load(Ordering::SeqCst)
                && let Some(returned) = returned.lock().expect("return signal").take()
            {
                // No await between this event and SQLx's pending-response ping.
                let _ = returned.send(());
            }
            Box::pin(async { Ok(true) })
        })
        .connect_lazy_with(options.clone().host("127.0.0.1").port(gate.port));
    (MemoryPool::fixture(pool, "main"), release_started)
}

async fn hold_query(connection: &mut MySqlConnection, gate: &mut ResponseGate) -> Result<()> {
    gate.armed.store(true, Ordering::SeqCst);
    let mut query = Box::pin(connection.fetch_one("SELECT 1"));
    timeout(QUERY_TIMEOUT, async {
        tokio::select! {
            result = query.as_mut() => {
                result?;
                anyhow::bail!("query completed without its held native response");
            }
            reached = &mut gate.reached => {
                reached.context("relay lost the response gate")?;
                Ok(())
            }
        }
    })
    .await
    .context("real query never reached its response gate")??;
    // Cancellation leaves the real response queued at the owned relay.
    drop(query);
    Ok(())
}

async fn session_absent(main: &MemoryPool, id: u64) -> Result<()> {
    timeout(QUERY_TIMEOUT, async {
        loop {
            let active: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ?",
            )
            .bind(id)
            .fetch_one(main)
            .await?;
            if active == 0 {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("original server session still exists after client disposal")?
}

async fn canceled_query(preclose: bool) -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let main = store.server_for_teardown().pool("main").await?;
    let options = main.connect_options();
    let mut gate = ResponseGate::start(&options).await?;
    let (pool, mut release_started) = fixture_pool(&options, &gate).await;
    let observation: Result<()> = async {
        let mut transaction = pool.begin().await?;
        let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
            .fetch_one(&mut *transaction)
            .await?;
        ensure!(pool.size() == 1 && pool.num_idle() == 0);
        ensure!(pool.checked_out.load(Ordering::SeqCst) == 1);
        ensure!(pool.options().get_max_connections() == 1);
        ensure!(pool.options().get_min_connections() == 0);
        hold_query(&mut transaction, &mut gate).await?;
        drop(transaction);

        // Pre-close builds the future before any yield, taking SQLx's closed-pool
        // direct-close path. The regression instead proves disposal began open.
        let mut closing = preclose.then(|| Box::pin(pool.close()));
        let event = timeout(QUERY_TIMEOUT, async {
            tokio::select! {
                started = &mut release_started, if !preclose => {
                    started.context("release control disconnected")?;
                    Ok::<_, anyhow::Error>("started_return")
                }
                eof = &mut gate.eof => {
                    eof.context("relay ended without client EOF")?;
                    Ok("client_eof")
                }
            }
        })
        .await
        .context("canceled session produced no causal disposal event")??;
        let closing = closing.get_or_insert_with(|| Box::pin(pool.close()));
        let ready = closing.as_mut().now_or_never().is_some();
        eprintln!(
            "canceled_pool preclose={preclose} event={event} close_ready={ready} size={} idle={} checked_out={}",
            pool.size(), pool.num_idle(), pool.checked_out.load(Ordering::SeqCst)
        );
        ensure!(ready, "pool close is pending after {event} while the canceled query response remains held");
        // This is server evidence, separately from client permit accounting.
        session_absent(&main, id).await?;
        Ok(())
    }
    .await;

    // The old-path failure is data until every owned resource is cleaned up.
    if let Some(release) = gate.release.take() {
        let _ = release.send(());
    }
    let closed = timeout(QUERY_TIMEOUT, pool.close()).await;
    let relayed = gate.finish().await;
    let store_closed = store.close().await;
    closed.context("fixture pool cleanup did not finish")?;
    relayed?;
    store_closed?;
    observation
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_query_disposal_does_not_wait_for_its_response() -> Result<()> {
    kuru_memory::test_support::closing(async { canceled_query(false).await }).await
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_query_preclose_control_skips_the_started_return_race() -> Result<()> {
    kuru_memory::test_support::closing(async { canceled_query(true).await }).await
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_explicit_release_discards_its_floating_connection() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let store = MemoryStore::temporary().await?;
        let main = store.server_for_teardown().pool("main").await?;
        let mut gate = ResponseGate::start(&main.connect_options()).await?;
        let (pool, mut release_started) = fixture_pool(&main.connect_options(), &gate).await;
        let observation: Result<()> = async {
            let mut session = pool.acquire().await?;
            let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut *session)
                .await?;
            hold_query(&mut session, &mut gate).await?;
            let mut returning = Box::pin(session.release());
            timeout(QUERY_TIMEOUT, async {
                tokio::select! {
                    () = returning.as_mut() => anyhow::bail!("release completed through a held response"),
                    started = &mut release_started => started.context("release control disconnected"),
                }
            })
            .await
            .context("explicit release never reached its pending response")??;
            ensure!(pool.size() == 1 && pool.num_idle() == 0);
            ensure!(pool.checked_out.load(Ordering::SeqCst) == 0);
            // The floating release still owns the only client permit. This
            // assertion deliberately makes no server-side capacity claim.
            let mut another = Box::pin(pool.acquire());
            ensure!(futures::poll!(another.as_mut()).is_pending());
            drop(another);
            drop(returning);
            timeout(QUERY_TIMEOUT, &mut gate.eof)
                .await
                .context("canceled explicit release did not close its socket")??;
            ensure!(pool.close().now_or_never().is_some());
            session_absent(&main, id).await?;
            Ok(())
        }
        .await;
        if let Some(release) = gate.release.take() {
            let _ = release.send(());
        }
        let closed = timeout(QUERY_TIMEOUT, pool.close()).await;
        let relayed = gate.finish().await;
        let store_closed = store.close().await;
        closed.context("fixture pool cleanup did not finish")?;
        relayed?;
        store_closed?;
        observation
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
async fn successful_release_commit_and_rollback_reuse_the_idle_session() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let store = MemoryStore::temporary().await?;
        let main = store.server_for_teardown().pool("main").await?;
        let pool = MemoryPool::fixture(
            MySqlPoolOptions::new()
                .max_connections(1)
                .min_connections(0)
                .acquire_timeout(QUERY_TIMEOUT)
                .connect_lazy_with((*main.connect_options()).clone()),
            "main",
        );
        let observation: Result<()> = async {
            let mut session = pool.acquire().await?;
            let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut *session)
                .await?;
            session.release().await;
            ensure!(pool.size() == 1 && pool.num_idle() == 1);
            for commit in [true, false] {
                let mut transaction = pool.begin().await?;
                let next: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                    .fetch_one(&mut *transaction)
                    .await?;
                ensure!(next == id, "successful work opened a replacement session");
                if commit {
                    transaction.commit().await?;
                } else {
                    transaction.rollback().await?;
                }
                ensure!(pool.size() == 1 && pool.num_idle() == 1);
                ensure!(pool.checked_out.load(Ordering::SeqCst) == 0);
            }
            let next: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&pool)
                .await?;
            ensure!(next == id && pool.num_idle() == 1);
            Ok(())
        }
        .await;
        let closed = timeout(QUERY_TIMEOUT, pool.close()).await;
        let store_closed = store.close().await;
        closed.context("successful fixture pool cleanup did not finish")?;
        store_closed?;
        observation
    })
    .await
}
