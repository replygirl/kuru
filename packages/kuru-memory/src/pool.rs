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
//! An acquisition that reaches its bound is reported as a typed
//! [`PoolAcquireTimedOut`] naming what it waited for and which bound ended
//! the wait. There is no `Deref` to the SQLx pool, so nothing acquires around
//! this funnel.
//!
//! A memory statement or operation budget runs its work in a budget scope,
//! [`within`] or [`within_until`], instead of `tokio::time::timeout`. The
//! funnel registers its in-flight acquisition with the enclosing scope, so a
//! budget that expires while the acquisition waits is reported as that
//! acquisition's [`PoolAcquireTimedOut`] (bound: the statement budget), and one
//! that expires during execution as a [`BudgetElapsed`] with no acquisition
//! diagnostic.

use std::{
    fmt,
    future::Future,
    io,
    ops::{Deref, DerefMut},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
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
use tokio::time::{Instant, sleep_until};

use crate::server::ConnectionObservation;

/// A pending acquisition this long leaves one diagnostics record, and one
/// that completes later leaves another. It only logs: it never decides an
/// acquisition's outcome.
pub const SLOW_ACQUIRE_THRESHOLD: Duration = Duration::from_secs(2);

tokio::task_local! {
    static BUDGET_SCOPE: BudgetScope;
}

/// The innermost budget scope of the current task: the earliest deadline of
/// it and every enclosing scope, that deadline's budget, and the in-flight
/// acquisition slot every nested scope shares.
#[derive(Clone)]
struct BudgetScope {
    deadline: Instant,
    budget: Duration,
    slot: Arc<InFlightSlot>,
}

/// The acquisition a budget scope's work is waiting on, if any. Each
/// registration carries its own token, so a concurrent acquisition in the
/// same scope never clears another's entry.
#[derive(Default)]
struct InFlightSlot {
    next: AtomicU64,
    current: StdMutex<Option<(u64, InFlightAcquire)>>,
}

struct InFlightAcquire {
    pool: MemoryPool,
    started: Instant,
    authenticated_before: u64,
    budget: Duration,
    remaining: Duration,
}

impl InFlightSlot {
    /// The scope's expiry, built while its work (and so any pending
    /// acquisition's registration) is still alive.
    fn elapsed(&self, budget: Duration) -> BudgetElapsed {
        let acquire = self
            .current
            .lock()
            .ok()
            .and_then(|current| {
                current.as_ref().map(|(_, acquire)| {
                    acquire.pool.diagnose(
                        acquire.started,
                        acquire.authenticated_before,
                        AcquireBound::StatementBudget,
                        acquire.budget,
                        acquire.remaining,
                    )
                })
            })
            .inspect(PoolAcquireTimedOut::report)
            .map(Box::new);
        BudgetElapsed { budget, acquire }
    }
}

/// Run memory statement or operation work under `budget`, as
/// `tokio::time::timeout` does, in a budget scope the pool funnel reads: an
/// acquisition inside it is bounded by what remains of the earliest enclosing
/// budget, and an expiry during that acquisition is reported as its typed
/// [`PoolAcquireTimedOut`].
pub async fn within<F: Future>(budget: Duration, work: F) -> Result<F::Output, BudgetElapsed> {
    scoped(
        Instant::now() + budget,
        budget,
        work,
        std::future::pending(),
    )
    .await
}

/// [`within`] for work whose budget is a deadline, as
/// `tokio::time::timeout_at` does.
pub async fn within_until<F: Future>(
    deadline: Instant,
    work: F,
) -> Result<F::Output, BudgetElapsed> {
    let budget = deadline.saturating_duration_since(Instant::now());
    scoped(deadline, budget, work, std::future::pending()).await
}

/// [`within`] whose scope also ends, through the same expiry path, as soon as
/// `expire` completes: an event standing in for the deadline, never a bound.
#[cfg(test)]
pub(crate) async fn within_or<F: Future>(
    budget: Duration,
    work: F,
    expire: impl Future<Output = ()>,
) -> Result<F::Output, BudgetElapsed> {
    scoped(Instant::now() + budget, budget, work, expire).await
}

async fn scoped<F: Future>(
    deadline: Instant,
    budget: Duration,
    work: F,
    expire: impl Future<Output = ()>,
) -> Result<F::Output, BudgetElapsed> {
    let scope = match BUDGET_SCOPE.try_with(BudgetScope::clone) {
        Ok(enclosing) if enclosing.deadline <= deadline => enclosing,
        Ok(enclosing) => BudgetScope {
            deadline,
            budget,
            slot: enclosing.slot,
        },
        Err(_) => BudgetScope {
            deadline,
            budget,
            slot: Arc::default(),
        },
    };
    let slot = scope.slot.clone();
    let (deadline, budget) = (scope.deadline, scope.budget);
    // The work is polled first, as `tokio::time::timeout` polls its future
    // before its delay.
    let expired = async {
        tokio::select! {
            biased;
            () = sleep_until(deadline) => {}
            () = expire => {}
        }
        slot.elapsed(budget)
    };
    tokio::select! {
        biased;
        output = BUDGET_SCOPE.scope(scope, work) => Ok(output),
        elapsed = expired => Err(elapsed),
    }
}

/// A memory statement or operation budget that elapsed. When it elapsed
/// while an acquisition was waiting, its source is that acquisition's
/// [`PoolAcquireTimedOut`]; otherwise it elapsed during execution and is no
/// acquisition timeout.
#[derive(Debug)]
pub struct BudgetElapsed {
    pub budget: Duration,
    pub acquire: Option<Box<PoolAcquireTimedOut>>,
}

impl fmt::Display for BudgetElapsed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "memory statement budget of {:.3} s elapsed",
            self.budget.as_secs_f64()
        )?;
        if self.acquire.is_some() {
            write!(formatter, " while acquiring a pool session")?;
        }
        Ok(())
    }
}

impl std::error::Error for BudgetElapsed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.acquire
            .as_ref()
            .map(|acquire| acquire.as_ref() as &(dyn std::error::Error + 'static))
    }
}

/// One acquisition counted as pending on its pool and, inside a budget scope,
/// registered in that scope's slot. Dropping it (on completion or
/// cancellation, including while queued) clears both.
struct PendingAcquire {
    observation: ConnectionObservation,
    registration: Option<(Arc<InFlightSlot>, u64)>,
}

impl PendingAcquire {
    fn register(
        pool: &MemoryPool,
        scope: Option<&BudgetScope>,
        started: Instant,
        authenticated_before: u64,
    ) -> Self {
        pool.observation.acquire_started();
        let registration = scope.map(|scope| {
            let token = scope.slot.next.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut current) = scope.slot.current.lock() {
                *current = Some((
                    token,
                    InFlightAcquire {
                        pool: pool.clone(),
                        started,
                        authenticated_before,
                        budget: scope.budget,
                        remaining: scope.deadline.saturating_duration_since(started),
                    },
                ));
            }
            (scope.slot.clone(), token)
        });
        Self {
            observation: pool.observation.clone(),
            registration,
        }
    }
}

impl Drop for PendingAcquire {
    fn drop(&mut self) {
        if let Some((slot, token)) = self.registration.take()
            && let Ok(mut current) = slot.current.lock()
            && current.as_ref().is_some_and(|(owner, _)| *owner == token)
        {
            current.take();
        }
        self.observation.acquire_ended();
    }
}

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

    /// Acquire one session. An acquisition that reaches the pool ceiling
    /// carries a [`PoolAcquireTimedOut`] as context over SQLx's
    /// `PoolTimedOut`; one ended by its budget scope is that scope's
    /// [`BudgetElapsed`].
    pub async fn acquire(&self) -> Result<PooledSession> {
        self.acquire_session()
            .await
            .map_err(|failure| match failure {
                AcquireFailure::TimedOut(diagnostic) => {
                    anyhow::Error::from(sqlx::Error::PoolTimedOut).context(*diagnostic)
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

    /// Mark the pool closed at once, then return the wait for every
    /// checked-out session, as SQLx's `Pool::close` does: callers build every
    /// close future before awaiting any, so no sibling pool keeps admitting.
    pub fn close(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.pool.close()
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

    /// Sessions this funnel counts as held by Kuru work.
    #[cfg(test)]
    pub(crate) fn checked_out(&self) -> u32 {
        self.checked_out.load(Ordering::SeqCst)
    }

    /// Acquisitions of this pool still waiting.
    #[cfg(test)]
    pub(crate) fn pending_acquires(&self) -> u64 {
        self.observation.pending_acquires()
    }

    async fn acquire_session(&self) -> std::result::Result<PooledSession, AcquireFailure> {
        // An authored identity rejection is sticky for the pool's life: the
        // same endpoint cannot answer differently, so not even an idle
        // session is handed out.
        if let Some(rejection) = self.observation.identity_rejection() {
            return Err(AcquireFailure::Sqlx(rejection));
        }
        let started = Instant::now();
        let authenticated_before = self.observation.authenticated();
        let scope = BUDGET_SCOPE.try_with(BudgetScope::clone).ok();
        let _pending =
            PendingAcquire::register(self, scope.as_ref(), started, authenticated_before);
        let acquiring = async {
            let acquiring = self.pool.acquire();
            tokio::pin!(acquiring);
            tokio::select! {
                biased;
                acquired = &mut acquiring => (acquired, false),
                () = sleep_until(started + SLOW_ACQUIRE_THRESHOLD) => {
                    self.still_waiting(started, authenticated_before, scope.as_ref());
                    (acquiring.await, true)
                }
            }
        };
        // The rejection is polled first, so one that lands with an idle
        // session on the same poll still ends the acquisition with its cause.
        let (acquired, slow) = tokio::select! {
            biased;
            rejection = self.observation.identity_rejected() => {
                return Err(AcquireFailure::Sqlx(rejection));
            }
            acquired = acquiring => acquired,
        };
        match acquired {
            Ok(connection) => {
                if slow {
                    tracing::warn!(
                        branch = %self.branch,
                        waited_ms = millis(started.elapsed()),
                        "memory pool acquire completed slowly"
                    );
                }
                self.checked_out.fetch_add(1, Ordering::SeqCst);
                Ok(PooledSession {
                    connection: Some(connection),
                    checked_out: self.checked_out.clone(),
                })
            }
            Err(sqlx::Error::PoolTimedOut) => {
                let ceiling = self.pool.options().get_acquire_timeout();
                let diagnostic = self.diagnose(
                    started,
                    authenticated_before,
                    AcquireBound::PoolCeiling,
                    ceiling,
                    ceiling,
                );
                diagnostic.report();
                Err(AcquireFailure::TimedOut(Box::new(diagnostic)))
            }
            Err(error) => Err(AcquireFailure::Sqlx(error)),
        }
    }

    /// What one acquisition, started at `started`, is waiting for now.
    fn diagnose(
        &self,
        started: Instant,
        authenticated_before: u64,
        bound: AcquireBound,
        budget: Duration,
        window: Duration,
    ) -> PoolAcquireTimedOut {
        let authenticated_total = self.observation.authenticated();
        let authenticated_during_wait = authenticated_total.saturating_sub(authenticated_before);
        let max = self.pool.options().get_max_connections();
        let checked_out = self.checked_out.load(Ordering::SeqCst);
        PoolAcquireTimedOut {
            branch: self.branch.to_string(),
            wait: PoolWait::classify(max, checked_out, authenticated_during_wait),
            bound,
            budget,
            max,
            size: self.pool.size(),
            idle: self.pool.num_idle(),
            checked_out,
            waited: started.elapsed(),
            window,
            authenticated_total,
            authenticated_during_wait,
            phase: (authenticated_during_wait > 0).then(|| self.observation.latest_phase()),
        }
    }

    /// The slow-acquire record of an acquisition still pending at the
    /// threshold: what it waits for so far and how much of its bound remains.
    fn still_waiting(
        &self,
        started: Instant,
        authenticated_before: u64,
        scope: Option<&BudgetScope>,
    ) {
        let (bound, budget, window, remaining) = match scope {
            Some(scope) => (
                AcquireBound::StatementBudget,
                scope.budget,
                scope.deadline.saturating_duration_since(started),
                scope.deadline.saturating_duration_since(Instant::now()),
            ),
            None => {
                let ceiling = self.pool.options().get_acquire_timeout();
                (
                    AcquireBound::PoolCeiling,
                    ceiling,
                    ceiling,
                    ceiling.saturating_sub(started.elapsed()),
                )
            }
        };
        let waiting = self.diagnose(started, authenticated_before, bound, budget, window);
        tracing::warn!(
            branch = %waiting.branch,
            wait = waiting.wait.label(),
            bound = waiting.bound.label(),
            budget_ms = millis(waiting.budget),
            remaining_ms = millis(remaining),
            max = waiting.max,
            size = waiting.size,
            idle = waiting.idle,
            checked_out = waiting.checked_out,
            pending = self.observation.pending_acquires(),
            waited_ms = millis(waiting.waited),
            authenticated_total = waiting.authenticated_total,
            authenticated_during_wait = waiting.authenticated_during_wait,
            phase = waiting.phase.unwrap_or("none"),
            "memory pool acquire still waiting"
        );
        self.observation.slow_acquire_recorded();
    }

    /// The `Executor` form: a timeout travels as `sqlx::Error::Io` of kind
    /// `Other` carrying the typed diagnostic, because an `Executor` must
    /// return `sqlx::Error` and `PoolTimedOut` is a unit variant.
    async fn statement_session(&self) -> std::result::Result<PooledSession, sqlx::Error> {
        self.acquire_session()
            .await
            .map_err(|failure| match failure {
                AcquireFailure::TimedOut(diagnostic) => {
                    sqlx::Error::Io(io::Error::other(*diagnostic))
                }
                AcquireFailure::Sqlx(error) => error,
            })
    }
}

enum AcquireFailure {
    TimedOut(Box<PoolAcquireTimedOut>),
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
    ///
    /// The session stops counting as held by Kuru work as soon as SQLx takes
    /// its connection, before the release check is awaited: if this future is
    /// cancelled there, SQLx closes the floating connection itself, and a
    /// release still in flight is not a held permit.
    pub async fn release(mut self) {
        if let Some(mut connection) = self.connection.take() {
            let returning = connection.return_to_pool();
            self.checked_out.fetch_sub(1, Ordering::SeqCst);
            returning.await;
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
    /// Capacity existed and a new connection reached Kuru's identity
    /// callback during this wait, after its TCP connect and MySQL
    /// authentication had finished.
    NewConnection,
    /// Capacity existed and no new connection reached Kuru's identity
    /// callback during this wait: an idle-connection check, a release still
    /// in flight, or a TCP or MySQL handshake that did not finish. The
    /// callback is the first point Kuru observes a new connection, so these
    /// cannot be told apart.
    NoIdentityCallback,
}

impl PoolWait {
    pub(crate) fn classify(max: u32, checked_out: u32, authenticated_during_wait: u64) -> Self {
        if checked_out >= max {
            Self::HeldConnections
        } else if authenticated_during_wait > 0 {
            Self::NewConnection
        } else {
            Self::NoIdentityCallback
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::HeldConnections => "held connections",
            Self::NewConnection => "new connection",
            Self::NoIdentityCallback => "no identity callback",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::HeldConnections => "for a connection held by Kuru work (every permit held)",
            Self::NewConnection => "for a new connection in Kuru's identity callback",
            Self::NoIdentityCallback => {
                "with no new connection reaching Kuru's identity callback (an idle-connection check, a release in flight, or a TCP or MySQL handshake that did not finish)"
            }
        }
    }
}

/// Which timer ended a timed-out acquisition. When the two deadlines fall on
/// one timer tick, it is whichever fired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquireBound {
    /// The budget scope of the statement or operation the acquisition served.
    StatementBudget,
    /// The pool's own lifetime acquire timeout, with no earlier budget scope.
    PoolCeiling,
}

impl AcquireBound {
    fn label(self) -> &'static str {
        match self {
            Self::StatementBudget => "statement budget",
            Self::PoolCeiling => "pool ceiling",
        }
    }
}

/// A bounded, secret-free report of a timed-out pool acquisition: no SQL
/// text, credentials, endpoints or paths.
#[derive(Clone, Debug)]
pub struct PoolAcquireTimedOut {
    pub branch: String,
    pub wait: PoolWait,
    /// The bound that ended the wait.
    pub bound: AcquireBound,
    /// The statement or operation budget, or the pool ceiling.
    pub budget: Duration,
    pub max: u32,
    pub size: u32,
    pub idle: usize,
    pub checked_out: u32,
    pub waited: Duration,
    /// The acquisition's own share of its bound: what was left of the budget
    /// when it began, or the whole pool ceiling.
    pub window: Duration,
    pub authenticated_total: u64,
    pub authenticated_during_wait: u64,
    /// The latest authentication phase, only when a connection entered
    /// authentication during this wait; otherwise it would be stale.
    pub phase: Option<&'static str>,
}

impl PoolAcquireTimedOut {
    fn report(&self) {
        tracing::warn!(
            branch = %self.branch,
            wait = self.wait.label(),
            bound = self.bound.label(),
            budget_ms = millis(self.budget),
            window_ms = millis(self.window),
            max = self.max,
            size = self.size,
            idle = self.idle,
            checked_out = self.checked_out,
            waited_ms = millis(self.waited),
            authenticated_total = self.authenticated_total,
            authenticated_during_wait = self.authenticated_during_wait,
            phase = self.phase.unwrap_or("none"),
            "memory pool acquire timed out"
        );
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl fmt::Display for PoolAcquireTimedOut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "memory pool acquire on kuru/{} timed out after {:.3} s ",
            self.branch,
            self.waited.as_secs_f64(),
        )?;
        match self.bound {
            AcquireBound::StatementBudget => write!(
                formatter,
                "(statement budget {:.3} s, {:.3} s left when the acquire began)",
                self.budget.as_secs_f64(),
                self.window.as_secs_f64(),
            )?,
            AcquireBound::PoolCeiling => write!(
                formatter,
                "(pool ceiling {:.3} s)",
                self.window.as_secs_f64()
            )?,
        }
        write!(formatter, " waiting {}", self.wait.describe())?;
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
/// [`MemoryPool::acquire`], the `Executor` carrier, or the source of a
/// [`BudgetElapsed`] that ended an acquisition.
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
            bound: AcquireBound::PoolCeiling,
            budget: Duration::from_secs(2),
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
        assert_eq!(PoolWait::classify(4, 0, 0), PoolWait::NoIdentityCallback);
        assert_eq!(PoolWait::classify(4, 3, 0), PoolWait::NoIdentityCallback);
    }

    #[test]
    fn diagnostic_names_its_wait_and_omits_a_stale_phase() {
        let idle = diagnostic(PoolWait::NoIdentityCallback, None).to_string();
        assert!(
            idle.contains(
                "waiting with no new connection reaching Kuru's identity callback (an idle-connection check, a release in flight, or a TCP or MySQL handshake that did not finish)"
            ),
            "{idle}"
        );
        assert!(!idle.contains("connection phase"), "{idle}");
        assert!(
            idle.contains("timed out after 2.004 s (pool ceiling 2.000 s) waiting with no new"),
            "{idle}"
        );
        assert!(
            idle.contains("pool size 3 of 4, 0 idle, 2 checked out; 7 connections authenticated since the pool opened, 0 during this wait"),
            "{idle}"
        );
        let new = diagnostic(PoolWait::NewConnection, Some("data directory query")).to_string();
        assert!(
            new.contains(
                "for a new connection in Kuru's identity callback (connection phase: data directory query)"
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

    /// A budget that ended an acquisition names the statement budget and the
    /// acquisition's share, and is found through `BudgetElapsed`'s source; a
    /// budget that elapsed during execution carries no acquisition diagnostic.
    #[test]
    fn budget_elapsed_carries_only_an_acquisition_it_ended() {
        let mut acquire = diagnostic(PoolWait::HeldConnections, None);
        acquire.bound = AcquireBound::StatementBudget;
        acquire.budget = Duration::from_millis(500);
        acquire.window = Duration::from_millis(400);
        acquire.waited = Duration::from_millis(401);
        let text = acquire.to_string();
        assert!(
            text.contains(
                "memory pool acquire on kuru/main timed out after 0.401 s (statement budget 0.500 s, 0.400 s left when the acquire began) waiting for a connection held by Kuru work"
            ),
            "{text}"
        );
        let ended = Err::<(), _>(BudgetElapsed {
            budget: Duration::from_millis(500),
            acquire: Some(Box::new(acquire)),
        });
        let error = anyhow::Context::context(ended, "memory read deadline exceeded")
            .expect_err("elapsed budget");
        assert_eq!(
            pool_acquire_timeout(&error).map(|found| found.bound),
            Some(AcquireBound::StatementBudget)
        );
        assert!(
            format!("{error:#}").contains(
                "memory read deadline exceeded: memory statement budget of 0.500 s elapsed while acquiring a pool session: memory pool acquire on kuru/main timed out"
            ),
            "{error:#}"
        );
        assert!(!error.chain().any(|cause| matches!(
            cause.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        )));
        let execution = anyhow::Error::from(BudgetElapsed {
            budget: Duration::from_secs(30),
            acquire: None,
        });
        assert!(pool_acquire_timeout(&execution).is_none());
        assert_eq!(
            execution.to_string(),
            "memory statement budget of 30.000 s elapsed"
        );
    }

    /// Outside any budget scope, the pool's own lifetime ceiling ends an
    /// acquisition, through the funnel's acquire (context over SQLx's
    /// `PoolTimedOut`) and through a statement (the `Executor` carrier). The
    /// fixture's listener completes TCP but never sends a MySQL greeting, so
    /// no connection reaches an identity callback; the fixture's own pool
    /// option is the bound under test.
    #[tokio::test]
    async fn unscoped_acquire_reports_the_pool_ceiling() -> Result<()> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let ceiling = Duration::from_millis(500);
        let pool = MemoryPool::fixture(
            sqlx::mysql::MySqlPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(ceiling)
                .connect_lazy_with(
                    sqlx::mysql::MySqlConnectOptions::new()
                        .host("127.0.0.1")
                        .port(listener.local_addr()?.port())
                        .ssl_mode(sqlx::mysql::MySqlSslMode::Disabled),
                ),
            "main",
        );
        let ensure_ceiling = |error: &anyhow::Error| -> Result<()> {
            let diagnostic = pool_acquire_timeout(error)
                .ok_or_else(|| anyhow::anyhow!("no typed diagnostic: {error:#}"))?;
            anyhow::ensure!(
                diagnostic.bound == AcquireBound::PoolCeiling,
                "{diagnostic}"
            );
            anyhow::ensure!(
                (diagnostic.budget, diagnostic.window) == (ceiling, ceiling),
                "{diagnostic}"
            );
            anyhow::ensure!(diagnostic.waited >= ceiling, "{diagnostic}");
            anyhow::ensure!(
                diagnostic.wait == PoolWait::NoIdentityCallback,
                "{diagnostic}"
            );
            anyhow::ensure!(
                format!("{error:#}").contains("(pool ceiling 0.500 s)"),
                "{error:#}"
            );
            Ok(())
        };
        let error = match pool.acquire().await {
            Ok(_) => anyhow::bail!("a server that never greets handed out a session"),
            Err(error) => error,
        };
        ensure_ceiling(&error)?;
        anyhow::ensure!(
            error.chain().any(|cause| matches!(
                cause.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::PoolTimedOut)
            )),
            "the ceiling's diagnostic lost SQLx's pool timeout: {error:#}"
        );
        let error = match sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&pool)
            .await
        {
            Ok(_) => anyhow::bail!("a server that never greets ran a statement"),
            Err(error) => anyhow::Error::from(error),
        };
        ensure_ceiling(&error)?;
        anyhow::ensure!(
            pool.pending_acquires() == 0,
            "an acquisition is still pending"
        );
        drop(listener);
        Ok(())
    }

    /// A nested scope ends at the earliest enclosing deadline and reports
    /// that deadline's budget; with no acquisition pending, its expiry
    /// carries no acquisition diagnostic. The inner scope ends on an event,
    /// so no time passes.
    #[tokio::test]
    async fn budget_scopes_nest_to_the_earliest_deadline() {
        let outer = Duration::from_secs(30);
        let inner = within(
            outer,
            within_or(
                outer * 120,
                std::future::pending::<()>(),
                std::future::ready(()),
            ),
        )
        .await
        .expect("the enclosing scope has not elapsed")
        .expect_err("the event ends the inner scope");
        assert_eq!(inner.budget, outer);
        assert!(inner.acquire.is_none());
        let shorter = within(
            outer,
            within_or(
                outer / 60,
                std::future::pending::<()>(),
                std::future::ready(()),
            ),
        )
        .await
        .expect("the enclosing scope has not elapsed")
        .expect_err("the event ends the inner scope");
        assert_eq!(shorter.budget, outer / 60);
        assert_eq!(within(outer, async { 7 }).await.ok(), Some(7));
    }
}
