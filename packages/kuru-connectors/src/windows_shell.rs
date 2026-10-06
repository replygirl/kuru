//! Built-in Windows shell work retains its native owner beyond caller drop.

use anyhow::{Context, Result, bail, ensure};
use kuru_platform::{fs::Directory, windows::process::NativeChild};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, oneshot};

use crate::{
    InvocationHold,
    shell_diagnostic::{ShellCapture, ShellFailureCategory, failure_with_cleanup},
};

pub(crate) struct ShellRegistry {
    inner: Arc<Inner>,
}
struct Inner {
    state: Mutex<State>,
    next: AtomicU64,
    changed: Notify,
}
#[derive(Default)]
struct State {
    closing: bool,
    owners: BTreeMap<u64, Arc<Control>>,
}
pub(crate) struct Control {
    cancelled: AtomicBool,
    changed: Notify,
}
impl Control {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }
    pub(crate) async fn cancelled(&self) {
        loop {
            let notified = self.changed.notified();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        for owner in self
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owners
            .values()
        {
            owner.cancel();
        }
    }
}
struct Caller(Arc<Control>);
impl Drop for Caller {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct Worker {
    registry: Weak<Inner>,
    id: u64,
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .owners
                .remove(&self.id);
            registry.changed.notify_waiters();
        }
    }
}

impl ShellRegistry {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                next: AtomicU64::new(1),
                changed: Notify::new(),
            }),
        }
    }

    pub(crate) async fn execute(
        &self,
        root: Arc<Directory>,
        path: PathBuf,
        command: String,
        duration: Duration,
        progress: Option<crate::ShellProgress>,
        hold: Option<InvocationHold>,
    ) -> Result<String> {
        ensure!(!command.trim().is_empty(), "shell command is empty");
        let control = Arc::new(Control {
            cancelled: AtomicBool::new(false),
            changed: Notify::new(),
        });
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            ensure!(!state.closing, "shell host is shutting down");
            state.owners.insert(id, control.clone());
        }
        let caller = Caller(control.clone());
        let worker = Worker {
            registry: Arc::downgrade(&self.inner),
            id,
        };
        let (reply, result) = oneshot::channel();
        std::thread::Builder::new()
            .name("kuru-windows-shell".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        let _ =
                            reply.send(Err(anyhow::anyhow!("Windows shell worker runtime failed")));
                        return;
                    }
                };
                let mut owner: Option<NativeChild> = None;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runtime.block_on(crate::tools::windows_shell_owned(
                        &root,
                        &path,
                        &command,
                        duration,
                        progress.as_ref(),
                        &control,
                        &mut owner,
                    ))
                }));
                let result = result.unwrap_or_else(|_| {
                    Err(failure_with_cleanup(
                        ShellFailureCategory::OperationFailed,
                        owner.is_some(),
                        &ShellCapture::new(),
                    ))
                });
                let result = match result {
                    Err(error)
                        if error
                            .downcast_ref::<crate::shell_diagnostic::ProjectedShellDiagnostic>()
                            .is_some() =>
                    {
                        Err(error)
                    }
                    Err(_) => {
                        let unconfirmed = owner
                            .as_mut()
                            .is_some_and(|owner| !matches!(owner.try_wait(), Ok(Some(_))));
                        Err(if unconfirmed {
                            failure_with_cleanup(
                                ShellFailureCategory::OperationFailed,
                                true,
                                &ShellCapture::new(),
                            )
                        } else {
                            crate::shell_diagnostic::failure(
                                ShellFailureCategory::OperationFailed,
                                &ShellCapture::new(),
                            )
                        })
                    }
                    Ok(result) => Ok(result),
                };
                // Existing result/refusal is available before a retained cleanup
                // retry. This thread owns the actual child and immutable hold.
                let _ = reply.send(result);
                if let Some(mut owner) = owner {
                    runtime.block_on(async {
                        while !matches!(owner.try_wait(), Ok(Some(_))) {
                            if crate::process::stop(&mut owner).await.is_ok() {
                                break;
                            }
                            // Reuse the native wait's poll interval solely for
                            // retrying retained cleanup; it grants no admission.
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    });
                }
                drop(hold);
                drop(worker);
            })
            .context("cannot start retained Windows shell owner")?;
        let result = result
            .await
            .context("Windows shell owner ended without a result");
        drop(caller);
        result?
    }

    pub(crate) async fn shutdown(&self) -> Result<()> {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closing = true;
            for owner in state.owners.values() {
                owner.cancel();
            }
        }
        // Existing native stop plus parallel pipe close allowances. A refusal
        // leaves the workers/barriers retained; it does not claim quiescence.
        if tokio::time::timeout(crate::process::CLEANUP.saturating_mul(2), async {
            loop {
                let changed = self.inner.changed.notified();
                if self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .owners
                    .is_empty()
                {
                    return;
                }
                changed.await;
            }
        })
        .await
        .is_err()
        {
            bail!("Windows shell cleanup remains unconfirmed; ownership is retained");
        }
        Ok(())
    }
}
