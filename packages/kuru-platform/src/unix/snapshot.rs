//! Read-only, bounded process-tree descriptions and cleanup membership.
//!
//! One stock `ps` invocation lists every process; the rows are filtered in Rust
//! to a root, the members of the process group it leads and the parent-ID
//! closure of both. A descendant reparented after its parent exited is still
//! found through its group. Rich listings use `-A`. Minimal pre-reap membership
//! uses macOS's documented group selector; Linux retains `-A` because procps
//! gives `-g` a different meaning.
//!
//! The result is point-in-time text. Numeric IDs are printed, never signalled,
//! waited for or otherwise acted on, so a reused ID can only mislabel a row.
//! Callers take the snapshot while they still anchor the root's identity, for
//! example before reaping it. Pre-reap cleanup uses an absolute-deadline
//! membership listing while the root remains retained. Another use is
//! [`group_members_within`], which a post-reap cleanup reads at most once, after
//! signal zero was refused with `EPERM` and within the time left before its
//! deadline, to tell a group recycled by another user from a survivor of ours.

use std::{
    collections::BTreeSet,
    fmt,
    io::{self, Read},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// Stock location on macOS and on usrmerged Linux distributions.
pub const PS: &str = "/bin/ps";

/// Polling allowance for `ps`, including draining its pipes. Native spawn and
/// helper cleanup can finish later under their retained worker ownership.
pub const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(2);

const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
const ROW_LIMIT: usize = 64;
const COMMAND_LIMIT: usize = 512;
const DIAGNOSTIC_COLUMNS: &str = "pid=,ppid=,pgid=,uid=,ruid=,stat=,time=,rss=,args=";
const MEMBERSHIP_COLUMNS: &str = "pid=,pgid=,stat=";

/// Minimal read-only membership, without filtering by user or terminal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupMember {
    pub pid: u32,
    pub pgid: u32,
    pub state: String,
}

/// One process as `ps` reported it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRow {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    /// Effective user ID.
    pub uid: u32,
    /// Real user ID.
    pub ruid: u32,
    pub state: String,
    /// Cumulative CPU time in `ps`'s own `time` format.
    pub cpu_time: String,
    pub rss_kib: u64,
    /// Command and arguments, truncated for diagnostics.
    pub command: String,
}

impl fmt::Display for ProcessRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "pid={} ppid={} pgid={} uid={} ruid={} stat={} time={} rss={}KiB args={}",
            self.pid,
            self.ppid,
            self.pgid,
            self.uid,
            self.ruid,
            self.state,
            self.cpu_time,
            self.rss_kib,
            self.command
        )
    }
}

/// Rows for `root`, its led group and their descendants, ordered by ID.
pub fn tree(root: u32) -> io::Result<Vec<ProcessRow>> {
    tree_with(Path::new(PS), root)
}

/// [`tree`] through an explicit `ps` executable; a test seam for failures.
pub fn tree_with(program: &Path, root: u32) -> io::Result<Vec<ProcessRow>> {
    Ok(select(parse(&list(program)?)?, root))
}

/// Every listed member of process group `group`, ordered by ID.
///
/// Unlike [`tree`], no parent closure is followed: the rows answer only which
/// processes `ps` currently lists in that numeric group. A system that hides
/// other users' processes (for example Linux `hidepid`) omits them; the
/// caller's own processes are always listed.
pub fn group_members(group: u32) -> io::Result<Vec<ProcessRow>> {
    group_members_with(Path::new(PS), group)
}

/// [`group_members`] through an explicit `ps` executable.
pub fn group_members_with(program: &Path, group: u32) -> io::Result<Vec<ProcessRow>> {
    group_members_within_with(program, group, SNAPSHOT_TIMEOUT)
}

/// [`group_members`] bounded by `timeout`, which never exceeds
/// [`SNAPSHOT_TIMEOUT`]. A zero timeout is a timed-out listing.
pub fn group_members_within(group: u32, timeout: Duration) -> io::Result<Vec<ProcessRow>> {
    group_members_within_with(Path::new(PS), group, timeout)
}

/// [`group_members_within`] through an explicit `ps` executable.
pub fn group_members_within_with(
    program: &Path,
    group: u32,
    timeout: Duration,
) -> io::Result<Vec<ProcessRow>> {
    Ok(members_of(
        parse(&list_within(program, timeout.min(SNAPSHOT_TIMEOUT))?)?,
        group,
    ))
}

/// Read-only membership with absolute admission and polling deadlines.
///
/// Busy platform spawn-lock admission returns `WouldBlock`, without waiting
/// behind another spawn. Native spawn and helper cleanup are not preemptible;
/// callers needing nonblocking polling retain this work on an owned worker.
pub fn group_members_until_with(
    program: &Path,
    group: u32,
    deadline: Instant,
) -> io::Result<Vec<GroupMember>> {
    group_members_until(program, group, deadline, &AtomicBool::new(false))
}

pub(super) fn group_members_until(
    program: &Path,
    group: u32,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> io::Result<Vec<GroupMember>> {
    let deadline = deadline.min(Instant::now() + SNAPSHOT_TIMEOUT);
    let mut members: Vec<_> = parse_members(&list_until(
        program,
        MEMBERSHIP_COLUMNS,
        Some(group),
        deadline,
        cancelled,
        None,
    )?)?
    .into_iter()
    .filter(|row| row.pgid == group)
    .collect();
    members.sort_by_key(|row| row.pid);
    Ok(members)
}

/// Every process `ps` lists, ordered by ID.
pub fn processes() -> io::Result<Vec<ProcessRow>> {
    let mut rows = parse(&list(Path::new(PS))?)?;
    rows.sort_by_key(|row| row.pid);
    Ok(rows)
}

/// Diagnostic text for `root`'s tree. A snapshot failure becomes
/// `snapshot unavailable: <reason>` so callers can append it to an original
/// error without ever replacing that error.
pub fn describe(root: u32) -> String {
    describe_with(Path::new(PS), root)
}

/// [`describe`] through an explicit `ps` executable.
pub fn describe_with(program: &Path, root: u32) -> String {
    match tree_with(program, root) {
        Ok(rows) => describe_rows(root, &rows),
        Err(error) => format!("snapshot unavailable: {error}"),
    }
}

/// Diagnostic text for rows a caller already recorded with [`tree`].
pub fn describe_rows(root: u32, rows: &[ProcessRow]) -> String {
    let mut text = format!("process tree of {root} ({} rows): [", rows.len());
    for (index, row) in rows.iter().take(ROW_LIMIT).enumerate() {
        if index > 0 {
            text.push_str("; ");
        }
        text.push_str(&row.to_string());
    }
    if rows.len() > ROW_LIMIT {
        text.push_str(&format!("; {} more omitted", rows.len() - ROW_LIMIT));
    }
    text.push(']');
    text
}

/// Which of the `recorded` rows a fresh listing still shows, as text. A row
/// matches on process ID and command, so a reused ID running another program is
/// not reported as a survivor. The result is a diagnostic reading, never a
/// basis for signalling: a coincidental match can only mislabel a row.
pub fn describe_still_listed(recorded: &[ProcessRow]) -> String {
    describe_still_listed_with(Path::new(PS), recorded)
}

/// [`describe_still_listed`] through an explicit `ps` executable.
/// The `recorded` rows a fresh listing still shows, matched on process ID and
/// command as in [`describe_still_listed`]. Never a basis for signalling.
pub fn still_listed(recorded: &[ProcessRow]) -> io::Result<Vec<ProcessRow>> {
    still_listed_with(Path::new(PS), recorded)
}

/// [`still_listed`] through an explicit `ps` executable.
pub fn still_listed_with(program: &Path, recorded: &[ProcessRow]) -> io::Result<Vec<ProcessRow>> {
    Ok(retain_listed(parse(&list(program)?)?, recorded))
}

pub fn describe_still_listed_with(program: &Path, recorded: &[ProcessRow]) -> String {
    match list(program).and_then(|output| parse(&output)) {
        Ok(rows) => {
            let live = retain_listed(rows, recorded);
            if live.is_empty() {
                format!(
                    "none of {} recorded processes remain listed",
                    recorded.len()
                )
            } else {
                format!(
                    "{} of {} recorded processes remain listed: [{}]",
                    live.len(),
                    recorded.len(),
                    live.iter()
                        .take(ROW_LIMIT)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            }
        }
        Err(error) => format!("snapshot unavailable: {error}"),
    }
}

fn retain_listed(listed: Vec<ProcessRow>, recorded: &[ProcessRow]) -> Vec<ProcessRow> {
    let mut live: Vec<_> = listed
        .into_iter()
        .filter(|row| {
            recorded
                .iter()
                .any(|before| before.pid == row.pid && before.command == row.command)
        })
        .collect();
    live.sort_by_key(|row| row.pid);
    live
}

fn members_of(rows: Vec<ProcessRow>, group: u32) -> Vec<ProcessRow> {
    let mut members: Vec<_> = rows.into_iter().filter(|row| row.pgid == group).collect();
    members.sort_by_key(|row| row.pid);
    members
}

fn select(rows: Vec<ProcessRow>, root: u32) -> Vec<ProcessRow> {
    let mut members: BTreeSet<u32> = rows
        .iter()
        .filter(|row| row.pid == root || row.pgid == root)
        .map(|row| row.pid)
        .collect();
    loop {
        let before = members.len();
        members.extend(
            rows.iter()
                .filter(|row| members.contains(&row.ppid))
                .map(|row| row.pid)
                .collect::<Vec<_>>(),
        );
        if members.len() == before {
            break;
        }
    }
    let mut selected: Vec<_> = rows
        .into_iter()
        .filter(|row| members.contains(&row.pid))
        .collect();
    selected.sort_by_key(|row| row.pid);
    selected
}

fn parse_members(output: &str) -> io::Result<Vec<GroupMember>> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let invalid = || {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid ps membership row: {line:?}"),
                )
            };
            if fields.len() != 3 {
                return Err(invalid());
            }
            Ok(GroupMember {
                pid: fields[0].parse().map_err(|_| invalid())?,
                pgid: fields[1].parse().map_err(|_| invalid())?,
                state: fields[2].into(),
            })
        })
        .collect()
}

fn parse(output: &str) -> io::Result<Vec<ProcessRow>> {
    let mut rows = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let mut next = |name: &str| {
            fields
                .next()
                .ok_or_else(|| io::Error::other(format!("ps row lacks {name}: {line:?}")))
        };
        let number = |name: &str, value: &str| {
            value
                .parse::<u64>()
                .map_err(|_| io::Error::other(format!("ps row has invalid {name}: {line:?}")))
        };
        let pid = number("pid", next("pid")?)?;
        let ppid = number("ppid", next("ppid")?)?;
        let pgid = number("pgid", next("pgid")?)?;
        // macOS prints IDs above i32::MAX, such as nobody's, as negative
        // numbers; uid_t is the same 32 bits either way.
        let user = |name: &str, value: &str| {
            value
                .parse::<u32>()
                .ok()
                .or_else(|| value.parse::<i32>().ok().map(|id| id as u32))
                .ok_or_else(|| io::Error::other(format!("ps row has invalid {name}: {line:?}")))
        };
        let uid = user("uid", next("uid")?)?;
        let ruid = user("ruid", next("ruid")?)?;
        let state = next("stat")?.to_owned();
        let cpu_time = next("time")?.to_owned();
        let rss_kib = number("rss", next("rss")?)?;
        let mut command = fields.collect::<Vec<_>>().join(" ");
        if command.len() > COMMAND_LIMIT {
            let mut end = COMMAND_LIMIT;
            while !command.is_char_boundary(end) {
                end -= 1;
            }
            command.truncate(end);
            command.push_str("...");
        }
        let id = |value: u64, name: &str| {
            u32::try_from(value)
                .map_err(|_| io::Error::other(format!("ps row {name} exceeds range: {line:?}")))
        };
        rows.push(ProcessRow {
            pid: id(pid, "pid")?,
            ppid: id(ppid, "ppid")?,
            pgid: id(pgid, "pgid")?,
            uid,
            ruid,
            state,
            cpu_time,
            rss_kib,
            command,
        });
    }
    Ok(rows)
}

fn list(program: &Path) -> io::Result<String> {
    list_within(program, SNAPSHOT_TIMEOUT)
}

fn list_within(program: &Path, timeout: Duration) -> io::Result<String> {
    list_until(
        program,
        DIAGNOSTIC_COLUMNS,
        None,
        Instant::now() + timeout,
        &AtomicBool::new(false),
        Some(timeout),
    )
}

fn admitted(deadline: Instant, cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "ps inspection cancelled",
        ));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "no time left to run ps",
        ));
    }
    Ok(())
}

fn list_until(
    program: &Path,
    columns: &str,
    group: Option<u32>,
    deadline: Instant,
    cancelled: &AtomicBool,
    relative_timeout: Option<Duration>,
) -> io::Result<String> {
    list_until_with_readers(
        program,
        columns,
        group,
        deadline,
        cancelled,
        relative_timeout,
        start_reader,
    )
}

type ReaderJob = thread::JoinHandle<(io::Result<()>, Vec<u8>)>;
type ReaderPipe = Option<Box<dyn Read + Send>>;

fn start_reader(
    pipe: ReaderPipe,
    stream: &'static str,
    sender: mpsc::Sender<()>,
) -> io::Result<ReaderJob> {
    thread::Builder::new()
        .name(format!("kuru-ps-{stream}"))
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = match pipe {
                Some(pipe) => pipe
                    .take(OUTPUT_LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .map(|_| ()),
                None => Err(io::Error::other(format!("missing ps {stream}"))),
            };
            let _ = sender.send(());
            (result, bytes)
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HelperOperation {
    Poll,
    Kill,
    Wait,
}

fn helper_observation<T>(armed: &mut bool, result: io::Result<T>) -> io::Result<T> {
    if let Err(error) = &result
        && error.kind() != io::ErrorKind::Interrupted
    {
        *armed = false;
    }
    result
}

fn stop_helper_with<T>(
    armed: &mut bool,
    mut operate: impl FnMut(HelperOperation) -> io::Result<Option<T>>,
) -> io::Result<Option<T>> {
    if !*armed {
        return Ok(None);
    }
    match helper_observation(armed, operate(HelperOperation::Poll)) {
        Ok(Some(status)) => return Ok(Some(status)),
        Ok(None) => {
            let _ = operate(HelperOperation::Kill);
        }
        // Interrupted observation grants no signal. Retain exact wait/drains
        // on this worker until the helper naturally finishes instead.
        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
        Err(error) => return Err(error),
    }
    helper_observation(armed, operate(HelperOperation::Wait))
}

fn stop_helper(child: &mut Child, armed: &mut bool) -> io::Result<Option<ExitStatus>> {
    stop_helper_with(armed, |operation| match operation {
        HelperOperation::Poll => child.try_wait(),
        HelperOperation::Kill => child.kill().map(|_| None),
        HelperOperation::Wait => child.wait().map(Some),
    })
}

fn list_until_with_readers(
    program: &Path,
    columns: &str,
    group: Option<u32>,
    deadline: Instant,
    cancelled: &AtomicBool,
    relative_timeout: Option<Duration>,
    mut reader: impl FnMut(ReaderPipe, &'static str, mpsc::Sender<()>) -> io::Result<ReaderJob>,
) -> io::Result<String> {
    admitted(deadline, cancelled)?;
    let timeout =
        relative_timeout.unwrap_or_else(|| deadline.saturating_duration_since(Instant::now()));
    let mut command = Command::new(program);
    #[cfg(target_os = "macos")]
    if let Some(group) = group {
        // Apple's -g selects process groups only in UNIX2003 mode. -x keeps
        // no-terminal members; no UID selector is added. Never combine -A,
        // whose OR selection would restore the full-host traversal.
        command
            .args(["-g", &group.to_string(), "-x"])
            .env("COMMAND_MODE", "unix2003");
    } else {
        command.arg("-A");
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = group;
        command.arg("-A");
    }
    command
        .args(["-o", columns])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // std creates these pipes inside spawn; under the platform spawn lock no
    // owned child can copy them before they are close-on-exec.
    let spawning = if relative_timeout.is_none() {
        match super::SPAWN.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "platform spawn is busy",
                ));
            }
        }
    } else {
        super::spawn_lock()
    };
    if relative_timeout.is_none() {
        admitted(deadline, cancelled)?;
    }
    let spawned = command.spawn();
    drop(spawning);
    let mut child = spawned
        .map_err(|error| io::Error::new(error.kind(), format!("spawn {program:?}: {error}")))?;
    let mut armed = true;
    let (sender, receiver) = mpsc::channel();
    let stdout = reader(
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
        "stdout",
        sender.clone(),
    );
    let stdout = match stdout {
        Ok(reader) => reader,
        Err(error) => {
            drop(child.stderr.take());
            let _ = stop_helper(&mut child, &mut armed);
            return Err(error);
        }
    };
    let stderr = reader(
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
        "stderr",
        sender.clone(),
    );
    let stderr = match stderr {
        Ok(reader) => reader,
        Err(error) => {
            let _ = stop_helper(&mut child, &mut armed);
            let _ = stdout.join();
            return Err(error);
        }
    };
    drop(sender);
    // Both readers report EOF through the channel; the root is polled between
    // those bounded receives, and at the same interval once both have ended.
    // Preserve the existing post-spawn allowance for relative diagnostics.
    // The pre-reap path never refreshes its absolute inspection deadline.
    let deadline = relative_timeout.map_or(deadline, |timeout| Instant::now() + timeout);
    let mut finished = 0;
    let mut status = None;
    let mut observation_error = None;
    loop {
        if status.is_none() {
            match helper_observation(&mut armed, child.try_wait()) {
                Ok(observed) => status = observed,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    observation_error = Some(error);
                    break;
                }
            }
        }
        if finished == 2 && status.is_some() {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || cancelled.load(Ordering::Acquire) {
            break;
        }
        let interval = remaining.min(Duration::from_millis(10));
        if finished < 2 {
            if receiver.recv_timeout(interval).is_ok() {
                finished += 1;
            }
        } else {
            thread::sleep(interval);
        }
    }
    let timed_out = status.is_none() || finished < 2 || Instant::now() >= deadline;
    if status.is_none() {
        // The owned child handle, not a numeric ID, stops the helper; `ps`
        // starts no descendants, so its pipes close once it is reaped.
        // Retain all helper cleanup even on a polling error. Joining readers
        // happens only here on the observation worker, never on its caller.
        match stop_helper(&mut child, &mut armed) {
            Ok(reaped) => status = reaped,
            Err(error) => observation_error = Some(error),
        }
    }
    let (stdout_result, stdout_bytes) = stdout
        .join()
        .map_err(|_| io::Error::other("ps stdout reader panicked"))?;
    let (stderr_result, stderr_bytes) = stderr
        .join()
        .map_err(|_| io::Error::other("ps stderr reader panicked"))?;
    if let Some(error) = observation_error {
        return Err(error);
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "ps inspection cancelled",
        ));
    }
    if timed_out || Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{program:?} did not finish within {timeout:?}"),
        ));
    }
    stdout_result?;
    stderr_result?;
    if stdout_bytes.len() as u64 > OUTPUT_LIMIT {
        return Err(io::Error::other("ps output exceeds limit"));
    }
    let status = status.expect("status is observed before readers are joined");
    if !status.success() {
        return Err(io::Error::other(format!(
            "{program:?} failed with {status}: {}",
            String::from_utf8_lossy(&stderr_bytes[..stderr_bytes.len().min(1024)]).trim()
        )));
    }
    if relative_timeout.is_none() && !stderr_bytes.is_empty() {
        return Err(io::Error::other(format!(
            "{program:?} reported an inspection error: {}",
            String::from_utf8_lossy(&stderr_bytes[..stderr_bytes.len().min(1024)]).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&stdout_bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_helper_wait_authority_never_kills_or_waits_again() {
        for code in [
            rustix::io::Errno::CHILD.raw_os_error(),
            rustix::io::Errno::IO.raw_os_error(),
        ] {
            let mut armed = true;
            let mut operations = Vec::new();
            let error = stop_helper_with::<()>(&mut armed, |operation| {
                operations.push(operation);
                Err(io::Error::from_raw_os_error(code))
            })
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(code));
            assert!(!armed);
            assert_eq!(operations, [HelperOperation::Poll]);
            let result = stop_helper_with::<()>(&mut armed, |_| panic!("disarmed helper syscall"));
            assert!(result.unwrap().is_none());
        }
        let mut armed = true;
        let mut operations = Vec::new();
        let result = stop_helper_with(&mut armed, |operation| {
            operations.push(operation);
            if operation == HelperOperation::Poll {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok(Some(()))
            }
        });
        assert_eq!(result.unwrap(), Some(()));
        assert!(armed);
        assert_eq!(operations, [HelperOperation::Poll, HelperOperation::Wait]);
    }

    #[test]
    fn partial_reader_start_failure_reaps_the_owned_helper() {
        for failed_stream in ["stdout", "stderr"] {
            let root = tempfile::tempdir().unwrap();
            let program = root.path().join("reader-failure-ps");
            let marker = root.path().join("helper-id");
            let script = format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec /bin/sleep 30\n",
                marker.display()
            );
            let writer = Command::new("/bin/sh")
                .args([
                    "-c",
                    "printf '%s' \"$2\" > \"$1\" && chmod 755 \"$1\"",
                    "kuru-ps-reader-fixture",
                ])
                .arg(&program)
                .arg(script)
                .status()
                .unwrap();
            assert!(writer.success());
            let mut helper = None;
            let mut started = Vec::new();
            let result = list_until_with_readers(
                &program,
                MEMBERSHIP_COLUMNS,
                Some(20),
                Instant::now() + super::super::tests::TEST_BOUND,
                &AtomicBool::new(false),
                None,
                |pipe, stream, sender| {
                    if stream == failed_stream {
                        let limit = Instant::now() + super::super::tests::TEST_BOUND;
                        while !marker.exists() && Instant::now() < limit {
                            thread::sleep(Duration::from_millis(2));
                        }
                        helper = std::fs::read_to_string(&marker)
                            .ok()
                            .and_then(|id| id.parse::<i32>().ok());
                        drop(pipe);
                        Err(io::Error::other("injected reader startup failure"))
                    } else {
                        let reader = start_reader(pipe, stream, sender)?;
                        started.push(stream);
                        Ok(reader)
                    }
                },
            );
            assert_eq!(
                result.unwrap_err().to_string(),
                "injected reader startup failure"
            );
            let helper =
                rustix::process::Pid::from_raw(helper.expect("helper never published readiness"))
                    .unwrap();
            let reaped = rustix::process::waitid(
                rustix::process::WaitId::Pid(helper),
                rustix::process::WaitIdOptions::EXITED
                    | rustix::process::WaitIdOptions::NOHANG
                    | rustix::process::WaitIdOptions::NOWAIT,
            );
            assert_eq!(
                reaped.unwrap_err(),
                rustix::io::Errno::CHILD,
                "helper was not reaped after {failed_stream} startup failure"
            );
            assert_eq!(started.len(), usize::from(failed_stream == "stderr"));
        }
    }

    fn row(pid: u32, ppid: u32, pgid: u32) -> ProcessRow {
        ProcessRow {
            pid,
            ppid,
            pgid,
            uid: 501,
            ruid: 501,
            state: "S".to_owned(),
            cpu_time: "0:00.00".to_owned(),
            rss_kib: 1,
            command: format!("process {pid}"),
        }
    }

    #[test]
    fn selection_follows_group_and_parent_closure() {
        let rows = vec![
            row(1, 0, 1),
            row(10, 1, 10),
            // Reparented to init after its parent exited; kept by its group.
            row(12, 1, 10),
            // Left the group; kept by its parent.
            row(13, 12, 13),
            row(14, 13, 13),
            row(20, 1, 20),
        ];
        let pids: Vec<_> = select(rows, 10).iter().map(|row| row.pid).collect();
        assert_eq!(pids, [10, 12, 13, 14]);
    }

    #[test]
    fn parse_keeps_spaced_arguments_and_rejects_malformed_rows() {
        let rows =
            parse("  7   1   7  501     0 Ss   0:01.50  2048 /bin/sh -c sleep 60\n\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].uid, rows[0].ruid), (501, 0));
        let nobody = parse("854 1 854 -2 -2 Ss 0:00 1 dnsmasq\n").unwrap();
        assert_eq!(
            (nobody[0].uid, nobody[0].ruid),
            (u32::MAX - 1, u32::MAX - 1)
        );
        assert!(parse("7 1 7 4294967296 0 S 0:00 1 cmd\n").is_err());
        assert_eq!(rows[0].command, "/bin/sh -c sleep 60");
        assert_eq!(rows[0].rss_kib, 2048);
        assert!(parse("7 1\n").is_err());
        assert!(parse("x 1 7 0 0 S 0:00 1 cmd\n").is_err());
        assert!(parse("7 1 7 root 0 S 0:00 1 cmd\n").is_err());
        let long = format!("8 1 8 0 0 S 0:00 1 {}", "é".repeat(COMMAND_LIMIT));
        assert!(parse(&long).unwrap()[0].command.ends_with("..."));
    }

    #[test]
    fn minimal_membership_keeps_state_and_rejects_incomplete_or_extra_fields() {
        let rows = parse_members(" 7 7 Z+\n 8 7 S\n 9 9 R\n\n").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[0].pid, rows[0].pgid, rows[0].state.as_str()),
            (7, 7, "Z+")
        );
        for malformed in ["7 7", "7 7 Z extra", "x 7 S", "7 4294967296 S"] {
            assert_eq!(
                parse_members(malformed).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn group_listing_without_time_left_runs_nothing() {
        // A program that cannot exist proves nothing was spawned.
        let error =
            group_members_within_with(Path::new("/nonexistent/kuru-ps"), 20, Duration::ZERO)
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
        assert!(error.to_string().contains("no time left"), "{error}");
        let error = group_members_with(Path::new("/nonexistent/kuru-ps"), 20).unwrap_err();
        assert!(error.to_string().contains("spawn"), "{error}");
    }

    #[test]
    fn delayed_worker_and_cancelled_admission_never_spawn() {
        for cancel in [false, true] {
            let (release, wait) = mpsc::channel();
            let cancelled = std::sync::Arc::new(AtomicBool::new(cancel));
            let cancellation = cancelled.clone();
            let deadline = Instant::now();
            let worker = thread::spawn(move || {
                wait.recv().unwrap();
                group_members_until(
                    Path::new("/nonexistent/kuru-delayed-ps"),
                    20,
                    deadline,
                    &cancellation,
                )
            });
            release.send(()).unwrap();
            let error = worker.join().unwrap().unwrap_err();
            assert_eq!(
                error.kind(),
                if cancel {
                    io::ErrorKind::Interrupted
                } else {
                    io::ErrorKind::TimedOut
                }
            );
            assert!(!error.to_string().contains("spawn"), "{error}");
        }
    }

    #[test]
    fn group_members_follow_only_the_numeric_group() {
        let rows = vec![
            row(30, 1, 20),
            row(20, 1, 20),
            row(21, 20, 21),
            row(9, 1, 9),
        ];
        let pids: Vec<_> = members_of(rows, 20).iter().map(|row| row.pid).collect();
        assert_eq!(pids, [20, 30]);
    }

    #[test]
    fn still_listed_matches_recorded_processes_by_id_and_command() {
        let recorded = vec![row(10, 1, 10), row(11, 10, 10), row(12, 10, 10)];
        let mut reused = row(12, 1, 12);
        reused.command = "another program".to_owned();
        let listed = vec![row(11, 1, 10), reused, row(99, 1, 99)];
        let live: Vec<_> = retain_listed(listed, &recorded)
            .iter()
            .map(|row| row.pid)
            .collect();
        assert_eq!(live, [11]);
    }

    #[test]
    fn formatting_bounds_rows() {
        let rows: Vec<_> = (1..=ROW_LIMIT as u32 + 2)
            .map(|pid| row(pid, 0, 1))
            .collect();
        let text = describe_rows(1, &rows);
        assert!(text.starts_with(&format!("process tree of 1 ({} rows): [", rows.len())));
        assert!(text.ends_with("; 2 more omitted]"), "{text}");
    }
}
