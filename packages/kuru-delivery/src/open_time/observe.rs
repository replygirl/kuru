//! Outside observation of one run: Kuru and Dolt processes under the scratch
//! root (sysinfo), and names appearing and vanishing in its data and cache
//! directories. Files are only listed and never opened: the endpoint and
//! identity records are private handshake state.
//!
//! Sampling has a fixed interval, so every time here is bracketed by two
//! ticks: a process or file first seen at `t` appeared after the previous
//! tick. Processes shorter than one interval can be missed.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Names never walked: Dolt's chunk store and statistics, and Kuru's
/// diagnostics ring, change on every write and carry no stage.
const SKIPPED: [&str; 5] = ["noms", "stats", "temptf", "eventsData", "diagnostics"];
/// Deep enough for `cache/<version>/.install-*/private/probe/root/.dolt` and
/// `data/memory/<store>/data/.dolt/sql-server.info`.
const DEPTH: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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
#[derive(Clone, Debug, Serialize)]
pub struct ProcessSpan {
    pub pid: u32,
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<Store>,
    /// Alive at the first tick, before the measured command started.
    pub preexisting: bool,
    /// The role it had when first seen, if it changed at exec.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen_before_exec_as: Option<Role>,
    pub first_seen_ms: f64,
    /// The previous tick, at which it was not yet seen.
    pub not_seen_ms: Option<f64>,
    pub last_seen_ms: f64,
    /// The first tick at which it was gone; absent while still alive.
    pub gone_by_ms: Option<f64>,
}

/// One appearance of a normalised name.
#[derive(Clone, Debug, Serialize)]
pub struct FileSpan {
    pub key: String,
    pub appeared_by_ms: f64,
    /// The previous tick without it; absent when it existed at the first tick.
    pub absent_at_ms: Option<f64>,
    pub vanished_by_ms: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Observation {
    pub processes: Vec<ProcessSpan>,
    pub files: Vec<FileSpan>,
    pub ticks: u64,
    pub interval_ms: f64,
    pub max_gap_ms: f64,
    pub mean_tick_cost_ms: f64,
}

/// A Kuru or Dolt process visible to this user.
#[derive(Clone, Debug)]
pub struct Seen {
    pub pid: u32,
    pub role: Role,
    pub store: Option<Store>,
    pub ours: bool,
}

/// Incremental process listing. Executable paths and arguments are read once
/// per process, when it is first listed.
pub struct Processes {
    system: System,
    root: PathBuf,
}

impl Processes {
    pub fn new(root: &Path) -> Self {
        Self {
            system: System::new(),
            root: comparable(root),
        }
    }

    pub fn list(&mut self) -> Vec<Seen> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        // A process first listed between fork and exec kept its parent's
        // image and arguments, and they are read only once: read them again
        // for any Kuru or Dolt process whose name no longer matches its image
        // or whose arguments were not yet readable.
        let stale: Vec<Pid> = self
            .system
            .processes()
            .iter()
            .filter(|(_, process)| {
                let name = stem(process.name());
                (name == "kuru" || name == "dolt")
                    && (process.cmd().is_empty()
                        || process
                            .exe()
                            .and_then(Path::file_name)
                            .is_none_or(|image| stem(image) != name))
            })
            .map(|(pid, _)| *pid)
            .collect();
        if !stale.is_empty() {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&stale),
                false,
                ProcessRefreshKind::nothing()
                    .without_tasks()
                    .with_exe(UpdateKind::Always)
                    .with_cmd(UpdateKind::Always),
            );
        }
        let mut seen = Vec::new();
        for (pid, process) in self.system.processes() {
            let name = stem(process.name());
            let stem = name.as_str();
            if stem != "kuru" && stem != "dolt" {
                continue;
            }
            let cmd: Vec<String> = process
                .cmd()
                .iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            let arguments = cmd.get(1..).unwrap_or_default();
            let Some(role) = classify(stem, arguments) else {
                continue;
            };
            let image = process
                .exe()
                .map(Path::to_path_buf)
                .or_else(|| process.cmd().first().map(PathBuf::from));
            let ours = image.is_some_and(|image| comparable(&image).starts_with(&self.root));
            seen.push(Seen {
                pid: pid_number(*pid),
                role,
                store: (role == Role::SqlServer)
                    .then(|| Store::of(arguments))
                    .flatten(),
                ours,
            });
        }
        seen.sort_by_key(|seen| seen.pid);
        seen
    }
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

/// Counts of Kuru and Dolt processes by role, split into those running from
/// the scratch root and any others on the host.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Census {
    pub ours: BTreeMap<&'static str, usize>,
    pub foreign: BTreeMap<&'static str, usize>,
}

impl Census {
    pub fn of(seen: &[Seen]) -> Self {
        let mut census = Self::default();
        for process in seen {
            let counts = if process.ours {
                &mut census.ours
            } else {
                &mut census.foreign
            };
            *counts.entry(process.role.label()).or_default() += 1;
        }
        census
    }
}

/// Wait until no process runs from the root and no `<project>/endpoint.json`
/// is published under `services`, polling
/// every `poll`. Returns the wait in milliseconds; a bound is an
/// infrastructure failure that names what is still alive. Nothing is killed.
pub async fn wait_retired(
    processes: &mut Processes,
    services: &Path,
    bound: Duration,
    poll: Duration,
) -> Result<f64> {
    let started = Instant::now();
    loop {
        let alive: Vec<Seen> = processes
            .list()
            .into_iter()
            .filter(|seen| seen.ours)
            .collect();
        let published = published(services);
        if alive.is_empty() && !published {
            return Ok(millis(started.elapsed()));
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
        tokio::time::sleep(poll).await;
    }
}

pub fn millis(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 10_000.0).round() / 10.0
}

/// Samples processes and files on its own thread until finished.
pub struct Observer {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Observation>,
}

impl Observer {
    /// `watched` maps a key prefix (`data`, `cache`) to its directory.
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

struct Open {
    span: ProcessSpan,
}

fn sample(
    root: &Path,
    watched: &[(&'static str, PathBuf)],
    epoch: Instant,
    interval: Duration,
    stop: &AtomicBool,
) -> Observation {
    let mut processes = Processes::new(root);
    let mut open: HashMap<u32, Open> = HashMap::new();
    let mut files: HashMap<String, FileSpan> = HashMap::new();
    let mut observation = Observation {
        interval_ms: millis(interval),
        ..Observation::default()
    };
    let mut previous: Option<f64> = None;
    let mut cost = Duration::ZERO;
    loop {
        let finishing = stop.load(Ordering::SeqCst);
        let tick = Instant::now();
        let now = millis(tick.saturating_duration_since(epoch));
        let first = previous.is_none();
        // Processes
        let mut alive = HashSet::new();
        for seen in processes.list().into_iter().filter(|seen| seen.ours) {
            alive.insert(seen.pid);
            open.entry(seen.pid)
                .and_modify(|entry| {
                    entry.span.last_seen_ms = now;
                    // First seen between fork and exec: keep the fork as its
                    // start and take the role of the image it became.
                    if entry.span.role != seen.role {
                        entry
                            .span
                            .seen_before_exec_as
                            .get_or_insert(entry.span.role);
                        entry.span.role = seen.role;
                        entry.span.store = seen.store;
                    }
                })
                .or_insert_with(|| Open {
                    span: ProcessSpan {
                        pid: seen.pid,
                        role: seen.role,
                        store: seen.store,
                        preexisting: first,
                        seen_before_exec_as: None,
                        first_seen_ms: now,
                        not_seen_ms: previous,
                        last_seen_ms: now,
                        gone_by_ms: None,
                    },
                });
        }
        let gone: Vec<u32> = open
            .keys()
            .filter(|pid| !alive.contains(pid))
            .copied()
            .collect();
        for pid in gone {
            if let Some(mut entry) = open.remove(&pid) {
                entry.span.gone_by_ms = Some(now);
                observation.processes.push(entry.span);
            }
        }
        // Files
        let mut present = HashSet::new();
        for (prefix, directory) in watched {
            walk(directory, prefix, 0, &mut present);
        }
        for key in &present {
            files.entry(key.clone()).or_insert_with(|| FileSpan {
                key: key.clone(),
                appeared_by_ms: now,
                absent_at_ms: previous,
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
                span.vanished_by_ms = Some(now);
                observation.files.push(span);
            }
        }
        if let Some(previous) = previous {
            observation.max_gap_ms = observation.max_gap_ms.max(now - previous);
        }
        previous = Some(now);
        observation.ticks += 1;
        let spent = tick.elapsed();
        cost += spent;
        if finishing {
            break;
        }
        if let Some(rest) = interval.checked_sub(spent) {
            std::thread::sleep(rest);
        }
    }
    observation
        .processes
        .extend(open.into_values().map(|entry| entry.span));
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
        millis(cost) / observation.ticks as f64
    };
    observation
}

fn walk(directory: &Path, prefix: &str, depth: usize, present: &mut HashSet<String>) {
    if depth > DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    if depth == 0 {
        present.insert(prefix.to_owned());
    }
    for entry in entries.flatten() {
        let name: OsString = entry.file_name();
        let name = name.to_string_lossy();
        if SKIPPED.contains(&name.as_ref()) {
            continue;
        }
        let key = normalize(&format!("{prefix}/{name}"));
        let directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
        present.insert(key.clone());
        if directory {
            walk(
                &entry.path(),
                &key_prefix(prefix, &name),
                depth + 1,
                present,
            );
        }
    }
}

/// The raw prefix for children, normalised per segment by [`normalize`].
fn key_prefix(prefix: &str, name: &str) -> String {
    format!("{prefix}/{name}")
}

/// Whether any owner's service endpoint is published under `services`.
pub fn published(services: &Path) -> bool {
    std::fs::read_dir(services).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().join("endpoint.json").exists())
    })
}
