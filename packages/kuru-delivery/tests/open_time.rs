#![cfg(feature = "tooling")]
//! The open-time harness driven end to end against the delivery fixture,
//! which stands in for `kuru` as a live subprocess with streamed stderr.

use std::{fs, path::Path, time::Duration};

use kuru_delivery::open_time::{self, Options};
use serde_json::Value;

#[cfg(unix)]
#[path = "support/launch_budget.rs"]
mod launch_budget;

fn options(root: &Path, iterations: usize, prompt: &str) -> Options {
    let mut options = Options::new(
        env!("CARGO_BIN_EXE_kuru-delivery-fixture").into(),
        root.join("output"),
    );
    options.scratch = Some(root.join("scratch"));
    options.iterations = iterations;
    options.label = "fixture".into();
    options.prompt = prompt.into();
    options.run_bound = Duration::from_secs(60);
    options
}

fn records(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("output/records.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_run_of_a_live_command_is_recorded_and_summarised() {
    let root = tempfile::tempdir().unwrap();
    let report = open_time::run(&options(root.path(), 2, "measure"))
        .await
        .unwrap();
    assert_eq!((report.records, report.failed_opens), (8, 0));
    let records = records(root.path());
    let cases: Vec<&str> = records
        .iter()
        .map(|record| record["case"].as_str().unwrap())
        .collect();
    assert_eq!(
        cases,
        [
            "first-launch",
            "cold-existing",
            "warm-reopen",
            "new-project",
            "first-launch",
            "cold-existing",
            "warm-reopen",
            "new-project"
        ]
    );
    for record in &records {
        assert_eq!(record["schema"], "kuru.open-time.v3");
        // The fixture writes both signals when KURU_OPEN_MARKERS=1 reaches
        // it through the cleared environment; the marker ends the open.
        assert_eq!(record["stages"]["ready_signal"], "marker", "{record}");
        assert_eq!(record["binary"]["version"], "native fixture 0.2.0");
        assert_eq!(record["outcome"]["success"], true);
        assert_eq!(record["outcome"]["json_stdout"], true);
        let ready = record["stages"]["ready_ms"].as_f64().unwrap();
        let preamble = record["stages"]["cli_preamble_ms"].as_f64().unwrap();
        let exit = record["timings"]["exit_ms"].as_f64().unwrap();
        assert!(preamble <= ready && ready <= exit, "{record}");
        assert!(record["probe"]["cpu_ms"].as_f64().unwrap() > 0.0);
        assert!(record["probe"]["io_ms"].as_f64().unwrap() > 0.0);
        assert!(record["observation"]["ticks"].as_u64().unwrap() >= 1);
        assert_eq!(record["observation"]["files_observed"], true);
        assert_eq!(record["mode"]["files"], true);
        // Five repeats of the CPU probe, and their median.
        assert_eq!(
            record["probe"]["cpu_samples_ms"].as_array().unwrap().len(),
            5
        );
        // The census before and after: the fixture has exited by then, and
        // its store directory is not a staging one.
        for census in [&record["census_before"], &record["census_after"]] {
            assert_eq!(census["ours"], serde_json::json!({}), "{record}");
            assert_eq!(census["staging_directories"], 0);
            assert_eq!(census["interrupted_directories"], 0);
            assert!(census["cost_ms"].as_f64().unwrap() >= 0.0);
        }
        // The stages partition the open.
        let sum: f64 = record["stages"]["partition"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stage| stage["ms"].as_f64().unwrap())
            .sum();
        assert!((sum - ready).abs() < 0.05, "{record}");
        // Only progress lines and markers are kept, never paths or other
        // output.
        for line in record["stderr"].as_array().unwrap() {
            let text = line["text"].as_str().unwrap();
            assert!(
                text.starts_with("Memory: ") || text.starts_with("kuru-open-marker v1 "),
                "{text}"
            );
        }
        let text = record.to_string();
        assert!(!text.contains(&*root.path().to_string_lossy()), "{text}");
    }
    // The first launch, the warm reopen and the new project wait for
    // retirement; the cold open runs right after the first launch's wait,
    // and the warm reopen inside the cold open's window.
    assert!(records[0]["retire_wait_ms"].is_number());
    assert!(records[1]["retire_wait_ms"].is_null());
    assert!(records[2]["retire_wait_ms"].is_number());
    assert!(records[3]["retire_wait_ms"].is_number());
    // The new project is a different project in the same data directory:
    // the first project's store is named apart from the first tick on, and
    // the new one is seen under `<project>`.
    for record in [&records[3], &records[7]] {
        let files = record["observation"]["files"].as_array().unwrap();
        let named = |key: &str| files.iter().find(|file| file["key"] == key);
        let prior = named("data/memory/<prior-project>").expect("prior project store");
        assert!(prior["absent_at_ms"].is_null(), "{record}");
        assert!(named("data/memory/<project>").is_some(), "{record}");
    }
    for record in [&records[1], &records[2]] {
        let files = record["observation"]["files"].as_array().unwrap();
        assert!(
            files
                .iter()
                .all(|file| !file["key"].as_str().unwrap().contains("<prior-project>")),
            "{record}"
        );
    }
    assert!(records[0]["owner_retired_ms"].is_number());
    assert_eq!(report.final_retire_wait_ms, None);
    assert!(records[1]["gap_since_previous_ms"].is_number());
    let summary = fs::read_to_string(root.path().join("output/summary.md")).unwrap();
    assert_eq!(summary, report.summary);
    assert!(summary.contains("report only"), "{summary}");
    assert!(
        summary.contains("| warm-reopen | open (to ready) | 2 |"),
        "{summary}"
    );
    // The scratch root and every iteration directory are removed.
    assert_eq!(
        fs::read_dir(root.path().join("scratch")).unwrap().count(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_open_is_a_result_and_the_series_completes() {
    let root = tempfile::tempdir().unwrap();
    let report = open_time::run(&options(root.path(), 1, "fail"))
        .await
        .unwrap();
    assert_eq!((report.records, report.failed_opens), (4, 4));
    for record in records(root.path()) {
        assert_eq!(record["outcome"]["success"], false);
        assert_eq!(record["outcome"]["exit_code"], 1);
        // The markers before the error are progress, never the error line.
        assert_eq!(
            record["outcome"]["error"],
            "Error: memory service readiness deadline exceeded"
        );
        assert!(record["stages"]["ready_ms"].is_null());
        assert!(record["stages"]["ready_signal"].is_null());
    }
    assert!(report.summary.contains("4 of 4 runs failed to open"));
}

#[tokio::test]
async fn a_missing_binary_is_an_infrastructure_failure() {
    let root = tempfile::tempdir().unwrap();
    let mut options = options(root.path(), 1, "measure");
    options.binary = root.path().join("absent");
    let error = open_time::run(&options).await.unwrap_err();
    assert!(
        format!("{error:#}").contains("measured binary"),
        "{error:#}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_process_still_running_from_the_root_is_a_bounded_failure() {
    use kuru_delivery::open_time::observe::{Processes, wait_retired};
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncWriteExt;

    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let binary = root_path.join("bin/kuru");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    // A sibling's fork can inherit a writable copy descriptor until exec,
    // even with CLOEXEC, making Linux refuse this executable with ETXTBSY.
    // Only the copying child opens the destination for writing; awaiting its
    // exit leaves no such descriptor in the test parent for siblings to copy.
    let mut copy = kuru_delivery::command::Command::new("/bin/cp");
    copy.arg(env!("CARGO_BIN_EXE_kuru-delivery-fixture"))
        .arg(&binary);
    let copied = kuru_delivery::command::output(&mut copy, launch_budget::until_job_deadline())
        .await
        .expect("fixture executable copy failed to start or finish");
    assert!(
        copied.status.success(),
        "fixture executable copy failed: {}",
        String::from_utf8_lossy(&copied.stderr)
    );
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    // The fixture's copy mode runs until its input closes.
    let mut child = tokio::process::Command::new(&binary)
        .arg("copy")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"held").await.unwrap();
    let services = root_path.join("services");
    let mut processes = Processes::new(&root_path);
    let error = wait_retired(
        &mut processes,
        &services,
        Duration::from_millis(300),
        Duration::from_millis(20),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("did not retire"), "{error}");
    assert!(
        error.contains(&format!("cli {}", child.id().unwrap())),
        "{error}"
    );
    // The census sees the live process from the root, reads its open files
    // and queries its listening ports through this platform's listing.
    let census = open_time::census::take(&mut processes, &root_path).await;
    assert_eq!(census.ours.get("cli"), Some(&1), "{census:?}");
    assert_eq!(census.open_files_unread, 0, "{census:?}");
    assert!(census.open_files_total() > 0, "{census:?}");
    assert_eq!(
        census.listening_tcp,
        Some(std::collections::BTreeMap::new()),
        "{census:?}"
    );
    drop(input);
    assert!(child.wait().await.unwrap().success());
    // A published service endpoint alone also keeps the wait going.
    fs::create_dir_all(services.join("project")).unwrap();
    fs::write(services.join("project/endpoint.json"), b"{}").unwrap();
    let error = wait_retired(
        &mut processes,
        &services,
        Duration::from_millis(100),
        Duration::from_millis(20),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("service endpoint present: true"), "{error}");
    fs::remove_file(services.join("project/endpoint.json")).unwrap();
    let waited = wait_retired(
        &mut processes,
        &services,
        Duration::from_secs(30),
        Duration::from_millis(20),
    )
    .await
    .unwrap();
    assert!(waited.wait_ms >= 0.0);
    assert!(waited.owner_retired_ms <= waited.wait_ms);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_control_series_lists_no_files_and_the_ramp_waits_once() {
    let root = tempfile::tempdir().unwrap();
    let mut control = options(root.path(), 1, "measure");
    control.files = false;
    control.interval = Duration::from_millis(200);
    let report = open_time::run(&control).await.unwrap();
    assert_eq!((report.records, report.failed_opens), (4, 0));
    for record in records(root.path()) {
        assert_eq!(record["observation"]["files_observed"], false);
        assert_eq!(record["observation"]["files"], serde_json::json!([]));
        assert_eq!(record["observation"]["interval_ms"], 200.0);
        assert_eq!(record["mode"]["files"], false);
    }
    assert!(
        report.summary.contains("control series"),
        "{}",
        report.summary
    );
    assert!(
        report.summary.contains("sampled every 200 ms"),
        "{}",
        report.summary
    );

    let ramp_root = tempfile::tempdir().unwrap();
    let mut ramp = options(ramp_root.path(), 2, "measure");
    ramp.retire_wait = false;
    let report = open_time::run(&ramp).await.unwrap();
    assert_eq!((report.records, report.failed_opens), (8, 0));
    for record in records(ramp_root.path()) {
        assert!(record["retire_wait_ms"].is_null(), "{record}");
        assert_eq!(record["mode"]["retire_wait"], false);
    }
    assert!(report.final_retire_wait_ms.is_some());
    assert!(report.summary.contains("Ramp series"), "{}", report.summary);
    assert!(
        report
            .summary
            .contains("Final wait for every owner of the ramp to retire"),
        "{}",
        report.summary
    );
    // Every iteration directory is removed after the final wait.
    assert_eq!(
        fs::read_dir(ramp_root.path().join("scratch"))
            .unwrap()
            .count(),
        0
    );
}

/// A binary from before the marker change (legacy lines only), one from after
/// it (markers and the plain sentence, no `Memory: ` lines), and one of each
/// whose open fails: each run records which signal ended its open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn either_readiness_signal_ends_the_open_and_the_record_names_it() {
    for (prompt, signal, failed) in [
        ("legacy", Some("legacy"), 0),
        ("markers", Some("marker"), 0),
        ("markers-fail", None, 4),
        ("legacy-fail", None, 4),
    ] {
        let root = tempfile::tempdir().unwrap();
        let report = open_time::run(&options(root.path(), 1, prompt))
            .await
            .unwrap();
        assert_eq!(
            (report.records, report.failed_opens),
            (4, failed),
            "{prompt}"
        );
        for record in records(root.path()) {
            let texts: Vec<&str> = record["stderr"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line["text"].as_str().unwrap())
                .collect();
            let markers = texts
                .iter()
                .filter(|text| text.starts_with("kuru-open-marker v1 "))
                .count();
            let legacy = texts
                .iter()
                .filter(|text| text.starts_with("Memory: "))
                .count();
            match signal {
                Some(signal) => {
                    assert_eq!(record["stages"]["ready_signal"], signal, "{record}");
                    let ready = record["stages"]["ready_ms"].as_f64().unwrap();
                    let last = record["stages"]["milestones"]
                        .as_array()
                        .unwrap()
                        .last()
                        .unwrap()
                        .clone();
                    assert_eq!(last["name"], "ready", "{record}");
                    assert_eq!(last["at_ms"].as_f64().unwrap(), ready, "{record}");
                    assert_eq!(record["outcome"]["success"], true);
                }
                None => {
                    assert!(record["stages"]["ready_signal"].is_null(), "{record}");
                    assert!(record["stages"]["ready_ms"].is_null(), "{record}");
                    assert_eq!(
                        record["outcome"]["error"],
                        "Error: memory service readiness deadline exceeded",
                        "{record}"
                    );
                }
            }
            if prompt.starts_with("markers") {
                assert_eq!(legacy, 0, "{record}");
                assert!(markers >= 2, "{record}");
                // The plain sentence at open start is kept as progress.
                assert!(
                    texts.contains(&"Opening this project's memory…"),
                    "{record}"
                );
                assert_eq!(
                    record["stages"]["markers"][0]["event"], "open-start",
                    "{record}"
                );
                assert!(record["stages"]["markers"][0]["monotonic_ns"].is_u64());
            } else {
                assert_eq!(markers, 0, "{record}");
                assert!(legacy >= 1, "{record}");
            }
        }
    }
}

/// The `kuru-delivery open-time` command against the fixture, with only the
/// given `KURU_OPEN_TIME_*` variables: none is inherited, and no CI job
/// summary is written.
async fn command(root: &Path, gate: &[(&str, &str)]) -> std::process::Output {
    use kuru_delivery::command::{Command, output};
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    command.arg("open-time").kill_on_drop(true);
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy();
        if text.starts_with("KURU_OPEN_TIME_") || text == "GITHUB_STEP_SUMMARY" {
            command.env_remove(&name);
        }
    }
    command
        .env(
            "KURU_OPEN_TIME_BINARY",
            env!("CARGO_BIN_EXE_kuru-delivery-fixture"),
        )
        .env("KURU_OPEN_TIME_OUTPUT", root.join("output"))
        .env("KURU_OPEN_TIME_SCRATCH", root.join("scratch"))
        .env("KURU_OPEN_TIME_ITERATIONS", "1")
        .env("KURU_OPEN_TIME_LABEL", "fixture");
    for (name, value) in gate {
        command.env(name, value);
    }
    output(&mut command, Duration::from_secs(600))
        .await
        .expect("open-time command exceeded its fixture deadline or failed to start")
}

/// The fixture starts no owner and no engine, so every case starts 0
/// engines and the warm reopen observes no owner.
fn fixture_gate(first_launch: &'static str) -> Vec<(&'static str, &'static str)> {
    vec![
        ("KURU_OPEN_TIME_EXPECT_FIRST_LAUNCH_STARTS", first_launch),
        ("KURU_OPEN_TIME_EXPECT_COLD_EXISTING_STARTS", "0"),
        (
            "KURU_OPEN_TIME_EXPECT_WARM_REOPEN_STARTS",
            "no-owner-observed=0",
        ),
        ("KURU_OPEN_TIME_EXPECT_NEW_PROJECT_STARTS", "0"),
        ("KURU_OPEN_TIME_BUDGET_NEW_PROJECT_MS", "60000"),
        ("KURU_OPEN_TIME_BUDGET_COLD_EXISTING_MS", "60000"),
    ]
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_gate_passes_and_fails_the_command_after_writing_every_record() {
    let root = tempfile::tempdir().unwrap();
    let passed = command(root.path(), &fixture_gate("0")).await;
    let (stdout, stderr) = (text(&passed.stdout), text(&passed.stderr));
    assert!(passed.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("Open-time gate passed."), "{stdout}");
    assert_eq!(records(root.path()).len(), 4);
    let summary = fs::read_to_string(root.path().join("output/summary.md")).unwrap();
    assert!(summary.contains("(gate)"), "{summary}");
    assert!(summary.contains("## Open-time gate: passed"), "{summary}");
    assert!(
        summary.contains("Open-time gate decision: passed, decided by the first series"),
        "{summary}"
    );
    // A pass measures no second series.
    assert!(!root.path().join("output/retry").exists());

    let root = tempfile::tempdir().unwrap();
    let failed = command(root.path(), &fixture_gate("3")).await;
    let (stdout, stderr) = (text(&failed.stdout), text(&failed.stderr));
    assert!(!failed.status.success(), "{stdout}\n{stderr}");
    assert!(stderr.contains("open-time gate failed"), "{stderr}");
    assert!(
        stderr.contains("first-launch: 0 engine starts, expected 3, in iteration 1"),
        "{stderr}"
    );
    // Every record and the summary are written before the command fails.
    assert_eq!(records(root.path()).len(), 4);
    let summary = fs::read_to_string(root.path().join("output/summary.md")).unwrap();
    assert!(summary.contains("## Open-time gate: failed"), "{summary}");
    assert!(summary.contains("Medians not checked"), "{summary}");
    // A count violation fails at once: no second series.
    assert!(!root.path().join("output/retry").exists());
    assert!(!summary.contains("second series (gate)"), "{summary}");
    assert!(
        summary.contains("Open-time gate decision: failed, decided by the first series"),
        "{summary}"
    );
}

fn records_in(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Matching counts with a 0 ms budget: the first series misses only a median
/// budget, so one more full series is measured beside it, misses too and
/// decides.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_median_miss_measures_one_more_series_which_decides() {
    let root = tempfile::tempdir().unwrap();
    let mut gate = fixture_gate("0");
    for (name, value) in &mut gate {
        if name.starts_with("KURU_OPEN_TIME_BUDGET_") {
            *value = "0";
        }
    }
    let failed = command(root.path(), &gate).await;
    let (stdout, stderr) = (text(&failed.stdout), text(&failed.stderr));
    assert!(!failed.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("Recorded 8 runs in 2 series"),
        "{stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("open-time gate failed, decided by the second series"),
        "{stderr}"
    );
    assert!(stderr.contains("second series (decides):"), "{stderr}");
    assert!(stderr.contains("over the 0 ms budget"), "{stderr}");

    // Each series' records in its own file, every case once per series.
    let output = root.path().join("output");
    for path in [
        output.join("records.jsonl"),
        output.join("retry/records.jsonl"),
    ] {
        let records = records_in(&path);
        let cases: Vec<&str> = records
            .iter()
            .map(|record| record["case"].as_str().unwrap())
            .collect();
        assert_eq!(
            cases,
            [
                "first-launch",
                "cold-existing",
                "warm-reopen",
                "new-project"
            ],
            "{}",
            path.display()
        );
    }

    // Both series and the decision in the one summary, in order.
    let summary = fs::read_to_string(output.join("summary.md")).unwrap();
    let order = [
        "## Kuru open time: fixture, first series (gate)",
        "## Open-time gate, first series: failed",
        "Open-time gate decision: a second series decides",
        "## Kuru open time: fixture, second series (gate)",
        "## Open-time gate, second series: failed",
        "Open-time gate decision: failed, decided by the second series",
    ];
    let mut from = 0;
    for heading in order {
        let at = summary[from..]
            .find(heading)
            .unwrap_or_else(|| panic!("{heading} missing after byte {from}:\n{summary}"));
        from += at + heading.len();
    }
    assert!(!summary.contains("## Open-time gate: "), "{summary}");
    // The scratch roots are removed after a completed measurement.
    assert_eq!(
        fs::read_dir(root.path().join("scratch")).unwrap().count(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_gate_variables_the_command_is_report_only_and_a_partial_gate_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let report = command(root.path(), &[]).await;
    let stdout = text(&report.stdout);
    assert!(
        report.status.success(),
        "{stdout}\n{}",
        text(&report.stderr)
    );
    assert!(stdout.contains("(report only)"), "{stdout}");
    assert!(stdout.contains("No budget is applied."), "{stdout}");
    assert!(!stdout.contains("Open-time gate"), "{stdout}");

    // One variable without the rest is refused before anything is measured.
    let root = tempfile::tempdir().unwrap();
    let partial = command(
        root.path(),
        &[("KURU_OPEN_TIME_BUDGET_NEW_PROJECT_MS", "890")],
    )
    .await;
    let stderr = text(&partial.stderr);
    assert!(!partial.status.success(), "{stderr}");
    assert!(
        stderr.contains("missing KURU_OPEN_TIME_EXPECT_FIRST_LAUNCH_STARTS"),
        "{stderr}"
    );
    assert!(!root.path().join("output").exists());

    // The gate applies to the main series only.
    let root = tempfile::tempdir().unwrap();
    let mut control = fixture_gate("0");
    control.push(("KURU_OPEN_TIME_FILES", "off"));
    let refused = command(root.path(), &control).await;
    let stderr = text(&refused.stderr);
    assert!(!refused.status.success(), "{stderr}");
    assert!(stderr.contains("main series only"), "{stderr}");
    assert!(!root.path().join("output").exists());
}
