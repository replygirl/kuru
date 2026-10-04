//! Test-support spawn rows for the coverage partition's runner ledger.
//!
//! Every entry point is a no-op unless [`SPAWN_LEDGER_ENV`] names the
//! partition's runner ledger, which the coverage runner sets on each test
//! process it starts. A memory supervisor, a memory service owner and a
//! terminal fixture child leave the test's process group, so the runner's
//! group cleanup never reaches them: when one outlives its test, its coverage
//! profile appears after the partition's tests. Each spawn site appends one
//! JSON line naming the child's process ID, its executable, its role and the
//! originating test, so the partition's profile invariant can name the test
//! behind a late `kuru-<pid>-<signature>.profraw`.
//!
//! Recording never writes to stdout or stderr, never returns an error into
//! the caller's path and never changes the spawn it describes. One line is
//! one `write` to a file opened for append, so concurrent writers do not
//! interleave within a line.

use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// The coverage runner's ledger, set on each test process it starts.
pub const SPAWN_LEDGER_ENV: &str = "KURU_COVERAGE_SPAWN_LEDGER";
/// Spawn row format version; `kuru-delivery` parses exactly this.
pub const SPAWN_ROW_VERSION: u32 = 1;
/// A Dolt lifetime supervisor (`--internal-dolt-supervisor`).
pub const DOLT_SUPERVISOR: &str = "dolt-supervisor";
/// A project memory service owner (`--internal-memory-service`).
pub const MEMORY_OWNER: &str = "memory-owner";
/// A child a test starts on a pseudo-terminal, in its own session.
pub const TERMINAL_CHILD: &str = "terminal-child";

fn ledger() -> Option<OsString> {
    std::env::var_os(SPAWN_LEDGER_ENV).filter(|value| !value.is_empty())
}

/// Environment a spawned child needs so its own spawns are recorded against
/// the same test: the ledger, and the originating test's label when the
/// lifecycle trace is not already forwarding it.
pub fn forwarded() -> Vec<(OsString, OsString)> {
    let Some(path) = ledger() else {
        return Vec::new();
    };
    let mut environment = vec![(SPAWN_LEDGER_ENV.into(), path)];
    if super::lifecycle_trace::root().is_none() {
        environment.push((
            super::lifecycle_trace::LABEL_ENV.into(),
            super::lifecycle_trace::label().into(),
        ));
    }
    environment
}

/// The spawn row for `pid`, as one line without its terminator.
pub fn row(pid: u32, executable: &Path, role: &str, test: &str, at: u64) -> String {
    serde_json::json!({
        "record": "spawn",
        "version": SPAWN_ROW_VERSION,
        "pid": pid,
        "parent": std::process::id(),
        "role": role,
        "executable": executable.to_string_lossy(),
        "test": test,
        "at": at,
    })
    .to_string()
}

/// Append the spawn row of a child this process just started.
pub fn record(pid: u32, executable: &Path, role: &str) {
    let Some(path) = ledger() else {
        return;
    };
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let mut line = row(pid, executable, role, &super::lifecycle_trace::label(), at);
    line.push('\n');
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_one_line_naming_the_child_its_role_and_the_test() {
        let row = row(
            41,
            Path::new("/target/debug/kuru-memory"),
            DOLT_SUPERVISOR,
            "tests::one",
            7,
        );
        assert!(!row.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(&row).unwrap();
        assert_eq!(value["record"], "spawn");
        assert_eq!(value["version"], SPAWN_ROW_VERSION);
        assert_eq!(value["pid"], 41);
        assert_eq!(value["parent"], std::process::id());
        assert_eq!(value["role"], DOLT_SUPERVISOR);
        assert_eq!(value["executable"], "/target/debug/kuru-memory");
        assert_eq!(value["test"], "tests::one");
        assert_eq!(value["at"], 7);
    }
}
