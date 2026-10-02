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

/// One read failure, injected once, on the first template file whose path
/// ends with `suffix` and, when `under` is set, has a component named
/// `under`: a published template's files lie beneath its key, and a build's
/// capture and verification walks do not. An I/O error unless `verdict`.
pub(crate) struct ReadFault {
    pub(crate) suffix: String,
    under: Option<String>,
    verdict: bool,
    fired: AtomicBool,
}

impl ReadFault {
    pub(crate) fn new(suffix: &str) -> Arc<Self> {
        Arc::new(Self {
            suffix: suffix.to_owned(),
            under: None,
            verdict: false,
            fired: AtomicBool::new(false),
        })
    }

    /// The first read of any file beneath the published template named
    /// `key` fails: with a byte verdict when `verdict`, else an I/O error.
    pub(crate) fn published(key: &str, verdict: bool) -> Arc<Self> {
        Arc::new(Self {
            suffix: String::new(),
            under: Some(key.to_owned()),
            verdict,
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

/// An ordered observation of the capture, or of a project's creation from
/// the template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    /// A file or directory was synced.
    Synced(PathBuf),
    /// The capture's manifest was written and synced.
    Manifest,
    /// A template stage's identity record is about to be written into this
    /// stage, after its copy.
    Identity(PathBuf),
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
    /// Statements run on the build engine's `main` after the shape check
    /// passed: a published template whose bytes a copy's own shape check
    /// then refuses.
    pub(crate) after_shape: Vec<String>,
    pub(crate) pause: Option<Arc<Pause>>,
    pub(crate) events: Option<Arc<StdMutex<Vec<Event>>>>,
    /// After the build engine is reaped and before the capture: write the
    /// build store's own path into a new file of its `data/`, as a leak the
    /// byte scan must find.
    pub(crate) plant_leak: bool,
    /// The template key a project stage's identity record names in place
    /// of the compiled one: a stage the supervisor refuses before adoption,
    /// as a stage copied by another build would be.
    pub(crate) stage_key: Option<String>,
}

/// The file [`Hooks::plant_leak`] writes, relative to the build store's
/// `data/`.
pub(crate) const PLANTED_LEAK: &str = "kuru-planted-leak";

tokio::task_local! {
    pub(crate) static HOOKS: Hooks;
}

fn current() -> Option<Hooks> {
    HOOKS.try_with(Clone::clone).ok()
}

/// The hooks scoped around the calling task, for work it hands to another
/// task or a blocking thread, which re-enters them: task-local values do not
/// cross `tokio::spawn` or `spawn_blocking`.
pub(crate) fn captured() -> Option<Hooks> {
    current()
}

pub(in crate::store) fn identity_written(stage: &Path) {
    if let Some(events) = current().and_then(|hooks| hooks.events) {
        events
            .lock()
            .expect("hook events")
            .push(Event::Identity(stage.to_owned()));
    }
}

/// The key to write into a project stage's identity record instead of the
/// compiled one, when scoped.
pub(in crate::store) fn stage_key() -> Option<String> {
    current().and_then(|hooks| hooks.stage_key)
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

pub(super) fn read_fault(object: &Path) -> Result<(), CreationFailure> {
    if let Some(fault) = current().and_then(|hooks| hooks.read)
        && object.to_string_lossy().ends_with(&fault.suffix)
        && fault.under.as_deref().is_none_or(|under| {
            object
                .components()
                .any(|component| component.as_os_str() == under)
        })
        && !fault.fired.swap(true, Ordering::AcqRel)
    {
        if fault.verdict {
            return Err(CreationFailure::verdict(format!(
                "injected digest mismatch on {}",
                object.display()
            )));
        }
        return Err(CreationFailure::io(std::io::Error::other(
            "injected template read error",
        )));
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

pub(super) fn before_capture(store: &Path) -> Result<()> {
    if current().is_some_and(|hooks| hooks.plant_leak) {
        let mut bytes = b"planted before ".to_vec();
        bytes.extend_from_slice(store.to_string_lossy().as_bytes());
        bytes.extend_from_slice(b" after");
        files::write(&store.join(DATA).join(PLANTED_LEAK), &bytes)?;
    }
    Ok(())
}

pub(super) fn manifest_written() {
    if let Some(events) = current().and_then(|hooks| hooks.events) {
        events.lock().expect("hook events").push(Event::Manifest);
    }
}

/// Run `statements` in order on one connection of `main`, so session state
/// such as `USE` carries from one to the next. A caller that changes the
/// session's database restores it last: the connection returns to the pool.
async fn execute(main: &MemoryPool, statements: &[String]) -> Result<()> {
    if statements.is_empty() {
        return Ok(());
    }
    let mut connection = tokio::time::timeout(QUERY_TIMEOUT, main.acquire())
        .await
        .context("injected build statement connection deadline exceeded")??;
    for statement in statements {
        tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query(sqlx::AssertSqlSafe(statement.clone())).execute(&mut *connection),
        )
        .await
        .context("injected build statement deadline exceeded")??;
    }
    Ok(())
}

/// On the live build engine, before the shape check: run the scoped
/// statements, then wait at the scoped pause.
pub(in crate::store) async fn before_shape(main: &MemoryPool) -> Result<()> {
    let Some(hooks) = current() else {
        return Ok(());
    };
    execute(main, &hooks.before_shape).await?;
    if let Some(pause) = hooks.pause {
        pause.reached.notify_one();
        pause.resume.notified().await;
    }
    Ok(())
}

/// On the live build engine, after the shape check passed: run the scoped
/// statements.
pub(in crate::store) async fn after_shape(main: &MemoryPool) -> Result<()> {
    match current() {
        Some(hooks) => execute(main, &hooks.after_shape).await,
        None => Ok(()),
    }
}
