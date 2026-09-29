//! Outside observation of one run: Kuru and Dolt processes under the scratch
//! root (sysinfo), and names appearing and vanishing in its data and cache
//! directories.
//!
//! Directories are enumerated; files are never opened. Enumerating opens a
//! directory handle for the length of one listing, and on Windows an open
//! handle inside a directory's tree can make renaming or removing that
//! directory fail. So the observer never enters an engine install stage
//! (`.install-*`), a store staging directory (`*.staging-*`) or `interrupted`,
//! the directories the product renames or removes during an open: their
//! appearance and disappearance are timed from the parent listing only. Each
//! listing's names are collected and its handle closed before any child is
//! listed, and Dolt's chunk store, statistics and temporary directories are
//! skipped. The directories still listed (`cache`, a version directory, the
//! activated engine directory, `data`, `data/memory`, the active store,
//! `services`, `locks`) are not renamed or removed by a successful open.
//!
//! Sampling has a fixed period: the sampler thread sleeps for the rest of each
//! interval. Every tick stamps its start, before the process listing, and its
//! end, after the file walk. A process or name first seen in tick k appeared
//! after the start of tick k-1 and no later than the end of tick k:
//! `first_seen_ms` and `appeared_by_ms` are that end stamp, and the earlier
//! start stamp is kept beside it. The widest such bracket of a run is its
//! `max_bracket_ms`. Processes shorter than one interval can be missed.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, Uid, UpdateKind};

/// Names never listed: Dolt's chunk store, statistics and temporary
/// directories, and Kuru's diagnostics ring, change on every write and carry
/// no stage; Dolt removes its temporary directories during a run.
const SKIPPED: [&str; 6] = [
    "noms",
    "stats",
    "temptf",
    "tmp",
    "eventsData",
    "diagnostics",
];
/// Deep enough for `data/memory/<store>/data/.dolt/sql-server.info` and
/// `cache/<version>/<target>/dolt`.
const DEPTH: usize = 4;

/// A directory the observer lists from its parent but never enters: an
/// engine install stage, a store staging directory and the preserved
/// interrupted stages. The product renames or removes these during an open.
pub fn unentered(name: &str) -> bool {
    name.starts_with(".install-") || name.contains(".staging-") || name == "interrupted"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Cli,
    Owner,
    Supervisor,
    SqlServer,
    DoltVersion,
    OtherDolt,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Owner => "owner",
            Self::Supervisor => "supervisor",
            Self::SqlServer => "sql-server",
            Self::DoltVersion => "dolt-version",
            Self::OtherDolt => "other-dolt",
        }
    }
}

/// The role of a `kuru` or `dolt` process, from its image stem and arguments
/// (without argv[0]). Any other image is not Kuru's.
pub fn classify(stem: &str, args: &[String]) -> Option<Role> {
    match stem {
        "kuru" => Some(
            if args.iter().any(|arg| arg == "--internal-memory-service") {
                Role::Owner
            } else if args.iter().any(|arg| arg == "--internal-dolt-supervisor") {
                Role::Supervisor
            } else {
                Role::Cli
            },
        ),
        "dolt" => Some(match args.first().map(String::as_str) {
            Some("sql-server") => Role::SqlServer,
            Some("version") => Role::DoltVersion,
            _ => Role::OtherDolt,
        }),
        _ => None,
    }
}

/// Which store an engine serves: a staging directory or the active store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Store {
    Staging,
    Active,
}

impl Store {
    pub fn of(args: &[String]) -> Option<Self> {
        let config = args
            .windows(2)
            .find(|pair| pair[0] == "--config")
            .map(|pair| pair[1].as_str())?;
        Some(if config.contains(".staging-") {
            Self::Staging
        } else {
            Self::Active
        })
    }
}

/// A root-relative name with its run-specific parts replaced, so one key
/// names the same thing in every run: a 64-hex project hash, a UUID (stage
/// directories, staged records, retired endpoints), an install directory's
/// random suffix and a numeric temporary suffix.
pub fn normalize(relative: &str) -> String {
    relative
        .split('/')
        .map(|segment| {
            let (hash, rest) = match segment.get(..64) {
                Some(prefix) if prefix.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
                    ("<project>", &segment[64..])
                }
                _ => ("", segment),
            };
            let rest = replace_uuids(rest);
            let rest = match rest.strip_prefix(".install-") {
                Some(suffix) if !suffix.is_empty() && suffix != "lock" => {
                    ".install-<tmp>".to_owned()
                }
                _ => rest,
            };
            let rest = match rest.rsplit_once('-') {
                Some((stem, digits))
                    if digits.len() >= 4 && digits.bytes().all(|byte| byte.is_ascii_digit()) =>
                {
                    format!("{stem}-<n>")
                }
                _ => rest,
            };
            format!("{hash}{rest}")
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_uuid(candidate: &[u8]) -> bool {
    candidate.len() == 36
        && candidate.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn replace_uuids(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes.get(index..index + 36).is_some_and(is_uuid) {
            out.push_str("<id>");
            index += 36;
        } else {
            let character = segment[index..].chars().next().unwrap_or_default();
            out.push(character);
            index += character.len_utf8().max(1);
        }
    }
    out
}

/// One observed process, times in milliseconds from the run's epoch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessSpan {
    pub pid: u32,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<Store>,
    /// Alive at the first tick, before the measured command started.
    pub preexisting: bool,
    /// The role it had when first seen, if it changed at exec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_before_exec_as: Option<Role>,
    /// End of the first tick that listed it: it started no later.
    pub first_seen_ms: f64,
    /// Start of the previous tick, which did not list it: it started later.
    pub not_seen_ms: Option<f64>,
    /// Start of the last tick that listed it: it was still alive then.
    pub last_seen_ms: f64,
    /// End of the first tick without it; absent while still alive.
    pub gone_by_ms: Option<f64>,
    /// Ticks that listed it.
    pub samples: u64,
}

impl ProcessSpan {
    /// Lifetime between two sightings: a lower bound, absent for a process
    /// listed by one tick only.
    pub fn lifetime_lower_ms(&self) -> Option<f64> {
        (self.samples >= 2).then(|| round(self.last_seen_ms - self.first_seen_ms).max(0.0))
    }

    /// Lifetime from the last tick without it to the first tick without it
    /// again: an upper bound.
    pub fn lifetime_upper_ms(&self) -> Option<f64> {
        Some(round(self.gone_by_ms? - self.not_seen_ms?))
    }
}

/// One appearance of a normalised name.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileSpan {
    pub key: String,
    /// End of the first tick that listed it.
    pub appeared_by_ms: f64,
    /// Start of the previous tick without it; absent when it existed at the
    /// first tick.
    pub absent_at_ms: Option<f64>,
    /// Start of the last tick that listed it.
    #[serde(default)]
    pub last_present_ms: Option<f64>,
    /// End of the first tick without it again.
    pub vanished_by_ms: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Observation {
    pub processes: Vec<ProcessSpan>,
    pub files: Vec<FileSpan>,
    /// False in the control mode, which lists no files at all.
    #[serde(default)]
    pub files_observed: bool,
    pub ticks: u64,
    pub interval_ms: f64,
    /// Largest start-to-start gap between two ticks.
    pub max_gap_ms: f64,
    /// Largest start of one tick to end of the next: every sampled time of
    /// the run is late by at most this much.
    #[serde(default)]
    pub max_bracket_ms: f64,
    pub mean_tick_cost_ms: f64,
}

/// A Kuru or Dolt process visible to this user.
#[derive(Clone, Debug)]
pub struct Seen {
    pub pid: u32,
    pub role: Role,
    pub store: Option<Store>,
    /// Running from the scratch root.
    pub ours: bool,
    /// `kuru` or `dolt`, without a path or `.exe`.
    pub image: String,
    /// Owned by this user; absent when the owner could not be read. Read only
    /// by [`Processes::census`].
    pub mine: Option<bool>,
    /// Open file descriptors, or handles on Windows, of a process from the
    /// scratch root. Read only by [`Processes::census`].
    pub open_files: Option<usize>,
}

/// Incremental process listing. Executable paths and arguments of Kuru and
/// Dolt processes are read again on every listing.
pub struct Processes {
    system: System,
    root: PathBuf,
    me: Option<Uid>,
}

impl Processes {
    pub fn new(root: &Path) -> Self {
        let mut system = System::new();
        let me = sysinfo::get_current_pid().ok().and_then(|pid| {
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                false,
                ProcessRefreshKind::nothing()
                    .without_tasks()
                    .with_user(UpdateKind::Always),
            );
            system
                .process(pid)
                .and_then(|process| process.user_id().cloned())
        });
        Self {
            system,
            root: comparable(root),
            me,
        }
    }

    /// Every Kuru and Dolt process, as the sampler lists them each tick.
    pub fn list(&mut self) -> Vec<Seen> {
        self.listing(false)
    }

    /// The same listing with each process's owner and, for processes from the
    /// scratch root, its open file or handle count. Slower; taken only before
    /// and after a run.
    pub fn census(&mut self) -> Vec<Seen> {
        self.listing(true)
    }

    fn listing(&mut self, detail: bool) -> Vec<Seen> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        // An image and its arguments are otherwise read once, and a process
        // first listed between fork and exec (or whose name the platform
        // caches) would keep its parent's. Read them again on every sample
        // for the few Kuru and Dolt processes.
        let candidates: Vec<Pid> = self
            .system
            .processes()
            .iter()
            .filter(|(_, process)| {
                is_kuru_or_dolt(&stem(process.name()))
                    || process
                        .exe()
                        .and_then(Path::file_name)
                        .is_some_and(|image| is_kuru_or_dolt(&stem(image)))
            })
            .map(|(pid, _)| *pid)
            .collect();
        if !candidates.is_empty() {
            let mut refresh = ProcessRefreshKind::nothing()
                .without_tasks()
                .with_exe(UpdateKind::Always)
                .with_cmd(UpdateKind::Always);
            if detail {
                refresh = refresh.with_user(UpdateKind::OnlyIfNotSet);
            }
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&candidates),
                false,
                refresh,
            );
        }
        let mut seen = Vec::new();
        for pid in candidates {
            let Some(process) = self.system.process(pid) else {
                continue;
            };
            // The image, not the process name, which may be stale or truncated.
            let image = process
                .exe()
                .map(Path::to_path_buf)
                .or_else(|| process.cmd().first().map(PathBuf::from));
            let stem = image
                .as_deref()
                .and_then(Path::file_name)
                .map_or_else(|| stem(process.name()), stem);
            if !is_kuru_or_dolt(&stem) {
                continue;
            }
            let cmd: Vec<String> = process
                .cmd()
                .iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            let arguments = cmd.get(1..).unwrap_or_default();
            let Some(role) = classify(&stem, arguments) else {
                continue;
            };
            let ours = image.is_some_and(|image| comparable(&image).starts_with(&self.root));
            let mine = match (detail, &self.me, process.user_id()) {
                (true, Some(me), Some(user)) => Some(me == user),
                _ => None,
            };
            seen.push(Seen {
                pid: pid_number(pid),
                role,
                store: (role == Role::SqlServer)
                    .then(|| Store::of(arguments))
                    .flatten(),
                ours,
                image: stem,
                mine,
                open_files: (detail && ours).then(|| process.open_files()).flatten(),
            });
        }
        seen.sort_by_key(|seen| seen.pid);
        seen
    }
}

fn is_kuru_or_dolt(stem: &str) -> bool {
    stem == "kuru" || stem == "dolt"
}

/// An image or process name, case-folded and without `.exe`.
fn stem(name: &std::ffi::OsStr) -> String {
    let name = name.to_string_lossy().to_ascii_lowercase();
    name.strip_suffix(".exe")
        .map_or_else(|| name.clone(), str::to_owned)
}

fn pid_number(pid: Pid) -> u32 {
    pid.as_u32()
}

/// A path comparable by prefix: without a Windows verbatim prefix, and
/// case-folded on Windows.
fn comparable(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if cfg!(windows) {
        PathBuf::from(text.replace('/', "\\").to_lowercase())
    } else {
        PathBuf::from(text)
    }
}

/// How long the root's processes took to retire after a run.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Retirement {
    /// Until no process runs from the root and no service endpoint is
    /// published.
    pub wait_ms: f64,
    /// Until the first poll without a memory owner process from the root.
    pub owner_retired_ms: f64,
}

/// Wait until no process runs from the root and no `<project>/endpoint.json`
/// is published under `services`, polling every `poll`. A bound is an
/// infrastructure failure that names what is still alive. Nothing is killed.
pub async fn wait_retired(
    processes: &mut Processes,
    services: &Path,
    bound: Duration,
    poll: Duration,
) -> Result<Retirement> {
    let started = Instant::now();
    let mut owner_retired_ms = None;
    loop {
        let alive: Vec<Seen> = processes
            .list()
            .into_iter()
            .filter(|seen| seen.ours)
            .collect();
        if owner_retired_ms.is_none() && !alive.iter().any(|seen| seen.role == Role::Owner) {
            owner_retired_ms = Some(millis(started.elapsed()));
        }
        let published = published(services);
        if alive.is_empty() && !published {
            let wait_ms = millis(started.elapsed());
            return Ok(Retirement {
                wait_ms,
                owner_retired_ms: owner_retired_ms.unwrap_or(wait_ms),
            });
        }
        if started.elapsed() >= bound {
            let roles: Vec<String> = alive
                .iter()
                .map(|seen| format!("{} {}", seen.role.label(), seen.pid))
                .collect();
            bail!(
                "processes under the scratch root did not retire within {} s: [{}]; service endpoint present: {published}",
                bound.as_secs(),
                roles.join(", ")
            );
        }
        // The measurement tool's poll for process exit; not a product wait.
        tokio::time::sleep(poll).await;
    }
}

pub fn millis(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 10_000.0).round() / 10.0
}

/// A difference rounded to the tenth of a millisecond the records keep.
pub fn round(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Samples processes and, unless disabled, files on its own thread until
/// finished.
pub struct Observer {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Observation>,
}

impl Observer {
    /// `watched` maps a key prefix (`data`, `cache`) to its directory; an
    /// empty list is the control mode, which lists no files.
    pub fn start(
        root: &Path,
        watched: Vec<(&'static str, PathBuf)>,
        epoch: Instant,
        interval: Duration,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let root = root.to_owned();
        let handle = std::thread::spawn(move || sample(&root, &watched, epoch, interval, &flag));
        Self { stop, handle }
    }

    pub fn finish(self) -> Observation {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap_or_default()
    }
}

fn sample(
    root: &Path,
    watched: &[(&'static str, PathBuf)],
    epoch: Instant,
    interval: Duration,
    stop: &AtomicBool,
) -> Observation {
    let mut processes = Processes::new(root);
    let mut open: HashMap<u32, ProcessSpan> = HashMap::new();
    let mut files: HashMap<String, FileSpan> = HashMap::new();
    let mut observation = Observation {
        interval_ms: millis(interval),
        files_observed: !watched.is_empty(),
        ..Observation::default()
    };
    let mut previous: Option<f64> = None;
    let mut cost = Duration::ZERO;
    loop {
        let finishing = stop.load(Ordering::SeqCst);
        let tick = Instant::now();
        let start = millis(tick.saturating_duration_since(epoch));
        let first = previous.is_none();
        let listed: Vec<Seen> = processes
            .list()
            .into_iter()
            .filter(|seen| seen.ours)
            .collect();
        let mut present = HashSet::new();
        for (prefix, directory) in watched {
            walk(directory, prefix, 0, &mut present);
        }
        // Stamped after both listings, so a first sighting's time is an
        // upper bound on when the process or name appeared.
        let end = millis(Instant::now().saturating_duration_since(epoch));
        let mut alive = HashSet::new();
        for seen in listed {
            alive.insert(seen.pid);
            open.entry(seen.pid)
                .and_modify(|span| {
                    span.last_seen_ms = start;
                    span.samples += 1;
                    // First seen between fork and exec: keep the fork as its
                    // start and take the role of the image it became.
                    if span.role != seen.role {
                        span.seen_before_exec_as.get_or_insert(span.role);
                        span.role = seen.role;
                        span.store = seen.store;
                    }
                })
                .or_insert_with(|| ProcessSpan {
                    pid: seen.pid,
                    role: seen.role,
                    store: seen.store,
                    preexisting: first,
                    seen_before_exec_as: None,
                    first_seen_ms: end,
                    not_seen_ms: previous,
                    last_seen_ms: start,
                    gone_by_ms: None,
                    samples: 1,
                });
        }
        let gone: Vec<u32> = open
            .keys()
            .filter(|pid| !alive.contains(pid))
            .copied()
            .collect();
        for pid in gone {
            if let Some(mut span) = open.remove(&pid) {
                span.gone_by_ms = Some(end);
                observation.processes.push(span);
            }
        }
        for key in &present {
            files
                .entry(key.clone())
                .and_modify(|span| span.last_present_ms = Some(start))
                .or_insert_with(|| FileSpan {
                    key: key.clone(),
                    appeared_by_ms: end,
                    absent_at_ms: previous,
                    last_present_ms: Some(start),
                    vanished_by_ms: None,
                });
        }
        let vanished: Vec<String> = files
            .keys()
            .filter(|key| !present.contains(*key))
            .cloned()
            .collect();
        for key in vanished {
            if let Some(mut span) = files.remove(&key) {
                span.vanished_by_ms = Some(end);
                observation.files.push(span);
            }
        }
        if let Some(previous) = previous {
            observation.max_gap_ms = observation.max_gap_ms.max(round(start - previous));
            observation.max_bracket_ms = observation.max_bracket_ms.max(round(end - previous));
        }
        previous = Some(start);
        observation.ticks += 1;
        let spent = tick.elapsed();
        cost += spent;
        if finishing {
            break;
        }
        // The sampler's period: the measurement tool's own pacing, reported
        // with every result as `interval_ms`.
        if let Some(rest) = interval.checked_sub(spent) {
            std::thread::sleep(rest);
        }
    }
    observation.processes.extend(open.into_values());
    observation.files.extend(files.into_values());
    observation.processes.sort_by(|left, right| {
        left.first_seen_ms
            .total_cmp(&right.first_seen_ms)
            .then(left.pid.cmp(&right.pid))
    });
    observation.files.sort_by(|left, right| {
        left.appeared_by_ms
            .total_cmp(&right.appeared_by_ms)
            .then_with(|| left.key.cmp(&right.key))
    });
    observation.mean_tick_cost_ms = if observation.ticks == 0 {
        0.0
    } else {
        round(millis(cost) / observation.ticks as f64)
    };
    observation
}

/// List `directory` and record every normalised name under `prefix`. The
/// listing's names are collected and its enumeration handle closed before
/// any child directory is listed; an [`unentered`] directory is recorded
/// but never listed.
pub fn walk(directory: &Path, prefix: &str, depth: usize, present: &mut HashSet<String>) {
    if depth > DEPTH {
        return;
    }
    let entries: Vec<(String, bool, PathBuf)> = match std::fs::read_dir(directory) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.file_type().is_ok_and(|kind| kind.is_dir()),
                    entry.path(),
                )
            })
            .collect(),
        Err(_) => return,
    };
    if depth == 0 {
        present.insert(prefix.to_owned());
    }
    for (name, is_directory, path) in entries {
        if SKIPPED.contains(&name.as_str()) {
            continue;
        }
        let raw = format!("{prefix}/{name}");
        present.insert(normalize(&raw));
        if is_directory && !unentered(&name) {
            walk(&path, &raw, depth + 1, present);
        }
    }
}

/// Whether any owner's service endpoint is published under `services`. The
/// listing is closed before any record is checked.
pub fn published(services: &Path) -> bool {
    let projects: Vec<PathBuf> = match std::fs::read_dir(services) {
        Ok(entries) => entries.flatten().map(|entry| entry.path()).collect(),
        Err(_) => return false,
    };
    projects
        .iter()
        .any(|project| project.join("endpoint.json").exists())
}
