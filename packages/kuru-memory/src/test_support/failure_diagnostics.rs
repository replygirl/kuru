//! DRAFT: not for merge. Test-support-only failure evidence for the macOS
//! "write outcome is uncertain" investigation.
//!
//! Every entry point is a no-op unless `KURU_TEST_FAILURE_DIAGNOSTICS_DIR`
//! names a directory. Recording never returns an error into the caller's
//! path and never changes the outcome it observes.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DIAGNOSTICS_ENV: &str = "KURU_TEST_FAILURE_DIAGNOSTICS_DIR";
/// Overrides the fixture open-concurrency permit count (default 4).
pub const OPEN_PERMITS_ENV: &str = "KURU_TEST_OPEN_PERMITS";
const OUTPUT_CAP: usize = 256 * 1024;
const OPERATION_CAP: usize = 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn root() -> Option<PathBuf> {
    std::env::var_os(DIAGNOSTICS_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

// DRAFT experiment: halve the macOS fixture open-concurrency default (4 -> 2)
// while investigating uncertain-write evidence on that platform. Not for merge.
#[cfg(target_os = "macos")]
const DEFAULT_OPEN_PERMITS: usize = 2;
#[cfg(not(target_os = "macos"))]
const DEFAULT_OPEN_PERMITS: usize = 4;

pub(crate) fn open_permit_limit() -> usize {
    std::env::var(OPEN_PERMITS_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_OPEN_PERMITS)
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect()
}

pub(crate) fn bounded_debug(value: &impl std::fmt::Debug) -> String {
    let mut text = format!("{value:?}");
    if text.len() > OPERATION_CAP {
        let mut end = OPERATION_CAP;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...[truncated]");
    }
    text
}

fn command_output(program: &str, arguments: &[&std::ffi::OsStr]) -> String {
    match std::process::Command::new(program).args(arguments).output() {
        Ok(output) => {
            let mut text = format!("$ {program} {arguments:?} -> {}\n", output.status);
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            if text.len() > OUTPUT_CAP {
                let mut end = OUTPUT_CAP;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
                text.push_str("\n...[truncated]\n");
            }
            text
        }
        Err(error) => format!("$ {program} {arguments:?} failed to start: {error}\n"),
    }
}

fn filesystem_state(directory: &Path) -> String {
    let mut text = String::new();
    let data_dir = directory
        .ancestors()
        .find(|ancestor| ancestor.join("memory").is_dir())
        .map(Path::to_path_buf);
    if cfg!(windows) {
        text.push_str(&command_output(
            "cmd",
            &[
                "/c".as_ref(),
                "dir".as_ref(),
                "/s".as_ref(),
                directory.as_os_str(),
            ],
        ));
        return text;
    }
    text.push_str(&command_output(
        "ls",
        &["-laR".as_ref(), directory.as_os_str()],
    ));
    if let Some(data_dir) = &data_dir {
        text.push_str(&command_output(
            "ls",
            &["-la".as_ref(), data_dir.as_os_str()],
        ));
        let lifecycles = data_dir.join("memory/lifecycles");
        text.push_str(&command_output(
            "ls",
            &["-laR".as_ref(), lifecycles.as_os_str()],
        ));
    }
    text.push_str(&command_output(
        "df",
        &["-k".as_ref(), directory.as_os_str()],
    ));
    text.push_str(&command_output(
        "df",
        &["-i".as_ref(), directory.as_os_str()],
    ));
    text
}

fn permit_state() -> String {
    match crate::store::TEMPORARY_PERMITS.get() {
        Some(permits) => format!(
            "fixture open permits in this process: limit={} available={} occupied={}\n",
            open_permit_limit(),
            permits.available_permits(),
            open_permit_limit().saturating_sub(permits.available_permits())
        ),
        None => format!(
            "fixture open permits not initialized in this process (limit would be {})\n",
            open_permit_limit()
        ),
    }
}

fn dolt_live_logs(root: &Path, directory: &Path) -> String {
    let needle = format!("directory: {}", directory.display());
    let mut text = String::new();
    let Ok(entries) = fs::read_dir(root.join("dolt-live")) else {
        return "no dolt-live mirror directory\n".into();
    };
    for entry in entries.flatten() {
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let content = String::from_utf8_lossy(&bytes);
        if content
            .lines()
            .next()
            .is_some_and(|line| line.starts_with(&needle))
        {
            let tail = &content[content.len().saturating_sub(OUTPUT_CAP)..];
            text.push_str(&format!(
                "=== {} ({} bytes)\n{tail}\n",
                entry.path().display(),
                bytes.len()
            ));
        }
    }
    if text.is_empty() {
        text.push_str("no matching dolt-live mirror for this directory\n");
    }
    text
}

/// Record a failure bundle and print its path. `kind` is `client-uncertain`
/// or `service-storage-failed`.
pub(crate) fn record(kind: &str, directory: &Path, operation: &str, error: &anyhow::Error) {
    if let Some(root) = root() {
        record_in(&root, kind, directory, operation, error);
    }
}

fn record_in(
    root: &Path,
    kind: &str,
    directory: &Path,
    operation: &str,
    error: &anyhow::Error,
) -> Option<PathBuf> {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed").to_owned();
    let bundle = root.join("bundles").join(format!(
        "{}-{kind}-{}-{}-{}",
        sanitize(&thread_name),
        std::process::id(),
        nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&bundle).ok()?;
    let summary = format!(
        "kind: {kind}\npid: {}\nthread: {thread_name}\nunix_nanos: {}\nexecutable: {:?}\noperation: {operation}\nstore directory: {}\n{}\nerror (display chain): {error:#}\n\nerror (debug):\n{error:?}\n",
        std::process::id(),
        nanos(),
        std::env::current_exe().ok(),
        directory.display(),
        permit_state(),
    );
    let _ = fs::write(bundle.join("summary.txt"), summary);
    let _ = fs::write(bundle.join("filesystem.txt"), filesystem_state(directory));
    let _ = fs::write(
        bundle.join("dolt-live.txt"),
        dolt_live_logs(root, directory),
    );
    if let Ok(bytes) = fs::read(directory.join("server.log")) {
        let tail = &bytes[bytes.len().saturating_sub(OUTPUT_CAP)..];
        let _ = fs::write(bundle.join("server.log.at-record"), tail);
    }
    eprintln!(
        "DRAFT failure diagnostics bundle ({kind}): {}",
        bundle.display()
    );
    // The retained server.log appears only after the supervisor stops. Harvest
    // it if the fixture directory still exists by then.
    let directory = directory.to_owned();
    let recorded = bundle.clone();
    let _ = std::thread::Builder::new()
        .name("kuru-failure-diagnostics".into())
        .spawn(move || {
            let log = directory.join("server.log");
            let started = SystemTime::now();
            for _ in 0..480 {
                std::thread::sleep(Duration::from_millis(250));
                let fresh = fs::metadata(&log)
                    .and_then(|metadata| metadata.modified())
                    .is_ok_and(|modified| modified >= started);
                if fresh {
                    if let Ok(bytes) = fs::read(&log) {
                        let _ = fs::write(bundle.join("server.log.after-shutdown"), bytes);
                    }
                    return;
                }
            }
            let _ = fs::write(
                bundle.join("server.log.after-shutdown.missing"),
                "no fresh server.log within 120 s\n",
            );
        });
    Some(recorded)
}

/// Supervisor-side live mirror of the Dolt sql-server stdout/stderr.
pub struct DoltMirror {
    file: Mutex<File>,
    secrets: Vec<String>,
}

impl DoltMirror {
    pub(crate) fn open(directory: &Path, secrets: Vec<String>) -> Option<Arc<Self>> {
        let root = root()?.join("dolt-live");
        fs::create_dir_all(&root).ok()?;
        let path = root.join(format!("{}-{}.log", std::process::id(), nanos()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(path)
            .ok()?;
        writeln!(file, "directory: {}", directory.display()).ok()?;
        Some(Arc::new(Self {
            file: Mutex::new(file),
            secrets,
        }))
    }

    pub(crate) fn write(&self, bytes: &[u8]) {
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        for secret in &self.secrets {
            if !secret.is_empty() {
                text = text.replace(secret, "[redacted]");
            }
        }
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(text.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_records_error_filesystem_and_matching_dolt_mirror() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let store = tempfile::tempdir()?;
        let directory = store.path().join("memory").join("project");
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("server.log"), "prior shutdown\n")?;
        fs::create_dir_all(root.path().join("dolt-live"))?;
        fs::write(
            root.path().join("dolt-live/1-1.log"),
            format!(
                "directory: {}.staging-x\nDolt said hello\n",
                directory.display()
            ),
        )?;
        fs::write(
            root.path().join("dolt-live/2-2.log"),
            "directory: /elsewhere\nunrelated\n",
        )?;
        let error = anyhow::anyhow!("Error 1105 (HY000): fsync failed").context("storage");
        let bundle =
            record_in(root.path(), "client-uncertain", &directory, "Op", &error).expect("bundle");
        let summary = fs::read_to_string(bundle.join("summary.txt"))?;
        assert!(summary.contains("storage: Error 1105 (HY000): fsync failed"));
        assert!(summary.contains("operation: Op"));
        assert!(summary.contains("fixture open permits"));
        let live = fs::read_to_string(bundle.join("dolt-live.txt"))?;
        assert!(live.contains("Dolt said hello") && !live.contains("unrelated"));
        assert!(bundle.join("server.log.at-record").is_file());
        #[cfg(unix)]
        {
            let filesystem = fs::read_to_string(bundle.join("filesystem.txt"))?;
            assert!(filesystem.contains("server.log") && filesystem.contains("$ df"));
        }
        Ok(())
    }

    #[test]
    fn operation_labels_are_bounded() {
        assert_eq!(
            bounded_debug(&"x".repeat(2000)).len(),
            OPERATION_CAP + "...[truncated]".len()
        );
        assert_eq!(bounded_debug(&"op"), "\"op\"");
    }
}
