//! Advisory file locks released by their owner rather than by the last close.
//!
//! On Unix a `flock` belongs to the open file description and closing a
//! descriptor releases it only once every descriptor referring to that
//! description is closed. A child that any thread of this process is spawning
//! holds a copy of every descriptor until its exec closes the close-on-exec
//! ones, so a lock released only by close can outlive its owner. On Windows,
//! closing a handle that still holds byte-range locks unlocks them at a time
//! the system does not bound. [`HeldLock`] therefore unlocks explicitly before
//! its file closes, on every exit path including unwinding and cancellation.

use std::{fs::File, ops::Deref};

/// An acquired exclusive advisory lock on a checked lock file.
#[derive(Debug)]
pub(crate) struct HeldLock(File);

impl HeldLock {
    /// Takes ownership of a file whose lock this process has just acquired.
    pub(crate) fn acquired(file: File) -> Self {
        Self(file)
    }
}

impl Deref for HeldLock {
    type Target = File;

    fn deref(&self) -> &File {
        &self.0
    }
}

impl Drop for HeldLock {
    fn drop(&mut self) {
        // A failed unlock leaves release to the close that follows, which is
        // the most that can be done during drop.
        let _ = self.0.unlock();
    }
}
