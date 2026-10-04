//! Test fixtures: an inventory of fake test executables, a scripted
//! [`Launcher`] and complete partition evidence built through the real
//! dispatcher, ledger validation and evidence writer.

use super::{
    Artifact, Inventory, LLVM_COV_VERSION, Launch, Launcher, LibtestProgress, Mode, ProfileEnv,
    ReceiptIdentity, ReceiptOptions, SCHEMA, StallEvidence, Supervision, dispatch_with,
    ledger::{self, JobLedger},
    lines::{Instantiation, LINES_SCHEMA, LineExport, LineSet},
    partition::PartitionScheme,
    plan, write_evidence, write_json,
};
use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::ExitStatus,
};
use tempfile::TempDir;

pub const HOST: &str = "x86_64-unknown-linux-gnu";
pub const SOURCE: &str = "abc123";

/// An exit status with this code.
pub fn exit(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }
}

pub fn artifact(package: &str, kind: &str, name: &str) -> Artifact {
    let executable = format!("debug/deps/{name}-0a1b");
    Artifact {
        package: package.to_owned(),
        package_root: format!("packages/{package}"),
        target_name: name.to_owned(),
        target_kind: vec![kind.to_owned()],
        crate_types: vec!["bin".to_owned()],
        source: format!("packages/{package}/src/lib.rs"),
        edition: "2024".to_owned(),
        doc: false,
        doctest: false,
        target_test: true,
        profile_test: true,
        opt_level: "0".to_owned(),
        debuginfo: "2".to_owned(),
        debug_assertions: true,
        overflow_checks: true,
        features: Vec::new(),
        filenames: vec![executable.clone()],
        executable: Some(executable),
    }
}

/// A workspace whose inventory names real (empty) executable files.
pub struct Workspace {
    pub temp: TempDir,
    pub root: PathBuf,
    pub target: PathBuf,
    pub inventory: PathBuf,
    pub value: Inventory,
    /// `--list --format terse` output per executable path.
    pub lists: BTreeMap<PathBuf, String>,
}

impl Workspace {
    /// One runnable artifact per scoped package plus a second, integration
    /// test for the first; the last artifact lists no tests at all.
    pub fn new(mode: Mode) -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let target = temp.path().join("target");
        fs::create_dir_all(target.join("debug/deps")).unwrap();
        let scope: Vec<String> = match mode {
            Mode::Instrumented => super::WORKSPACE_PACKAGES.map(str::to_owned).to_vec(),
            Mode::Uninstrumented => vec!["kuru-memory".to_owned()],
        };
        let mut artifacts: Vec<_> = scope
            .iter()
            .map(|package| artifact(package, "lib", &package.replace('-', "_")))
            .collect();
        artifacts.push(artifact(&scope[0], "test", "integration"));
        // A non-test library artifact is recorded but never run.
        let mut library = artifact(&scope[0], "lib", "library");
        library.profile_test = false;
        library.executable = None;
        artifacts.push(library);
        artifacts.sort();
        let mut lists = BTreeMap::new();
        let runnable: Vec<_> = artifacts
            .iter()
            .filter(|artifact| artifact.executable.is_some())
            .collect();
        let last = runnable.len() - 1;
        for (position, artifact) in runnable.iter().enumerate() {
            let path = target.join(artifact.executable.as_deref().unwrap());
            fs::write(&path, b"").unwrap();
            let count = if position == last {
                0
            } else {
                3 + position * 2
            };
            let text: String = (0..count)
                .map(|test| format!("tests::case_{test:02}: test\n"))
                .collect();
            lists.insert(path, text);
        }
        let value = Inventory {
            schema: SCHEMA,
            workspace_packages: super::WORKSPACE_PACKAGES.map(str::to_owned).to_vec(),
            scope,
            artifacts,
        };
        let inventory = temp.path().join("inventory.json");
        write_json(&inventory, &value).unwrap();
        Self {
            temp,
            root,
            target,
            inventory,
            value,
            lists,
        }
    }

    pub fn launcher(&self) -> ScriptedLauncher {
        ScriptedLauncher {
            lists: self.lists.clone(),
            ..ScriptedLauncher::default()
        }
    }

    pub fn runnable(&self) -> Vec<&Artifact> {
        self.value
            .artifacts
            .iter()
            .filter(|artifact| artifact.executable.is_some())
            .collect()
    }

    /// Dispatch every runnable artifact once, as Cargo would.
    pub async fn dispatch_all(
        &self,
        launcher: &mut ScriptedLauncher,
        partition: &PartitionScheme,
        ledger: &Path,
        diagnostics: &Path,
        exclusions: &[(&str, &str, &str)],
    ) -> Result<()> {
        fs::create_dir_all(diagnostics)?;
        for artifact in self.runnable() {
            let executable = self.target.join(artifact.executable.as_deref().unwrap());
            dispatch_with(
                launcher,
                &super::DispatchOptions {
                    root: &self.root,
                    inventory: &self.inventory,
                    host: HOST,
                    partition,
                    target_dir: &self.target,
                    ledger,
                    diagnostics,
                    deadline: super::unix_now()? + 3600,
                    executable: &executable,
                    args: &[],
                },
                &self.root.join(&artifact.package_root),
                exclusions,
            )
            .await?;
        }
        Ok(())
    }

    /// Build one partition's complete evidence into `output`, as the
    /// orchestrator and receipt writer would.
    pub async fn evidence(
        &self,
        mode: Mode,
        partition: &PartitionScheme,
        attempt: u64,
        output: &Path,
    ) -> Result<()> {
        let label = format!("{}-{attempt}", partition.index);
        let state = self.temp.path().join(format!("state-{label}"));
        let profiles = self.temp.path().join(format!("profiles-{label}"));
        fs::create_dir_all(&state)?;
        fs::create_dir_all(&profiles)?;
        let mut launcher = self.launcher();
        if mode == Mode::Instrumented {
            launcher.profiles = Some(profiles.clone());
        }
        let ledger_path = state.join("runner-ledger.jsonl");
        self.dispatch_all(
            &mut launcher,
            partition,
            &ledger_path,
            &state.join("diagnostics"),
            &[],
        )
        .await?;
        let records = plan::read_ledger(&ledger_path)?;
        let job_ledger = state.join("job-ledger.json");
        write_json(
            &job_ledger,
            &JobLedger {
                schema: SCHEMA,
                partition: partition.clone(),
                mode,
                job_started: 1,
                phases: BTreeMap::from([(
                    "tests".to_owned(),
                    ledger::Phase {
                        started: 2,
                        finished: 2 + u64::from(partition.index),
                    },
                )]),
                cache: ledger::CacheLedger {
                    helper: "hit".to_owned(),
                    seed: "miss".to_owned(),
                    seed_matched_key: None,
                    seed_imported: ledger::Transfer::default(),
                    seed_refused: BTreeMap::new(),
                    dependency_units_rebuilt: 400,
                    workspace_units_rebuilt: 12,
                },
                profiles: ledger::profiles(launcher.written, 0),
                executables: ledger::executables(&records),
            },
        )?;
        let lcov = state.join("coverage.lcov");
        let lines = state.join("coverage-lines.json");
        if mode == Mode::Instrumented {
            fs::write(&lcov, lcov_for(partition))?;
            fs::write(&lines, lines_for(partition))?;
        }
        write_evidence(
            &ReceiptOptions {
                root: &self.root,
                mode,
                partition,
                inventory: &self.inventory,
                ledger: &ledger_path,
                job_ledger: &job_ledger,
                profiles: &profiles,
                lcov: (mode == Mode::Instrumented).then_some(lcov.as_path()),
                lines: (mode == Mode::Instrumented).then_some(lines.as_path()),
                run_attempt: &attempt.to_string(),
                expected_source: SOURCE,
                llvm_cov: None,
                profile_env: &profile_env(),
                output,
                // Evidence is written for a given identity; no probe runs.
                deadline: 0,
            },
            identity(mode),
        )
    }
}

/// A partition's digested profile environment.
pub fn profile_env() -> ProfileEnv {
    ProfileEnv::from([
        ("env:RUST_TEST_THREADS".to_owned(), "2".to_owned()),
        (
            "show-env:LLVM_PROFILE_FILE".to_owned(),
            "${KURU_COVERAGE_TARGET}/kuru-%p-%${KURU_COVERAGE_POOL}m.profraw".to_owned(),
        ),
    ])
}

pub fn identity(mode: Mode) -> ReceiptIdentity {
    ReceiptIdentity {
        source: SOURCE.to_owned(),
        tree: "tree".to_owned(),
        cargo_lock_sha256: "lock".to_owned(),
        rustc: format!("rustc 1.98.1\nhost: {HOST}"),
        cargo: "cargo 1.98.1".to_owned(),
        target: HOST.to_owned(),
        cargo_llvm_cov: (mode == Mode::Instrumented).then(|| LLVM_COV_VERSION.to_owned()),
        target_os: "linux".to_owned(),
    }
}

/// One file of 20 lines; partition k hits lines k, k + count, ... below 20,
/// so the union hits 19 of 20 lines (95%) and no partition alone reaches the gate.
pub fn lcov_for(partition: &PartitionScheme) -> String {
    let mut text = String::from("SF:packages/kuru-core/src/lib.rs\nFN:1,_RNvf\n");
    text.push_str(&format!("FNDA:{},_RNvf\n", u32::from(partition.index == 1)));
    for line in 1..=20_u32 {
        let hit = line < 20 && (line - 1) % partition.count + 1 == partition.index;
        text.push_str(&format!("DA:{line},{}\n", u32::from(hit)));
    }
    text.push_str("end_of_record\n");
    text
}

/// Lines 1 to 20 of partition `k`, hit as in [`lcov_for`], as the one
/// instantiation of one group.
pub fn hits(partition: &PartitionScheme) -> Vec<u32> {
    (1..20_u32)
        .filter(|line| (line - 1) % partition.count + 1 == partition.index)
        .collect()
}

/// The canonical line export matching [`lcov_for`].
pub fn lines_for(partition: &PartitionScheme) -> String {
    let file = "packages/kuru-core/src/lib.rs".to_owned();
    let export = LineExport {
        schema: LINES_SCHEMA,
        files: vec![file.clone()],
        instantiations: vec![Instantiation {
            file,
            name: "_RNvf".to_owned(),
            line: 1,
            column: 1,
            mapped: LineSet::from_sorted((1..=20).collect()),
            covered: LineSet::from_sorted(hits(partition)),
        }],
    };
    String::from_utf8(export.render().unwrap()).unwrap()
}

/// A launcher that lists scripted names and "runs" selections instantly.
#[derive(Default)]
pub struct ScriptedLauncher {
    pub lists: BTreeMap<PathBuf, String>,
    /// Every exact selection's arguments, in order.
    pub runs: Vec<Vec<String>>,
    /// Added to the announced count of every selection.
    pub announce_offset: isize,
    pub status: i32,
    /// The one-based exact selection that exits 101 instead of `status`.
    pub failing_run: Option<usize>,
    pub stall: bool,
    pub list_error: bool,
    /// The bound each list was given, in order.
    pub list_bounds: Vec<std::time::Duration>,
    /// The time left each exact selection was given, in order.
    pub run_bounds: Vec<std::time::Duration>,
    pub run_error: bool,
    /// Where each process writes one raw profile, as an instrumented one would.
    pub profiles: Option<PathBuf>,
    pub written: usize,
}

impl ScriptedLauncher {
    fn profile(&mut self) -> Result<()> {
        if let Some(directory) = &self.profiles {
            self.written += 1;
            fs::write(
                // `%p-%m`: the count stands in for the process ID.
                directory.join(format!("kuru-{}-7_0.profraw", self.written)),
                b"profile",
            )?;
        }
        Ok(())
    }
}

impl Launcher for ScriptedLauncher {
    async fn list(&mut self, executable: &Path, bound: std::time::Duration) -> Result<String> {
        self.list_bounds.push(bound);
        if self.list_error {
            bail!("scripted list failure");
        }
        self.profile()?;
        Ok(self.lists.get(executable).cloned().unwrap_or_default())
    }

    async fn run(&mut self, launch: &Launch<'_>) -> Result<Supervision> {
        if self.run_error {
            bail!("scripted run failure");
        }
        self.profile()?;
        self.runs.push(launch.args.to_vec());
        self.run_bounds.push(launch.remaining);
        fs::write(&launch.log, b"running\n")?;
        if self.stall {
            let mut progress = LibtestProgress::default();
            progress.observe(b"test tests::stuck ... ");
            return Ok(Supervision::Stalled(Box::new(StallEvidence {
                progress,
                sample: None,
                termination: "Ok(())".to_owned(),
                cleanup: "Ok(())".to_owned(),
                presence_after_reap: None,
                output: "complete".to_owned(),
            })));
        }
        let selected = launch.args.len() as isize - 1;
        let status = if self.failing_run == Some(self.runs.len()) {
            101
        } else {
            self.status
        };
        Ok(Supervision::Exited(
            exit(status),
            usize::try_from(selected + self.announce_offset).ok(),
        ))
    }
}
