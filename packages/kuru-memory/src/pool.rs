//! The one funnel for every memory pool acquisition.
//!
//! A new pooled connection runs TCP, MySQL authentication and the identity
//! callback inside the pool's acquire window, so sequential work must reuse
//! an idle session instead of opening another. SQLx returns a dropped
//! connection through a spawned task that pings first; on a `current_thread`
//! runtime the same task's next statement reaches the idle queue before that
//! task runs and opens a new connection. Every statement run through
//! [`MemoryPool`] instead returns its connection inline, before the
//! statement's future completes.
//!
//! An acquisition SQLx times out is reported as a typed
//! [`PoolAcquireTimedOut`] naming what it waited for. There is no `Deref` to
//! the SQLx pool, so nothing acquires around this funnel.

use std::{
    fmt, io,
    ops::{Deref, DerefMut},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use futures::{TryStreamExt, future::BoxFuture, stream::BoxStream};
use sqlx::{
    Database, Either, Execute, Executor, MySql, MySqlConnection, MySqlPool, SqlStr,
    mysql::{MySqlQueryResult, MySqlRow, MySqlStatement, MySqlTypeInfo},
    pool::{PoolConnection, PoolOptions},
};
use sqlx_core::transaction::TransactionManager;
use tokio::time::Instant;

use crate::server::ConnectionObservation;

/// A memory branch pool. Cloning shares the same SQLx pool and observation.
#[derive(Clone)]
pub struct MemoryPool {
    pool: MySqlPool,
    branch: Arc<str>,
    observation: ConnectionObservation,
    checked_out: Arc<AtomicU32>,
}

impl fmt::Debug for MemoryPool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryPool")
            .field("branch", &self.branch)
            .field("size", &self.pool.size())
            .field("idle", &self.pool.num_idle())
            .finish_non_exhaustive()
    }
}

impl MemoryPool {
    pub(crate) fn new(pool: MySqlPool, branch: &str, observation: ConnectionObservation) -> Self {
        Self {
            pool,
            branch: branch.into(),
            observation,
            checked_out: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Wrap a fixture's own pool. It authenticates without the store's
    /// identity callback, so its observation counts nothing.
    #[cfg(test)]
    pub(crate) fn fixture(pool: MySqlPool, branch: &str) -> Self {
        Self::new(pool, branch, ConnectionObservation::detached())
    }

    /// Acquire one session. A timed-out acquisition carries a
    /// [`PoolAcquireTimedOut`] as context over SQLx's `PoolTimedOut`.
    pub async fn acquire(&self) -> Result<PooledSession> {
        self.acquire_session()
            .await
            .map_err(|failure| match failure {
                AcquireFailure::TimedOut(diagnostic) => {
                    anyhow::Error::from(sqlx::Error::PoolTimedOut).context(diagnostic)
                }
                AcquireFailure::Sqlx(error) => error.into(),
            })
    }

    /// Begin a read transaction on one session; commit or rollback returns the
    /// session inline.
    pub async fn begin(&self) -> Result<MemoryTransaction> {
        let mut session = self.acquire().await?;
        <MySql as Database>::TransactionManager::begin(&mut *session, None).await?;
        Ok(MemoryTransaction {
            session: Some(session),
        })
    }

    /// Mark the pool closed and wait for every checked-out session.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub fn is_closed(&self) -> bool {
        self.pool.is_closed()
    }

    pub fn options(&self) -> &PoolOptions<MySql> {
        self.pool.options()
    }

    pub fn size(&self) -> u32 {
        self.pool.size()
    }

    pub fn num_idle(&self) -> usize {
        self.pool.num_idle()
    }

    #[cfg(test)]
    pub(crate) fn connect_options(&self) -> Arc<sqlx::mysql::MySqlConnectOptions> {
        self.pool.connect_options()
    }

    /// New connections that entered this pool's authentication callback.
    #[cfg(test)]
    pub(crate) fn authenticated(&self) -> u64 {
        self.observation.authenticated()
    }

    #[cfg(test)]
    pub(crate) fn observation(&self) -> &ConnectionObservation {
        &self.observation
    }

    async fn acquire_session(&self) -> std::result::Result<PooledSession, AcquireFailure> {
        let started = Instant::now();
        let authenticated_before = self.observation.authenticated();
        match self.pool.acquire().await {
            Ok(connection) => {
                self.checked_out.fetch_add(1, Ordering::SeqCst);
                Ok(PooledSession {
                    connection: Some(connection),
                    checked_out: self.checked_out.clone(),
                })
            }
            Err(sqlx::Error::PoolTimedOut) => {
                let authenticated_total = self.observation.authenticated();
                let authenticated_during_wait =
                    authenticated_total.saturating_sub(authenticated_before);
                let max = self.pool.options().get_max_connections();
                let checked_out = self.checked_out.load(Ordering::SeqCst);
                let diagnostic = PoolAcquireTimedOut {
                    branch: self.branch.to_string(),
                    wait: PoolWait::classify(max, checked_out, authenticated_during_wait),
                    max,
                    size: self.pool.size(),
                    idle: self.pool.num_idle(),
                    checked_out,
                    waited: started.elapsed(),
                    window: self.pool.options().get_acquire_timeout(),
                    authenticated_total,
                    authenticated_during_wait,
                    phase: (authenticated_during_wait > 0).then(|| self.observation.latest_phase()),
                };
                tracing::warn!(
                    branch = %diagnostic.branch,
                    wait = diagnostic.wait.label(),
                    max = diagnostic.max,
                    size = diagnostic.size,
                    idle = diagnostic.idle,
                    checked_out = diagnostic.checked_out,
                    waited_ms = u64::try_from(diagnostic.waited.as_millis()).unwrap_or(u64::MAX),
                    authenticated_total = diagnostic.authenticated_total,
                    authenticated_during_wait = diagnostic.authenticated_during_wait,
                    phase = diagnostic.phase.unwrap_or("none"),
                    "memory pool acquire timed out"
                );
                Err(AcquireFailure::TimedOut(diagnostic))
            }
            Err(error) => Err(AcquireFailure::Sqlx(error)),
        }
    }

    /// The `Executor` form: a timeout travels as `sqlx::Error::Io` of kind
    /// `Other` carrying the typed diagnostic, because an `Executor` must
    /// return `sqlx::Error` and `PoolTimedOut` is a unit variant.
    async fn statement_session(&self) -> std::result::Result<PooledSession, sqlx::Error> {
        self.acquire_session()
            .await
            .map_err(|failure| match failure {
                AcquireFailure::TimedOut(diagnostic) => {
                    sqlx::Error::Io(io::Error::other(diagnostic))
                }
                AcquireFailure::Sqlx(error) => error,
            })
    }
}

enum AcquireFailure {
    TimedOut(PoolAcquireTimedOut),
    Sqlx(sqlx::Error),
}

/// One checked-out pool session.
pub struct PooledSession {
    connection: Option<PoolConnection<MySql>>,
    checked_out: Arc<AtomicU32>,
}

impl PooledSession {
    /// Return the session to its pool now: SQLx's release check runs inline,
    /// so the connection is idle again before the caller's next statement.
    /// The caller must have left the session as it found it (no open
    /// transaction or session state).
    pub async fn release(mut self) {
        if let Some(mut connection) = self.connection.take() {
            connection.return_to_pool().await;
            self.checked_out.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Remove the session from its pool. Dropping the returned connection
    /// ends the SQL session; the pool opens a replacement when needed.
    pub fn detach(mut self) -> MySqlConnection {
        let connection = self
            .connection
            .take()
            .expect("pooled session is checked out until consumed");
        self.checked_out.fetch_sub(1, Ordering::SeqCst);
        connection.detach()
    }
}

impl Drop for PooledSession {
    /// Without an explicit release or close (an error or cancellation path),
    /// SQLx's own spawned release returns or closes the connection.
    fn drop(&mut self) {
        if self.connection.take().is_some() {
            self.checked_out.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Deref for PooledSession {
    type Target = MySqlConnection;

    fn deref(&self) -> &MySqlConnection {
        self.connection
            .as_deref()
            .expect("pooled session is checked out until consumed")
    }
}

impl DerefMut for PooledSession {
    fn deref_mut(&mut self) -> &mut MySqlConnection {
        self.connection
            .as_deref_mut()
            .expect("pooled session is checked out until consumed")
    }
}

/// A transaction on one pooled session. Commit and rollback release the
/// session inline; dropping it queues SQLx's rollback, which the spawned
/// release flushes before the connection is idle again.
pub struct MemoryTransaction {
    session: Option<PooledSession>,
}

impl MemoryTransaction {
    pub async fn commit(mut self) -> Result<()> {
        let mut session = self.session.take().expect("open memory transaction");
        <MySql as Database>::TransactionManager::commit(&mut *session).await?;
        session.release().await;
        Ok(())
    }

    pub async fn rollback(mut self) -> Result<()> {
        let mut session = self.session.take().expect("open memory transaction");
        <MySql as Database>::TransactionManager::rollback(&mut *session).await?;
        session.release().await;
        Ok(())
    }
}

impl Drop for MemoryTransaction {
    fn drop(&mut self) {
        if let Some(mut session) = self.session.take() {
            <MySql as Database>::TransactionManager::start_rollback(&mut *session);
        }
    }
}

impl Deref for MemoryTransaction {
    type Target = MySqlConnection;

    fn deref(&self) -> &MySqlConnection {
        self.session.as_deref().expect("open memory transaction")
    }
}

impl DerefMut for MemoryTransaction {
    fn deref_mut(&mut self) -> &mut MySqlConnection {
        self.session
            .as_deref_mut()
            .expect("open memory transaction")
    }
}

/// Mirrors SQLx's `impl Executor for &Pool`, through the funnel and with an
/// inline release after each statement.
impl<'p> Executor<'p> for &'_ MemoryPool {
    type Database = MySql;

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxStream<'e, std::result::Result<Either<MySqlQueryResult, MySqlRow>, sqlx::Error>>
    where
        E: 'q + Execute<'q, MySql>,
    {
        let pool = self.clone();
        Box::pin(sqlx_core::try_stream! {
            let mut session = pool.statement_session().await?;
            let mut stream = (&mut *session).fetch_many(query);
            while let Some(value) = stream.try_next().await? {
                r#yield!(value);
            }
            drop(stream);
            session.release().await;
            Ok(())
        })
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxFuture<'e, std::result::Result<Option<MySqlRow>, sqlx::Error>>
    where
        E: 'q + Execute<'q, MySql>,
    {
        let pool = self.clone();
        Box::pin(async move {
            let mut session = pool.statement_session().await?;
            let row = (&mut *session).fetch_optional(query).await?;
            session.release().await;
            Ok(row)
        })
    }

    fn prepare_with<'e>(
        self,
        sql: SqlStr,
        parameters: &'e [MySqlTypeInfo],
    ) -> BoxFuture<'e, std::result::Result<MySqlStatement, sqlx::Error>>
    where
        'p: 'e,
    {
        let pool = self.clone();
        Box::pin(async move {
            let mut session = pool.statement_session().await?;
            let statement = (&mut *session).prepare_with(sql, parameters).await?;
            session.release().await;
            Ok(statement)
        })
    }
}

/// What a timed-out acquisition was waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolWait {
    /// Every permit was held by checked-out Kuru work: contention, or a
    /// holder that did not finish.
    HeldConnections,
    /// Capacity existed and a new connection entered authentication during
    /// this wait: TCP, MySQL authentication or Kuru's identity callback.
    NewConnection,
    /// Capacity existed and no new connection entered authentication during
    /// this wait: an idle-connection check, a release still in flight, or a
    /// connect that never reached the identity callback.
    IdleCheckOrRelease,
}

impl PoolWait {
    pub(crate) fn classify(max: u32, checked_out: u32, authenticated_during_wait: u64) -> Self {
        if checked_out >= max {
            Self::HeldConnections
        } else if authenticated_during_wait > 0 {
            Self::NewConnection
        } else {
            Self::IdleCheckOrRelease
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::HeldConnections => "held connections",
            Self::NewConnection => "new connection",
            Self::IdleCheckOrRelease => "idle check or release",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::HeldConnections => "for a connection held by Kuru work (every permit held)",
            Self::NewConnection => "for a new connection's authentication",
            Self::IdleCheckOrRelease => {
                "for an idle-connection check or a release, with no new connection authenticating"
            }
        }
    }
}

/// A bounded, secret-free report of a timed-out pool acquisition: no SQL
/// text, credentials, endpoints or paths.
#[derive(Clone, Debug)]
pub struct PoolAcquireTimedOut {
    pub branch: String,
    pub wait: PoolWait,
    pub max: u32,
    pub size: u32,
    pub idle: usize,
    pub checked_out: u32,
    pub waited: Duration,
    pub window: Duration,
    pub authenticated_total: u64,
    pub authenticated_during_wait: u64,
    /// The latest authentication phase, only when a connection entered
    /// authentication during this wait; otherwise it would be stale.
    pub phase: Option<&'static str>,
}

impl fmt::Display for PoolAcquireTimedOut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "memory pool acquire on kuru/{} timed out after {:.3} s (window {:.3} s) waiting {}",
            self.branch,
            self.waited.as_secs_f64(),
            self.window.as_secs_f64(),
            self.wait.describe(),
        )?;
        if let Some(phase) = self.phase {
            write!(formatter, " (connection phase: {phase})")?;
        }
        write!(
            formatter,
            "; pool size {} of {}, {} idle, {} checked out; {} connections authenticated since the pool opened, {} during this wait",
            self.size,
            self.max,
            self.idle,
            self.checked_out,
            self.authenticated_total,
            self.authenticated_during_wait,
        )
    }
}

impl std::error::Error for PoolAcquireTimedOut {}

/// The typed pool timeout anywhere in `error`'s chain: anyhow context from
/// [`MemoryPool::acquire`], or the `Executor` carrier.
pub fn pool_acquire_timeout(error: &anyhow::Error) -> Option<&PoolAcquireTimedOut> {
    // anyhow finds a context value at any depth only through its own
    // downcast; `chain()` yields the context wrappers instead.
    if let Some(diagnostic) = error.downcast_ref::<PoolAcquireTimedOut>() {
        return Some(diagnostic);
    }
    error.chain().find_map(|cause| {
        if let Some(diagnostic) = cause.downcast_ref::<PoolAcquireTimedOut>() {
            return Some(diagnostic);
        }
        let io = match cause.downcast_ref::<sqlx::Error>() {
            Some(sqlx::Error::Io(io)) => io,
            _ => cause.downcast_ref::<io::Error>()?,
        };
        io.get_ref()?.downcast_ref::<PoolAcquireTimedOut>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(wait: PoolWait, phase: Option<&'static str>) -> PoolAcquireTimedOut {
        PoolAcquireTimedOut {
            branch: "main".into(),
            wait,
            max: 4,
            size: 3,
            idle: 0,
            checked_out: 2,
            waited: Duration::from_millis(2004),
            window: Duration::from_secs(2),
            authenticated_total: 7,
            authenticated_during_wait: u64::from(phase.is_some()),
            phase,
        }
    }

    #[test]
    fn waits_are_classified_by_permits_and_authentications_during_the_wait() {
        assert_eq!(PoolWait::classify(4, 4, 0), PoolWait::HeldConnections);
        assert_eq!(PoolWait::classify(4, 4, 1), PoolWait::HeldConnections);
        assert_eq!(PoolWait::classify(4, 3, 1), PoolWait::NewConnection);
        assert_eq!(PoolWait::classify(4, 0, 0), PoolWait::IdleCheckOrRelease);
        assert_eq!(PoolWait::classify(4, 3, 0), PoolWait::IdleCheckOrRelease);
    }

    #[test]
    fn diagnostic_names_its_wait_and_omits_a_stale_phase() {
        let idle = diagnostic(PoolWait::IdleCheckOrRelease, None).to_string();
        assert!(
            idle.contains("idle-connection check or a release"),
            "{idle}"
        );
        assert!(!idle.contains("connection phase"), "{idle}");
        assert!(
            idle.contains("timed out after 2.004 s (window 2.000 s)"),
            "{idle}"
        );
        assert!(
            idle.contains("pool size 3 of 4, 0 idle, 2 checked out; 7 connections authenticated since the pool opened, 0 during this wait"),
            "{idle}"
        );
        let new = diagnostic(PoolWait::NewConnection, Some("data directory query")).to_string();
        assert!(
            new.contains(
                "for a new connection's authentication (connection phase: data directory query)"
            ),
            "{new}"
        );
        assert!(
            diagnostic(PoolWait::HeldConnections, None)
                .to_string()
                .contains("every permit held")
        );
    }

    /// The `Executor` carrier is found by the helper, is not an I/O kind any
    /// Kuru classifier matches (`TimedOut`, `ConnectionReset`), and prints the
    /// typed diagnostic.
    #[test]
    fn executor_carrier_is_typed_and_not_a_classified_io_kind() {
        let error = sqlx::Error::Io(io::Error::other(diagnostic(
            PoolWait::NewConnection,
            Some("x"),
        )));
        let sqlx::Error::Io(io) = &error else {
            unreachable!()
        };
        assert_eq!(io.kind(), io::ErrorKind::Other);
        assert!(
            error
                .to_string()
                .contains("memory pool acquire on kuru/main timed out")
        );
        let chained = anyhow::Error::from(error).context("memory read deadline exceeded");
        let found = pool_acquire_timeout(&chained).expect("typed carrier");
        assert_eq!(found.wait, PoolWait::NewConnection);
        let contextual = anyhow::Error::from(sqlx::Error::PoolTimedOut)
            .context(diagnostic(PoolWait::HeldConnections, None));
        assert_eq!(
            pool_acquire_timeout(&contextual).map(|found| found.wait),
            Some(PoolWait::HeldConnections)
        );
        assert!(contextual.chain().any(|cause| matches!(
            cause.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        )));
        assert!(pool_acquire_timeout(&anyhow::Error::from(sqlx::Error::PoolTimedOut)).is_none());
    }
}
