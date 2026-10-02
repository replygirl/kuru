//! Read-only, bounded process-tree description for failure diagnostics.
//!
//! One stock `ps` invocation lists every process; the rows are filtered in Rust
//! to a root, the members of the process group it leads and the parent-ID
//! closure of both. A descendant reparented after its parent exited is still
//! found through its group. `ps -g` is avoided because its meaning differs
//! between BSD and procps.
//!
//! The result is point-in-time text. Numeric IDs are printed, never signalled,
//! waited for or otherwise acted on, so a reused ID can only mislabel a row.
//! Callers take the snapshot while they still anchor the root's identity, for
//! example before reaping it, and only on a failure path. The one exception is
//! [`group_members_within`], which a post-reap cleanup reads at most once, after
//! signal zero was refused with `EPERM` and within the time left before its
//! deadline, to tell a group recycled by another user from a survivor of ours.

use std::{
    collections::BTreeSet,
    fmt,
    io::{self, Read},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// Stock location on macOS and on usrmerged Linux distributions.
pub const PS: &str = "/bin/ps";

/// Bound for the complete `ps` run, including draining its pipes.
pub const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(2);

const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
const ROW_LIMIT: usize = 64;
const COMMAND_LIMIT: usize = 512;

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
    if timeout.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("no time left to run {program:?}"),
        ));
    }
    let mut child = Command::new(program)
        .args([
            "-A",
            "-o",
            "pid=,ppid=,pgid=,uid=,ruid=,stat=,time=,rss=,args=",
        ])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| io::Error::new(error.kind(), format!("spawn {program:?}: {error}")))?;
    let (sender, receiver) = mpsc::channel();
    let drain = |pipe: Option<Box<dyn Read + Send>>, stream: &'static str| {
        let sender = sender.clone();
        thread::spawn(move || {
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
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
        "stdout",
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
        "stderr",
    );
    drop(sender);
    // Both readers report EOF through the channel; the root is polled between
    // those bounded receives, and at the same interval once both have ended.
    let deadline = Instant::now() + timeout;
    let mut finished = 0;
    let mut status = None;
    loop {
        if status.is_none() {
            status = child.try_wait()?;
        }
        if finished == 2 && status.is_some() {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
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
    let timed_out = status.is_none() || finished < 2;
    if status.is_none() {
        // The owned child handle, not a numeric ID, stops the helper; `ps`
        // starts no descendants, so its pipes close once it is reaped.
        let _ = child.kill();
        status = Some(child.wait()?);
    }
    let (stdout_result, stdout_bytes) = stdout
        .join()
        .map_err(|_| io::Error::other("ps stdout reader panicked"))?;
    let (stderr_result, stderr_bytes) = stderr
        .join()
        .map_err(|_| io::Error::other("ps stderr reader panicked"))?;
    if timed_out {
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
    Ok(String::from_utf8_lossy(&stdout_bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
