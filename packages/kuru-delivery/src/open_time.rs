//! Open-time measurement of a release `kuru` executable, driven from outside
//! the binary, and an optional gate over its records.
//!
//! Each iteration runs four cases in a fresh scratch root, with its own
//! HOME, configuration, data directory, engine cache and two project
//! directories, and a fresh copy of the executable:
//!
//! 1. `first-launch`: empty engine cache and data (engine extraction, a new
//!    project's staged creation), then a wait for the owner to retire;
//! 2. `cold-existing`: the same project with no live owner;
//! 3. `warm-reopen`: immediately after, while the cold open's owner may still
//!    be retiring (it retires as soon as its client detaches), then a wait
//!    for retirement;
//! 4. `new-project`: a second, different project in the same data directory
//!    and engine cache, so the engine is warm and only the project is new,
//!    then a wait for retirement.
//!
//! Each run is `kuru --provider demo --no-dream run <prompt> --json` with
//! offline memory: no network, no login and no credential store, and with
//! `KURU_OPEN_MARKERS=1`, so a binary that supports open markers writes them.
//! The harness records wall-clock times of the stderr progress lines and
//! markers and of exit, what [`observe`] saw, a [`census`] before and after
//! the run, host load before and after, and a fixed CPU and IO calibration
//! probe. It writes one `kuru.open-time.v3` JSON line per run and a Markdown
//! summary. Without a [`gate::Gate`] it is report only: a slow or failed open
//! is a result, and only an infrastructure failure (a missing binary, an
//! unwritable output, a process that does not retire) makes it fail. With a
//! gate, the completed main series is also checked against exact engine
//! starts per case and the `new-project` and `cold-existing` median budgets
//! ([`gate`]); the verdict is appended to the summary and returned, and the
//! command exits non-zero on a violation after every record is written. A
//! first series that misses only a median budget is followed by one more
//! full series, which decides ([`gate::Decision`]).
//!
//! Two modes serve local comparisons and are never gated: with file
//! observation off (the control) the observer lists processes only, and with
//! the retirement wait off (the ramp) consecutive runs start while earlier
//! owners are still alive.

pub mod census;
pub mod gate;
mod launch;
pub mod observe;
pub mod report;
#[cfg(test)]
mod tests;

use std::{
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use census::Census;
use observe::{Observation, Observer, Processes, millis};
use report::{Counts, OwnerPath, Stages};

/// v3: the `new-project` case, `stages.ready_signal` and `stages.markers`,
/// and the final milestone named `ready` for either signal.
pub const SCHEMA: &str = "kuru.open-time.v3";
const RECORDS: &str = "records.jsonl";
const SUMMARY: &str = "summary.md";
/// Holds the gate's second series' `records.jsonl`, beside the first's.
const RETRY: &str = "retry";
/// kuru-core `MemoryConfig::default().startup_timeout_secs` (config.rs): one
/// engine's startup timeout, and the owner's wait for a starter to attach.
///
/// This and the product budgets below are restated from the crates that own
/// them, because the delivery tool does not depend on kuru-memory (the
/// open-time job compiles it alone); `product_budgets_match_their_sources`
/// pins each to its source text.
const STARTUP: Duration = Duration::from_secs(30);
/// kuru-memory `server::SUPERVISOR_TRANSPORT_ALLOWANCE` (server.rs), added to
/// the startup timeout for readiness and to the stop graces for the
/// supervisor's exit report.
const SUPERVISOR_TRANSPORT_ALLOWANCE: Duration = Duration::from_secs(2);
/// kuru-memory `server::CLOSE_GRACE` and `KILL_GRACE` (server.rs).
const CLOSE_GRACE: Duration = Duration::from_secs(8);
const KILL_GRACE: Duration = Duration::from_secs(3);
/// kuru-memory `server::close_budget()` (server.rs): the graceful pool
/// drain, the Windows lifetime close, the supervisor reap allowance
/// (`CLOSE_GRACE + KILL_GRACE + SUPERVISOR_TRANSPORT_ALLOWANCE`) and the
/// post-reap pool drain, 8 + 3 + 13 + 8 = 32 s.
const CLOSE_BUDGET: Duration = Duration::from_secs(
    CLOSE_GRACE.as_secs()
        + KILL_GRACE.as_secs()
        + (CLOSE_GRACE.as_secs() + KILL_GRACE.as_secs() + SUPERVISOR_TRANSPORT_ALLOWANCE.as_secs())
        + CLOSE_GRACE.as_secs(),
);
/// kuru-memory `store::QUERY_TIMEOUT` (store.rs): one memory statement.
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
/// kuru-memory `service::rpc::OPERATION_TIMEOUT` (service/rpc.rs): the reply
/// to one request a client sends its memory owner.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(35);
/// kuru-memory `test_support::FreshOpen::FirstProject` (test_support.rs): the
/// engine starts and owned closes of the first open for a store template,
/// which builds the template, then copies it (staging, then active). The
/// open-time gate holds a release build's first launch to exactly these
/// three starts (`ci.yml`).
const FIRST_PROJECT_STARTS: u64 = 3;
const FIRST_PROJECT_CLOSES: u64 = FIRST_PROJECT_STARTS - 1;
/// Bound on one measured command (and on `kuru --version`); past it the
/// command is stopped and the run recorded `timed_out`. The longest case is
/// the first launch, which creates the per-machine store template. Its store
/// creation, at every one of its own bounds, is kuru-memory's fresh-open
/// budget for a first project (`test_support::starts_budget`): the startup
/// lock wait, each engine start (startup timeout and transport allowance)
/// with one statement, each owned close, and the staged directory's
/// quiescence wait: 30 + 3 x (32 + 30) + 2 x 32 + 30 = 310 s. The run's memory
/// requests add one reply budget (35 s), under the single-stall model of
/// kuru-memory's `fixture_deadline`: a request past its own bound fails the
/// run before this one. 345 s. Engine extraction and process start have no
/// product bound of their own and rely on this one.
const RUN_BOUND: Duration = Duration::from_secs(
    STARTUP.as_secs()
        + FIRST_PROJECT_STARTS
            * (STARTUP.as_secs()
                + SUPERVISOR_TRANSPORT_ALLOWANCE.as_secs()
                + QUERY_TIMEOUT.as_secs())
        + FIRST_PROJECT_CLOSES * CLOSE_BUDGET.as_secs()
        + STARTUP.as_secs()
        + OPERATION_TIMEOUT.as_secs(),
);
/// Bound on the wait for every process from a scratch root to retire. The
/// owner retires as soon as its last client detaches (kuru-memory
/// `service.rs`, `owner_retires_at_once_when_its_starter_detaches`); it has
/// no idle window. Retirement is then its store close: the write drain
/// behind at most one in-flight statement (`QUERY_TIMEOUT`, 30 s) and the
/// owned server close (`close_budget()`, 32 s), which reaps the supervisor and
/// engine. An owner whose starter never attached (a run that failed or timed
/// out first) instead waits the startup timeout (30 s) for it, with no write
/// in flight, then closes. The longer of the two paths: max(30, 30) + 32 =
/// 62 s. Past it the series stops with an infrastructure error naming what is
/// still alive; nothing is killed.
const RETIRE_BOUND: Duration = Duration::from_secs(
    if QUERY_TIMEOUT.as_secs() > STARTUP.as_secs() {
        QUERY_TIMEOUT.as_secs()
    } else {
        STARTUP.as_secs()
    } + CLOSE_BUDGET.as_secs(),
);
/// The measurement tool's poll for process exit while it waits for
/// retirement; reported with every summary.
const RETIRE_POLL: Duration = Duration::from_millis(100);
/// Calibration: SHA-256 over a fixed buffer, repeated, and a fixed file
/// round trip.
const CPU_PROBE_BYTES: usize = 256 * 1024 * 1024;
const CPU_PROBE_REPEATS: usize = 5;
const PROBE_BYTES: usize = 64 * 1024 * 1024;
const PROBE_CHUNK: usize = 1024 * 1024;
const ERROR_LIMIT: usize = 300;

#[derive(Clone, Debug)]
pub struct Options {
    /// The release `kuru` executable to measure; never modified or executed
    /// in place except for `--version`.
    pub binary: PathBuf,
    /// Receives `records.jsonl` and `summary.md`.
    pub output: PathBuf,
    /// Parent of the private scratch root; the system temporary directory
    /// when absent.
    pub scratch: Option<PathBuf>,
    pub iterations: usize,
    /// Names the host in the summary, for example the runner label.
    pub label: String,
    /// Also append the summary here (a CI job summary file).
    pub summary: Option<PathBuf>,
    pub interval: Duration,
    /// List file names as well as processes; off for the control series.
    pub files: bool,
    /// Wait for the owner to retire after the first launch, the warm reopen
    /// and the new project; off for the ramp series.
    pub retire_wait: bool,
    /// Bound on one command; past it the command is stopped and recorded.
    pub run_bound: Duration,
    pub prompt: String,
    /// Gate the main series; requires file observation and retirement waits.
    pub gate: Option<gate::Gate>,
}

impl Options {
    pub fn new(binary: PathBuf, output: PathBuf) -> Self {
        Self {
            binary,
            output,
            scratch: None,
            iterations: 10,
            label: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            summary: None,
            interval: Duration::from_millis(20),
            files: true,
            retire_wait: true,
            run_bound: RUN_BOUND,
            prompt: "measure".into(),
            gate: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Case {
    FirstLaunch,
    ColdExisting,
    WarmReopen,
    NewProject,
}

impl Case {
    /// In run order. `new-project` is appended, so the first three keep the
    /// predecessor and gap they always had.
    pub const ALL: [Self; 4] = [
        Self::FirstLaunch,
        Self::ColdExisting,
        Self::WarmReopen,
        Self::NewProject,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::FirstLaunch => "first-launch",
            Self::ColdExisting => "cold-existing",
            Self::WarmReopen => "warm-reopen",
            Self::NewProject => "new-project",
        }
    }
}

/// One stderr line and the time it was read, in milliseconds from the epoch.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct Line {
    pub t_ms: f64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Binary {
    pub sha256: String,
    pub bytes: u64,
    pub version: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Host {
    pub os: &'static str,
    pub arch: &'static str,
    pub cpus: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Load {
    /// One, five and fifteen minute load averages; absent on Windows.
    pub before: Option<[f64; 3]>,
    pub after: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Probe {
    /// Median of `cpu_samples_ms`.
    pub cpu_ms: f64,
    /// Each repeat of SHA-256 over 256 MiB.
    pub cpu_samples_ms: Vec<f64>,
    pub io_ms: f64,
}

/// How the series observed and paced its runs.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Mode {
    pub files: bool,
    pub interval_ms: f64,
    pub retire_wait: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Timings {
    pub spawn_ms: f64,
    pub first_stderr_ms: Option<f64>,
    pub exit_ms: Option<f64>,
    pub timed_out: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Outcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    /// First stderr line that is not progress ([`report::is_progress`]), with
    /// the scratch root replaced.
    pub error: Option<String>,
    /// Stdout parsed as the JSON `run --json` promises.
    pub json_stdout: bool,
}

/// One run: `kuru.open-time.v3`. Times are milliseconds from the moment
/// before the command was started. No path, credential or memory content.
#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub schema: &'static str,
    pub label: String,
    pub case: Case,
    pub iteration: usize,
    pub mode: Mode,
    pub binary: Binary,
    pub host: Host,
    pub started_unix_ms: u128,
    pub gap_since_previous_ms: Option<f64>,
    pub load: Load,
    pub probe: Probe,
    pub census_before: Census,
    /// Taken after the command exited and the observer stopped.
    pub census_after: Census,
    pub timings: Timings,
    /// Progress lines and open markers ([`report::kept_line`]).
    pub stderr: Vec<Line>,
    pub stages: Stages,
    pub counts: Counts,
    pub path: OwnerPath,
    pub observation: Observation,
    /// Wait after this run for every process under the root to retire.
    pub retire_wait_ms: Option<f64>,
    /// Part of that wait until no memory owner process ran from the root.
    pub owner_retired_ms: Option<f64>,
    pub outcome: Outcome,
}

impl Record {
    /// Whether memory opened: the run wrote a `ready` open marker or
    /// `Memory: ready.`. The one definition of a failed open; a command that
    /// fails after that line is counted separately.
    pub fn opened(&self) -> bool {
        self.stages.ready_ms.is_some()
    }
}

#[cfg(test)]
impl Record {
    pub(crate) fn fixture(case: Case, iteration: usize) -> Self {
        Self {
            schema: SCHEMA,
            label: "unit".into(),
            case,
            iteration,
            mode: Mode::default(),
            binary: Binary::default(),
            host: Host::default(),
            started_unix_ms: 0,
            gap_since_previous_ms: None,
            load: Load::default(),
            probe: Probe::default(),
            census_before: Census::default(),
            census_after: Census::default(),
            timings: Timings::default(),
            stderr: Vec::new(),
            stages: Stages::default(),
            counts: Counts::default(),
            path: OwnerPath::default(),
            observation: Observation::default(),
            retire_wait_ms: None,
            owner_retired_ms: None,
            outcome: Outcome::default(),
        }
    }
}

/// What a completed series produced.
#[derive(Debug)]
pub struct Report {
    /// Runs recorded across every series measured.
    pub records: usize,
    /// Series measured: 1, or 2 when the gate measured a second.
    pub series: usize,
    /// Runs without a readiness signal (a `ready` marker or `Memory: ready.`).
    pub failed_opens: usize,
    /// Runs that opened and then failed.
    pub failed_after_open: usize,
    /// The ramp series' single wait for every owner to retire.
    pub final_retire_wait_ms: Option<f64>,
    pub summary: String,
    /// The gate's judgement over the completed series, when gated.
    pub gate: Option<gate::Judgement>,
}

/// One iteration's private directories.
struct Scratch {
    root: PathBuf,
    binary: PathBuf,
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    project: PathBuf,
    /// The `new-project` case's project, beside the first.
    new_project: PathBuf,
    temporary: PathBuf,
    empty_path: PathBuf,
}

impl Scratch {
    fn create(root: PathBuf, source: &Path) -> Result<Self> {
        let scratch = Self {
            binary: root
                .join("bin")
                .join(format!("kuru{}", std::env::consts::EXE_SUFFIX)),
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            project: root.join("project"),
            new_project: root.join("new-project"),
            temporary: root.join("tmp"),
            empty_path: root.join("no-external-tools"),
            root,
        };
        for directory in [
            &scratch.root,
            &scratch.home,
            &scratch.config.join("kuru"),
            &scratch.project,
            &scratch.new_project,
            &scratch.temporary,
            &scratch.empty_path,
            &scratch.root.join("bin"),
        ] {
            fs::create_dir_all(directory)
                .with_context(|| format!("create {}", directory.display()))?;
        }
        let mut memory = toml::Table::new();
        memory.insert("offline".into(), toml::Value::Boolean(true));
        // This harness measures deliberately cold owner starts, not retention.
        memory.insert("service_idle_timeout_secs".into(), toml::Value::Integer(0));
        memory.insert(
            "cache_dir".into(),
            toml::Value::String(scratch.cache.to_string_lossy().into_owned()),
        );
        let mut settings = toml::Table::new();
        settings.insert("memory".into(), toml::Value::Table(memory));
        fs::write(scratch.config_file(), toml::to_string(&settings)?)?;
        // A fresh copy: the first launch includes any first-execution
        // verification the OS applies to a newly written executable.
        fs::copy(source, &scratch.binary)
            .with_context(|| format!("copy the measured binary to {}", scratch.root.display()))?;
        Ok(scratch)
    }

    fn config_file(&self) -> PathBuf {
        self.config.join("kuru").join("config.toml")
    }

    /// The project directory a case opens.
    fn project(&self, case: Case) -> &Path {
        match case {
            Case::NewProject => &self.new_project,
            _ => &self.project,
        }
    }

    /// Holds `<project>/endpoint.json` while an owner is published.
    fn endpoint_directory(&self) -> PathBuf {
        self.data.join("memory").join("services")
    }

    fn environment(&self) -> Vec<(OsString, OsString)> {
        let mut environment: Vec<(OsString, OsString)> = [
            ("HOME", &self.home),
            ("USERPROFILE", &self.home),
            ("APPDATA", &self.config),
            ("LOCALAPPDATA", &self.home.join("local")),
            ("XDG_CONFIG_HOME", &self.config),
            ("XDG_CACHE_HOME", &self.home.join("cache")),
            ("XDG_DATA_HOME", &self.home.join("data")),
            ("TMPDIR", &self.temporary),
            ("TMP", &self.temporary),
            ("TEMP", &self.temporary),
            ("PATH", &self.empty_path),
        ]
        .into_iter()
        .map(|(key, value)| (OsString::from(key), value.as_os_str().to_owned()))
        .collect();
        // A binary that supports open markers writes them; any other ignores
        // the variable and writes the legacy progress lines only.
        environment.push(("KURU_OPEN_MARKERS".into(), "1".into()));
        // An instrumented fixture keeps its coverage destination; a release
        // binary ignores it.
        for key in ["LLVM_PROFILE_FILE", "SystemRoot"] {
            if let Some(value) = std::env::var_os(key) {
                environment.push((key.into(), value));
            }
        }
        environment
    }

    fn arguments(&self, case: Case, prompt: &str) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["-C".into(), self.project(case).into()];
        args.push("--data-dir".into());
        args.push(self.data.clone().into());
        args.push("--config".into());
        args.push(self.config_file().into());
        for arg in ["--provider", "demo", "--no-dream", "run", prompt, "--json"] {
            args.push(arg.into());
        }
        args
    }
}

fn digest(path: &Path) -> Result<(String, u64)> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; PROBE_CHUNK];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        bytes,
    ))
}

/// A fixed CPU task (SHA-256 of 256 MiB, five times; its median is kept) and
/// a fixed IO task (write, fsync, read back and remove 64 MiB) in the scratch
/// root. For comparing runs of one label, not runners of different kinds.
fn calibrate(directory: &Path) -> Result<Probe> {
    let buffer = vec![0x5a_u8; PROBE_CHUNK];
    let cpu_samples_ms: Vec<f64> = (0..CPU_PROBE_REPEATS)
        .map(|_| {
            let started = Instant::now();
            let mut hasher = Sha256::new();
            for _ in 0..CPU_PROBE_BYTES / PROBE_CHUNK {
                hasher.update(&buffer);
            }
            std::hint::black_box(hasher.finalize());
            millis(started.elapsed())
        })
        .collect();
    let cpu_ms = report::stats(&cpu_samples_ms).map_or(0.0, |stats| stats.median);
    let path = directory.join("calibration.bin");
    let started = Instant::now();
    {
        let mut file =
            fs::File::create_new(&path).with_context(|| format!("create {}", path.display()))?;
        for _ in 0..PROBE_BYTES / PROBE_CHUNK {
            file.write_all(&buffer)?;
        }
        file.sync_all()?;
    }
    let mut read = Vec::with_capacity(PROBE_BYTES);
    fs::File::open(&path)?.read_to_end(&mut read)?;
    ensure!(
        read.len() == PROBE_BYTES,
        "calibration file read back short"
    );
    fs::remove_file(&path)?;
    Ok(Probe {
        cpu_ms,
        cpu_samples_ms,
        io_ms: millis(started.elapsed()),
    })
}

fn load() -> Option<[f64; 3]> {
    if cfg!(windows) {
        return None;
    }
    let load = sysinfo::System::load_average();
    Some([load.one, load.five, load.fifteen])
}

fn redact(text: &str, root: &Path) -> String {
    let mut text = text.to_owned();
    for form in [root.to_string_lossy().into_owned(), comparable_text(root)] {
        if !form.is_empty() {
            text = text.replace(&form, "<root>");
        }
    }
    text.chars().take(ERROR_LIMIT).collect()
}

/// The canonical path, without a Windows verbatim prefix on a drive path, so
/// the paths handed to the measured command are ordinary ones.
fn plain(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    let stripped = path
        .to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
        .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
        .map(PathBuf::from);
    Ok(stripped.unwrap_or(path))
}

fn comparable_text(root: &Path) -> String {
    root.canonicalize()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

struct Series<'a> {
    options: &'a Options,
    /// The series' scratch root, holding every iteration directory.
    root: PathBuf,
    binary: Binary,
    host: Host,
    processes: Processes,
    previous_exit: Option<Instant>,
    records: Vec<Record>,
    sink: fs::File,
    /// The ramp series' single wait for every owner to retire.
    final_retire_wait_ms: Option<f64>,
}

impl Series<'_> {
    async fn measure(&mut self, scratch: &Scratch, case: Case, iteration: usize) -> Result<Record> {
        let census_before = census::take(&mut self.processes, &self.root).await;
        let load_before = load();
        let probe = calibrate(&scratch.root)?;
        let launch = launch::Launch {
            program: scratch.binary.clone(),
            args: scratch.arguments(case, &self.options.prompt),
            environment: scratch.environment(),
            cwd: scratch.project(case).to_owned(),
        };
        // The projects already in the data directory are named apart, so the
        // new project's store is seen appearing. Listed before the epoch.
        let prior = if case == Case::NewProject {
            observe::project_hashes(&scratch.data.join("memory"))
        } else {
            Vec::new()
        };
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        let epoch = Instant::now();
        let gap_since_previous_ms = self
            .previous_exit
            .map(|exit| millis(epoch.saturating_duration_since(exit)));
        let watched = if self.options.files {
            vec![
                ("data", scratch.data.clone()),
                ("cache", scratch.cache.clone()),
            ]
        } else {
            Vec::new()
        };
        let observer = Observer::start(&scratch.root, watched, prior, epoch, self.options.interval);
        let finished = launch::run(&launch, epoch, self.options.run_bound).await;
        self.previous_exit = Some(Instant::now());
        let observation = observer.finish();
        let finished = finished?;
        let census_after = census::take(&mut self.processes, &self.root).await;
        let load_after = load();
        let derived = report::derive(&observation, &finished.lines, finished.exit_ms);
        let error = finished
            .lines
            .iter()
            .find(|line| !report::is_progress(&line.text))
            .filter(|_| !finished.success)
            .map(|line| redact(&line.text, &scratch.root));
        let stderr = finished
            .lines
            .iter()
            .filter(|line| report::kept_line(&line.text))
            .cloned()
            .collect();
        Ok(Record {
            schema: SCHEMA,
            label: self.options.label.clone(),
            case,
            iteration,
            mode: Mode {
                files: self.options.files,
                interval_ms: millis(self.options.interval),
                retire_wait: self.options.retire_wait,
            },
            binary: self.binary.clone(),
            host: self.host.clone(),
            started_unix_ms,
            gap_since_previous_ms,
            load: Load {
                before: load_before,
                after: load_after,
            },
            probe,
            census_before,
            census_after,
            timings: Timings {
                spawn_ms: finished.spawn_ms,
                first_stderr_ms: finished.lines.first().map(|line| line.t_ms),
                exit_ms: finished.exit_ms,
                timed_out: finished.timed_out,
            },
            stderr,
            stages: derived.stages,
            counts: derived.counts,
            path: derived.path,
            observation,
            retire_wait_ms: None,
            owner_retired_ms: None,
            outcome: Outcome {
                success: finished.success,
                exit_code: finished.exit_code,
                error,
                json_stdout: serde_json::from_slice::<serde_json::Value>(&finished.stdout).is_ok(),
            },
        })
    }

    async fn retire(&mut self, scratch: &Scratch, record: &mut Record) -> Result<()> {
        let services = scratch.endpoint_directory();
        let retired =
            observe::wait_retired(&mut self.processes, &services, RETIRE_BOUND, RETIRE_POLL)
                .await?;
        record.retire_wait_ms = Some(retired.wait_ms);
        record.owner_retired_ms = Some(retired.owner_retired_ms);
        Ok(())
    }

    /// Every iteration, then the ramp's single final retirement wait.
    /// Completed runs stay in `records` whatever stops the series.
    async fn all(&mut self, binary: &Path) -> Result<()> {
        let mut kept = Vec::new();
        for iteration in 1..=self.options.iterations {
            let directory = self.root.join(format!("iteration-{iteration:02}"));
            let scratch = Scratch::create(directory.clone(), binary)?;
            self.iteration(&scratch, iteration).await?;
            if self.options.retire_wait {
                // Every process under this directory has retired; remove its
                // engine cache and store before the next iteration.
                fs::remove_dir_all(&directory)
                    .with_context(|| format!("remove {}", directory.display()))?;
            } else {
                kept.push(directory);
            }
        }
        // The ramp series waits once, after its last run, for every owner it
        // started; only then can the iteration directories be removed.
        if kept.is_empty() {
            return Ok(());
        }
        let started = Instant::now();
        // Each wait also covers every process under the root, so the first
        // one outlasts the rest.
        for directory in &kept {
            let services = directory.join("data").join("memory").join("services");
            observe::wait_retired(&mut self.processes, &services, RETIRE_BOUND, RETIRE_POLL)
                .await?;
        }
        self.final_retire_wait_ms = Some(millis(started.elapsed()));
        for directory in &kept {
            fs::remove_dir_all(directory)
                .with_context(|| format!("remove {}", directory.display()))?;
        }
        Ok(())
    }

    fn keep(&mut self, record: Record) -> Result<()> {
        serde_json::to_writer(&mut self.sink, &record)?;
        self.sink.write_all(b"\n")?;
        self.sink.flush()?;
        self.records.push(record);
        Ok(())
    }

    async fn iteration(&mut self, scratch: &Scratch, iteration: usize) -> Result<()> {
        for case in Case::ALL {
            let mut record = self.measure(scratch, case, iteration).await?;
            let retired = if case == Case::ColdExisting || !self.options.retire_wait {
                Ok(())
            } else {
                self.retire(scratch, &mut record).await
            };
            self.keep(record)?;
            retired?;
        }
        Ok(())
    }
}

async fn version(binary: &Path, scratch: &Path, bound: Duration) -> Result<String> {
    let home = scratch.join("version-home");
    fs::create_dir_all(&home)?;
    let mut environment: Vec<(OsString, OsString)> =
        ["HOME", "USERPROFILE", "TMPDIR", "TMP", "TEMP"]
            .into_iter()
            .map(|key| (OsString::from(key), home.as_os_str().to_owned()))
            .collect();
    if let Some(value) = std::env::var_os("SystemRoot") {
        environment.push(("SystemRoot".into(), value));
    }
    if let Some(value) = std::env::var_os("LLVM_PROFILE_FILE") {
        environment.push(("LLVM_PROFILE_FILE".into(), value));
    }
    let launch = launch::Launch {
        program: binary.to_owned(),
        args: vec!["--version".into()],
        environment,
        cwd: home.clone(),
    };
    let finished = launch::run(&launch, Instant::now(), bound).await?;
    ensure!(
        finished.success,
        "{} --version failed: {:?}",
        binary.display(),
        finished.lines
    );
    Ok(String::from_utf8_lossy(&finished.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned())
}

/// A fresh private scratch root under `parent`, and its plain path.
fn scratch_root(parent: &Path) -> Result<(tempfile::TempDir, PathBuf)> {
    let scratch = tempfile::Builder::new()
        .prefix("kuru-open-time-")
        .tempdir_in(parent)
        .context("create the private scratch root")?;
    let root = plain(scratch.path())?;
    Ok((scratch, root))
}

/// What one series left: every completed run, and what stopped it.
struct Measured {
    records: Vec<Record>,
    final_retire_wait_ms: Option<f64>,
    /// An infrastructure failure that stopped the series part way.
    failure: Option<anyhow::Error>,
}

impl Measured {
    fn stopped(error: anyhow::Error) -> Self {
        Self {
            records: Vec::new(),
            final_retire_wait_ms: None,
            failure: Some(error),
        }
    }

    fn summary(&self, label: &str, gated: bool) -> String {
        let mut summary = report::summary_of(&self.records, label, gated);
        if let Some(wait) = self.final_retire_wait_ms {
            summary.push_str(&format!(
                "\nFinal wait for every owner of the ramp to retire: {wait:.0} ms.\n"
            ));
        }
        summary
    }

    fn runs(&self) -> Vec<gate::Run> {
        self.records.iter().map(gate::Run::from).collect()
    }
}

/// Measure one full series in the scratch root `root`, writing each record
/// to `records` as its run finishes.
async fn measure(
    options: &Options,
    binary: &Path,
    identity: &Binary,
    root: &Path,
    records: &Path,
) -> Measured {
    let sink = match fs::File::create(records) {
        Ok(sink) => sink,
        Err(error) => {
            return Measured::stopped(
                anyhow::Error::new(error).context(format!("create {}", records.display())),
            );
        }
    };
    let mut series = Series {
        options,
        root: root.to_owned(),
        binary: identity.clone(),
        host: Host {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            cpus: std::thread::available_parallelism().map_or(1, usize::from),
        },
        processes: Processes::new(root),
        previous_exit: None,
        records: Vec::new(),
        sink,
        final_retire_wait_ms: None,
    };
    let failure = series.all(binary).await.err();
    Measured {
        records: series.records,
        final_retire_wait_ms: series.final_retire_wait_ms,
        failure,
    }
}

/// Run the series. Records and the summary are written as runs finish, so an
/// infrastructure failure part way still leaves every completed run.
///
/// Gated, a first series that holds every structural check but misses a
/// median budget is followed by one more full series, with the same cases
/// and iterations in a fresh scratch root; its records go to
/// `<output>/retry/records.jsonl`, and the summary carries both series and
/// the decision ([`gate::Decision`]).
pub async fn run(options: &Options) -> Result<Report> {
    ensure!(options.iterations > 0, "at least one iteration is required");
    // The budgets derive from the main series; the control lists no files
    // and the ramp changes owner paths, so neither is gated.
    ensure!(
        options.gate.is_none() || (options.files && options.retire_wait),
        "the open-time gate applies to the main series only: file observation and \
         retirement waits must both be on"
    );
    let binary = plain(&options.binary)
        .with_context(|| format!("measured binary {}", options.binary.display()))?;
    ensure!(binary.is_file(), "{} is not a file", binary.display());
    fs::create_dir_all(&options.output)
        .with_context(|| format!("create {}", options.output.display()))?;
    let parent = options.scratch.clone().unwrap_or_else(std::env::temp_dir);
    fs::create_dir_all(&parent)?;
    let (scratch, root) = scratch_root(&parent)?;
    let (sha256, bytes) = digest(&binary)?;
    let version = version(&binary, &root, options.run_bound).await?;
    let identity = Binary {
        sha256,
        bytes,
        version,
    };
    let gated = options.gate.is_some();
    let first = measure(
        options,
        &binary,
        &identity,
        &root,
        &options.output.join(RECORDS),
    )
    .await;
    // One entry per series measured or attempted; `None` for a second series
    // that stopped before it had a scratch root of its own.
    let mut scratches = vec![Some(scratch)];
    let mut second = None;
    // A series stopped by an infrastructure failure is not gated: its
    // records are partial, and the failure is the result.
    let (summary, judgement) = match (&options.gate, first.failure.is_none()) {
        (Some(gate), true) => {
            let verdict = gate::evaluate(&first.runs(), gate);
            let decision = gate::Decision::decide(&verdict, None);
            if decision == gate::Decision::Remeasure {
                let label = format!("{}, first series", options.label);
                let mut summary = first.summary(&label, gated);
                summary.push_str(&gate::render_titled(
                    &verdict,
                    gate,
                    "Open-time gate, first series",
                ));
                summary.push_str(&decision.render());
                let directory = options.output.join(RETRY);
                let prepared = fs::create_dir_all(&directory)
                    .with_context(|| format!("create {}", directory.display()))
                    .and_then(|()| scratch_root(&parent));
                let measured = match prepared {
                    Ok((scratch, root)) => {
                        scratches.push(Some(scratch));
                        let records = directory.join(RECORDS);
                        measure(options, &binary, &identity, &root, &records).await
                    }
                    Err(error) => {
                        scratches.push(None);
                        Measured::stopped(error)
                    }
                };
                let label = format!("{}, second series", options.label);
                summary.push('\n');
                summary.push_str(&measured.summary(&label, gated));
                let judgement = if measured.failure.is_none() {
                    let retried = gate::evaluate(&measured.runs(), gate);
                    let decision = gate::Decision::decide(&verdict, Some(&retried));
                    summary.push_str(&gate::render_titled(
                        &retried,
                        gate,
                        "Open-time gate, second series",
                    ));
                    summary.push_str(&decision.render());
                    Some(gate::Judgement {
                        first: verdict,
                        second: Some(retried),
                        decision,
                    })
                } else {
                    summary.push_str(
                        "\n## Open-time gate, second series: not evaluated\n\n\
                         The second series stopped.\n",
                    );
                    None
                };
                second = Some(measured);
                (summary, judgement)
            } else {
                let mut summary = first.summary(&options.label, gated);
                summary.push_str(&gate::render(&verdict, gate));
                summary.push_str(&decision.render());
                (
                    summary,
                    Some(gate::Judgement {
                        first: verdict,
                        second: None,
                        decision,
                    }),
                )
            }
        }
        (Some(_), false) => {
            let mut summary = first.summary(&options.label, gated);
            summary.push_str("\n## Open-time gate: not evaluated\n\nThe series stopped.\n");
            (summary, None)
        }
        (None, _) => (first.summary(&options.label, gated), None),
    };
    fs::write(options.output.join(SUMMARY), &summary)?;
    if let Some(path) = &options.summary {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{summary}")?;
    }
    let mut first = first;
    let failure = first.failure.take().or_else(|| {
        second
            .as_mut()
            .and_then(|second: &mut Measured| second.failure.take())
    });
    let measured: Vec<&Measured> = std::iter::once(&first).chain(second.iter()).collect();
    let all = || measured.iter().flat_map(|series| series.records.iter());
    let failed_opens = all().filter(|record| !record.opened()).count();
    let failed_after_open = all()
        .filter(|record| record.opened() && !record.outcome.success)
        .count();
    let records = all().count();
    let series = measured.len();
    let final_retire_wait_ms = first.final_retire_wait_ms;
    if let Some(error) = failure {
        // Leave the stopped series' scratch root for inspection when a
        // process is still alive; any earlier one is removed. A second
        // series that could not start has no root, so nothing is kept.
        let kept = scratches.pop().flatten().map(tempfile::TempDir::keep);
        drop(scratches);
        let kept = kept.map_or_else(
            || "the stopped series had no scratch root, so none was kept".to_owned(),
            |kept| format!("scratch kept at {}", kept.display()),
        );
        bail!("open-time measurement stopped after {records} records: {error:#}; {kept}");
    }
    for scratch in scratches.into_iter().flatten() {
        scratch.close().context("remove the scratch root")?;
    }
    Ok(Report {
        records,
        series,
        failed_opens,
        failed_after_open,
        final_retire_wait_ms,
        summary,
        gate: judgement,
    })
}
