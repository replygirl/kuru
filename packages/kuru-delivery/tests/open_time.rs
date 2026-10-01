#![cfg(feature = "tooling")]
//! The open-time harness driven end to end against the delivery fixture,
//! which stands in for `kuru` as a live subprocess with streamed stderr.

use std::{fs, path::Path, time::Duration};

use kuru_delivery::open_time::{self, Options};
use serde_json::Value;

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
    fs::copy(env!("CARGO_BIN_EXE_kuru-delivery-fixture"), &binary).unwrap();
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
