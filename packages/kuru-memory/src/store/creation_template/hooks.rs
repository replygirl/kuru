//! Test-only faults and observations inside the template cache.
//!
//! A test scopes one [`Hooks`] value around the call it exercises with
//! [`HOOKS`]`.scope(...)`; code outside such a scope runs unchanged.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// A step of taking a key lock that a test can fail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LockStep {
    /// Opening the lock file.
    Open,
    /// The non-waiting lock call (`TryLockError::Error`).
    TryLock,
    /// The identity check after locking.
    Verify,
}

/// One read error, injected once, on the first template file whose path
/// ends with `suffix`.
pub(crate) struct ReadFault {
    pub(crate) suffix: String,
    fired: AtomicBool,
}

impl ReadFault {
    pub(crate) fn new(suffix: &str) -> Arc<Self> {
        Arc::new(Self {
            suffix: suffix.to_owned(),
            fired: AtomicBool::new(false),
        })
    }

    pub(crate) fn fired(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }
}

/// A pause inside the template build, after the chain and before the shape
/// check, on the live build engine.
#[derive(Default)]
pub(crate) struct Pause {
    pub(crate) reached: tokio::sync::Notify,
    pub(crate) resume: tokio::sync::Notify,
}

/// An ordered observation of the capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    /// A file or directory was synced.
    Synced(PathBuf),
    /// The capture's manifest was written and synced.
    Manifest,
}

#[derive(Clone, Default)]
pub(crate) struct Hooks {
    pub(crate) lock: Option<LockStep>,
    pub(crate) manifest_read_error: bool,
    pub(crate) read: Option<Arc<ReadFault>>,
    pub(crate) refuse_publication: bool,
    pub(crate) before_quarantine: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Statements run on the build engine's `main` before the shape check.
    pub(crate) before_shape: Vec<String>,
    pub(crate) pause: Option<Arc<Pause>>,
    pub(crate) events: Option<Arc<StdMutex<Vec<Event>>>>,
}

tokio::task_local! {
    pub(crate) static HOOKS: Hooks;
}

fn current() -> Option<Hooks> {
    HOOKS.try_with(Clone::clone).ok()
}

pub(super) fn lock_fault(step: LockStep) -> Result<()> {
    if current().and_then(|hooks| hooks.lock) == Some(step) {
        bail!("injected store template lock fault at {step:?}");
    }
    Ok(())
}

pub(super) fn manifest_fault() -> Result<()> {
    if current().is_some_and(|hooks| hooks.manifest_read_error) {
        return Err(std::io::Error::other("injected manifest read error").into());
    }
    Ok(())
}

pub(super) fn read_fault(object: &Path) -> Result<()> {
    if let Some(fault) = current().and_then(|hooks| hooks.read)
        && object.to_string_lossy().ends_with(&fault.suffix)
        && !fault.fired.swap(true, Ordering::AcqRel)
    {
        return Err(std::io::Error::other("injected template read error").into());
    }
    Ok(())
}

pub(super) fn publish_fault() -> Result<()> {
    if current().is_some_and(|hooks| hooks.refuse_publication) {
        bail!("injected store template publication failure");
    }
    Ok(())
}

pub(super) async fn before_quarantine() {
    if let Some(hook) = current().and_then(|hooks| hooks.before_quarantine) {
        hook();
    }
}

pub(super) fn synced(path: &Path) {
    if let Some(events) = current().and_then(|hooks| hooks.events) {
        events
            .lock()
            .expect("hook events")
            .push(Event::Synced(path.to_owned()));
    }
}

pub(super) fn manifest_written() {
    if let Some(events) = current().and_then(|hooks| hooks.events) {
        events.lock().expect("hook events").push(Event::Manifest);
    }
}

/// On the live build engine, before the shape check: run the scoped
/// statements, then wait at the scoped pause.
pub(in crate::store) async fn before_shape(main: &MySqlPool) -> Result<()> {
    let Some(hooks) = current() else {
        return Ok(());
    };
    for statement in &hooks.before_shape {
        tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query(sqlx::AssertSqlSafe(statement.clone())).execute(main),
        )
        .await
        .context("injected build statement deadline exceeded")??;
    }
    if let Some(pause) = hooks.pause {
        pause.reached.notify_one();
        pause.resume.notified().await;
    }
    Ok(())
}
