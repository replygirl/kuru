//! Report-only open-time measurement of a release `kuru` executable, driven
//! from outside the binary.
//!
//! Each iteration runs three cases in a fresh scratch root, with its own
//! HOME, configuration, data directory, engine cache and project, and a fresh
//! copy of the executable:
//!
//! 1. `first-launch`: empty engine cache and data (engine extraction, a new
//!    project's staged creation), then a wait for the owner to retire;
//! 2. `cold-existing`: the same project with no live owner;
//! 3. `warm-reopen`: immediately after, inside the owner's idle window, then a
//!    wait for retirement.
//!
//! Each run is `kuru --provider demo --no-dream run <prompt> --json` with
//! offline memory: no network, no login and no credential store. The harness
//! records wall-clock times of the stderr progress lines and of exit, what
//! [`observe`] saw, a process census before the run, host load before and
//! after, and a fixed CPU and IO calibration probe. It writes one
//! `kuru.open-time.v1` JSON line per run and a Markdown summary. It never
//! compares a time with a budget: a slow or failed open is a result, and only
//! an infrastructure failure (a missing binary, an unwritable output, a
//! process that does not retire) makes it fail.

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
use serde::Serialize;
use sha2::{Digest, Sha256};

use observe::{Census, Observation, Observer, Processes, millis};
use report::{Counts, OwnerPath, Stages};

pub const SCHEMA: &str = "kuru.open-time.v1";
const RECORDS: &str = "records.jsonl";
const SUMMARY: &str = "summary.md";
/// The owner idles 30 s after its last client (kuru-memory
/// `SERVICE_IDLE_TIMEOUT`), then closes its engine and supervisor.
const RETIRE_BOUND: Duration = Duration::from_secs(120);
const RETIRE_POLL: Duration = Duration::from_millis(100);
/// Calibration: SHA-256 over a fixed buffer and a fixed file round trip.
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
    /// Bound on one command; past it the command is stopped and recorded.
    pub run_bound: Duration,
    pub prompt: String,
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
            run_bound: Duration::from_secs(180),
            prompt: "measure".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Case {
    FirstLaunch,
    ColdExisting,
    WarmReopen,
}

impl Case {
    pub const ALL: [Self; 3] = [Self::FirstLaunch, Self::ColdExisting, Self::WarmReopen];

    pub fn label(self) -> &'static str {
        match self {
            Self::FirstLaunch => "first-launch",
            Self::ColdExisting => "cold-existing",
            Self::WarmReopen => "warm-reopen",
        }
    }
}

/// One stderr line and the time it was read, in milliseconds from the epoch.
#[derive(Clone, Debug, Serialize)]
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
    pub cpu_ms: f64,
    pub io_ms: f64,
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
    /// First non-progress stderr line, with the scratch root replaced.
    pub error: Option<String>,
    /// Stdout parsed as the JSON `run --json` promises.
    pub json_stdout: bool,
}

/// One run: `kuru.open-time.v1`. Times are milliseconds from the moment
/// before the command was started. No path, credential or memory content.
#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub schema: &'static str,
    pub label: String,
    pub case: Case,
    pub iteration: usize,
    pub binary: Binary,
    pub host: Host,
    pub started_unix_ms: u128,
    pub gap_since_previous_ms: Option<f64>,
    pub load: Load,
    pub probe: Probe,
    pub census_before: Census,
    pub timings: Timings,
    pub stderr: Vec<Line>,
    pub stages: Stages,
    pub counts: Counts,
    pub path: OwnerPath,
    pub observation: Observation,
    /// Wait after this run for every process under the root to retire.
    pub retire_wait_ms: Option<f64>,
    pub outcome: Outcome,
}

#[cfg(test)]
impl Record {
    pub(crate) fn fixture(case: Case, iteration: usize) -> Self {
        Self {
            schema: SCHEMA,
            label: "unit".into(),
            case,
            iteration,
            binary: Binary::default(),
            host: Host::default(),
            started_unix_ms: 0,
            gap_since_previous_ms: None,
            load: Load::default(),
            probe: Probe::default(),
            census_before: Census::default(),
            timings: Timings::default(),
            stderr: Vec::new(),
            stages: Stages::default(),
            counts: Counts::default(),
            path: OwnerPath::default(),
            observation: Observation::default(),
            retire_wait_ms: None,
            outcome: Outcome::default(),
        }
    }
}

/// What a completed series produced.
#[derive(Debug)]
pub struct Report {
    pub records: usize,
    pub failed_opens: usize,
    pub summary: String,
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
            temporary: root.join("tmp"),
            empty_path: root.join("no-external-tools"),
            root,
        };
        for directory in [
            &scratch.root,
            &scratch.home,
            &scratch.config.join("kuru"),
            &scratch.project,
            &scratch.temporary,
            &scratch.empty_path,
            &scratch.root.join("bin"),
        ] {
            fs::create_dir_all(directory)
                .with_context(|| format!("create {}", directory.display()))?;
        }
        let mut memory = toml::Table::new();
        memory.insert("offline".into(), toml::Value::Boolean(true));
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
        // An instrumented fixture keeps its coverage destination; a release
        // binary ignores it.
        for key in ["LLVM_PROFILE_FILE", "SystemRoot"] {
            if let Some(value) = std::env::var_os(key) {
                environment.push((key.into(), value));
            }
        }
        environment
    }

    fn arguments(&self, prompt: &str) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["-C".into(), self.project.clone().into()];
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

/// A fixed CPU task (SHA-256 of 64 MiB) and a fixed IO task (write, fsync,
/// read back and remove 64 MiB) in the scratch root.
fn calibrate(directory: &Path) -> Result<Probe> {
    let buffer = vec![0x5a_u8; PROBE_CHUNK];
    let started = Instant::now();
    let mut hasher = Sha256::new();
    for _ in 0..PROBE_BYTES / PROBE_CHUNK {
        hasher.update(&buffer);
    }
    std::hint::black_box(hasher.finalize());
    let cpu_ms = millis(started.elapsed());
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
    binary: Binary,
    host: Host,
    processes: Processes,
    previous_exit: Option<Instant>,
    records: Vec<Record>,
    sink: fs::File,
}

impl Series<'_> {
    async fn measure(&mut self, scratch: &Scratch, case: Case, iteration: usize) -> Result<Record> {
        let census_before = Census::of(&self.processes.list());
        let load_before = load();
        let probe = calibrate(&scratch.root)?;
        let launch = launch::Launch {
            program: scratch.binary.clone(),
            args: scratch.arguments(&self.options.prompt),
            environment: scratch.environment(),
            cwd: scratch.project.clone(),
        };
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        let epoch = Instant::now();
        let gap_since_previous_ms = self
            .previous_exit
            .map(|exit| millis(epoch.saturating_duration_since(exit)));
        let observer = Observer::start(
            &scratch.root,
            vec![
                ("data", scratch.data.clone()),
                ("cache", scratch.cache.clone()),
            ],
            epoch,
            self.options.interval,
        );
        let finished = launch::run(&launch, epoch, self.options.run_bound).await;
        self.previous_exit = Some(Instant::now());
        let observation = observer.finish();
        let finished = finished?;
        let load_after = load();
        let derived = report::derive(&observation, &finished.lines, finished.exit_ms);
        let error = finished
            .lines
            .iter()
            .find(|line| !line.text.starts_with("Memory"))
            .filter(|_| !finished.success)
            .map(|line| redact(&line.text, &scratch.root));
        let stderr = finished
            .lines
            .iter()
            .filter(|line| line.text.starts_with("Memory: "))
            .cloned()
            .collect();
        Ok(Record {
            schema: SCHEMA,
            label: self.options.label.clone(),
            case,
            iteration,
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
        record.retire_wait_ms = Some(
            observe::wait_retired(&mut self.processes, &services, RETIRE_BOUND, RETIRE_POLL)
                .await?,
        );
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
            let retired = if case == Case::ColdExisting {
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

/// Run the series. Records and the summary are written as runs finish, so an
/// infrastructure failure part way still leaves every completed run.
pub async fn run(options: &Options) -> Result<Report> {
    ensure!(options.iterations > 0, "at least one iteration is required");
    let binary = plain(&options.binary)
        .with_context(|| format!("measured binary {}", options.binary.display()))?;
    ensure!(binary.is_file(), "{} is not a file", binary.display());
    fs::create_dir_all(&options.output)
        .with_context(|| format!("create {}", options.output.display()))?;
    let parent = options.scratch.clone().unwrap_or_else(std::env::temp_dir);
    fs::create_dir_all(&parent)?;
    let scratch = tempfile::Builder::new()
        .prefix("kuru-open-time-")
        .tempdir_in(&parent)
        .context("create the private scratch root")?;
    let root = plain(scratch.path())?;
    let (sha256, bytes) = digest(&binary)?;
    let version = version(&binary, &root, options.run_bound).await?;
    let sink = fs::File::create(options.output.join(RECORDS))?;
    let mut series = Series {
        options,
        binary: Binary {
            sha256,
            bytes,
            version,
        },
        host: Host {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            cpus: std::thread::available_parallelism().map_or(1, usize::from),
        },
        processes: Processes::new(&root),
        previous_exit: None,
        records: Vec::new(),
        sink,
    };
    let mut failure = None;
    for iteration in 1..=options.iterations {
        let directory = root.join(format!("iteration-{iteration:02}"));
        let result = match Scratch::create(directory.clone(), &binary) {
            Ok(scratch) => series.iteration(&scratch, iteration).await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            failure = Some(error);
            break;
        }
        // Every process under this directory has retired; remove its engine
        // cache and store before the next iteration.
        fs::remove_dir_all(&directory)
            .with_context(|| format!("remove {}", directory.display()))?;
    }
    let summary = report::summary(&series.records, &options.label);
    fs::write(options.output.join(SUMMARY), &summary)?;
    if let Some(path) = &options.summary {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{summary}")?;
    }
    let failed_opens = series
        .records
        .iter()
        .filter(|record| !record.outcome.success)
        .count();
    let records = series.records.len();
    if let Some(error) = failure {
        // Leave the scratch root for inspection when a process is still alive.
        let kept = scratch.keep();
        bail!(
            "open-time measurement stopped after {records} records: {error:#}; scratch kept at {}",
            kept.display()
        );
    }
    scratch.close().context("remove the scratch root")?;
    Ok(Report {
        records,
        failed_opens,
        summary,
    })
}
