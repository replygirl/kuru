//! Test-support measurement trace for memory lifecycle ordering.
//!
//! Every entry point is a no-op unless [`LOG_DIR_ENV`] names a directory.
//! Recording never writes to stdout or stderr (a PTY may be attached), never
//! returns an error into the caller's path and never changes the outcome it
//! observes. It records, per process:
//!
//! - `dolt-<pid>-<nanos>.log`: a live mirror of one supervised Dolt server's
//!   output (the in-directory `server.log` is truncated and is lost with its
//!   fixture), with `@event` lines for the supervisor's stop and reap.
//! - `events-<pid>.log`: parent-side owner reap/drop handoffs and lifecycle
//!   lease directory moves/removals.
//!
//! Each line carries wall-clock nanoseconds and the originating test label
//! (the test thread name, forwarded to supervisors and service children).

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

/// Directory receiving the trace; unset or empty disables every recorder.
pub const LOG_DIR_ENV: &str = "KURU_TEST_DOLT_LOG_DIR";
/// Label forwarded to child processes so their records name the test.
pub const LABEL_ENV: &str = "KURU_TEST_DOLT_LOG_LABEL";
const MIRROR_CAP: usize = 256 * 1024;

pub fn root() -> Option<PathBuf> {
    std::env::var_os(LOG_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_graphic() {
                character
            } else {
                '_'
            }
        })
        .take(200)
        .collect()
}

/// The originating test: an inherited label, else this thread's name.
pub fn label() -> String {
    if let Some(value) = std::env::var_os(LABEL_ENV).filter(|value| !value.is_empty()) {
        return sanitize(&value.to_string_lossy());
    }
    sanitize(std::thread::current().name().unwrap_or("unnamed"))
}

/// Environment a spawned supervisor or service child needs to keep recording.
pub fn forwarded() -> Vec<(OsString, OsString)> {
    let Some(root) = root() else {
        return Vec::new();
    };
    vec![
        (LOG_DIR_ENV.into(), root.into_os_string()),
        (LABEL_ENV.into(), label().into()),
    ]
}

/// Append one parent-side lifecycle event.
pub fn event(kind: &str, fields: std::fmt::Arguments<'_>) {
    let Some(root) = root() else {
        return;
    };
    if fs::create_dir_all(&root).is_err() {
        return;
    }
    let line = format!(
        "t={} pid={} label={} kind={kind} {fields}\n",
        nanos(),
        std::process::id(),
        label()
    );
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(format!("events-{}.log", std::process::id())))
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Whether a path currently resolves, for event records only.
pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Supervisor-side live mirror of one Dolt sql-server's stdout and stderr.
pub struct DoltMirror {
    state: Mutex<(File, usize)>,
    secrets: Vec<String>,
}

impl DoltMirror {
    pub fn open(directory: &Path, secrets: Vec<String>) -> Option<Arc<Self>> {
        let root = root()?;
        fs::create_dir_all(&root).ok()?;
        let path = root.join(format!("dolt-{}-{}.log", std::process::id(), nanos()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(path)
            .ok()?;
        writeln!(
            file,
            "@event t={} kind=spawn label={} directory={}",
            nanos(),
            label(),
            directory.display()
        )
        .ok()?;
        Some(Arc::new(Self {
            state: Mutex::new((file, 0)),
            secrets,
        }))
    }

    pub fn write(&self, bytes: &[u8]) {
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        for secret in &self.secrets {
            if !secret.is_empty() {
                text = text.replace(secret, "[redacted]");
            }
        }
        if let Ok(mut state) = self.state.lock() {
            if state.1 >= MIRROR_CAP {
                return;
            }
            state.1 += text.len();
            let _ = state.0.write_all(text.as_bytes());
        }
    }

    pub fn note(&self, kind: &str, fields: std::fmt::Arguments<'_>) {
        if let Ok(mut state) = self.state.lock() {
            let _ = writeln!(state.0, "\n@event t={} kind={kind} {fields}", nanos());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_single_printable_tokens() {
        assert_eq!(sanitize("a b\tc::d"), "a_b_c::d");
        assert!(!label().contains(' '));
    }
}
