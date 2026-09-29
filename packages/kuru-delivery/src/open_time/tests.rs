use std::collections::{BTreeMap, HashSet};

use super::census::{self, parse_lsof, parse_netstat, parse_proc_net_tcp, socket_inode};
use super::observe::{
    FileSpan, Observation, ProcessSpan, Role, Seen, Store, classify, normalize, unentered, walk,
};
use super::report::{Counts, Derived, OwnerPath, derive, stats, summary};
use super::{Case, Line, Record};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn statistics_report_median_extremes_and_spread() {
    assert!(stats(&[]).is_none());
    let odd = stats(&[5.0, 1.0, 3.0]).unwrap();
    assert_eq!(
        (odd.n, odd.median, odd.min, odd.max, odd.spread),
        (3, 3.0, 1.0, 5.0, 4.0)
    );
    let even = stats(&[4.0, 1.0, 2.0, 10.0]).unwrap();
    assert_eq!(even.median, 3.0);
    assert_eq!(even.spread, 9.0);
    let single = stats(&[7.5]).unwrap();
    assert_eq!((single.median, single.spread), (7.5, 0.0));
}

#[test]
fn roles_come_from_the_image_name_and_its_arguments() {
    let service = args(&["--internal-memory-service", "p", "d"]);
    assert_eq!(classify("kuru", &service), Some(Role::Owner));
    assert_eq!(
        classify("kuru", &args(&["--internal-dolt-supervisor"])),
        Some(Role::Supervisor)
    );
    assert_eq!(
        classify("kuru", &args(&["-C", "p", "run", "measure", "--json"])),
        Some(Role::Cli)
    );
    assert_eq!(
        classify("dolt", &args(&["sql-server", "--config", "s.yaml"])),
        Some(Role::SqlServer)
    );
    assert_eq!(
        classify("dolt", &args(&["version"])),
        Some(Role::DoltVersion)
    );
    assert_eq!(classify("dolt", &args(&["log"])), Some(Role::OtherDolt));
    assert_eq!(classify("kuru-delivery", &service), None);
    assert_eq!(classify("bash", &args(&["kuru"])), None);
}

#[test]
fn engine_store_kind_comes_from_its_configuration_path() {
    let staging = args(&[
        "sql-server",
        "--config",
        "/r/data/memory/ab.staging-1/server.yaml",
    ]);
    assert_eq!(Store::of(&staging), Some(Store::Staging));
    let active = args(&["sql-server", "--config", "/r/data/memory/ab/server.yaml"]);
    assert_eq!(Store::of(&active), Some(Store::Active));
    assert_eq!(Store::of(&args(&["sql-server"])), None);
}

#[test]
fn file_keys_hide_project_hashes_stage_identities_and_install_suffixes() {
    let hash = "6c73e8a70c34a16c8587bbcb1a59d81a3181cb6ef8fb6d649739d5031623e48c";
    assert_eq!(
        normalize(&format!(
            "data/memory/{hash}.staging-40abe117-205d-4fa8-9f78-3aef3e63e21d/ready.json"
        )),
        "data/memory/<project>.staging-<id>/ready.json"
    );
    assert_eq!(
        normalize(&format!("data/memory/services/{hash}/endpoint.json")),
        "data/memory/services/<project>/endpoint.json"
    );
    assert_eq!(
        normalize("cache/2.3.5/.install-TDpOEW/private/probe/dolt"),
        "cache/2.3.5/.install-<tmp>/private/probe/dolt"
    );
    assert_eq!(
        normalize("cache/2.3.5/aarch64-apple-darwin/dolt"),
        "cache/2.3.5/aarch64-apple-darwin/dolt"
    );
    assert_eq!(
        normalize(&format!(
            "data/memory/{hash}.staging-1/staging/record-8298b058-c66d-41f0-85ca-adce2e890c2b.tmp"
        )),
        "data/memory/<project>.staging-1/staging/record-<id>.tmp"
    );
    assert_eq!(
        normalize("data/memory/x/data/.dolt/sql-server.info-4179905629"),
        "data/memory/x/data/.dolt/sql-server.info-<n>"
    );
    assert_eq!(normalize("cache/.install.lock"), "cache/.install.lock");
    // A shorter hexadecimal name, a short number and a near-UUID are kept.
    assert_eq!(normalize("data/abc123"), "data/abc123");
    assert_eq!(normalize("data/log-12"), "data/log-12");
    assert_eq!(
        normalize("data/8298b058-c66d-41f0-85ca-adce2e890c2"),
        "data/8298b058-c66d-41f0-85ca-adce2e890c2"
    );
}

fn process(role: Role, store: Option<Store>, first: f64, last: f64) -> ProcessSpan {
    ProcessSpan {
        pid: 1,
        role,
        store,
        preexisting: false,
        seen_before_exec_as: None,
        first_seen_ms: first,
        not_seen_ms: Some(first - 20.0),
        last_seen_ms: last,
        gone_by_ms: Some(last + 20.0),
        samples: ((last - first) / 20.0).round() as u64 + 1,
    }
}

fn file(key: &str, appeared: f64, vanished: Option<f64>) -> FileSpan {
    FileSpan {
        key: key.to_owned(),
        appeared_by_ms: appeared,
        absent_at_ms: Some(appeared - 10.0),
        last_present_ms: vanished.map(|vanished| vanished - 10.0),
        vanished_by_ms: vanished,
    }
}

fn line(t_ms: f64, text: &str) -> Line {
    Line {
        t_ms,
        text: text.to_owned(),
    }
}

/// A first launch as the observer sees it: stage directories only from
/// their parents, never entered.
fn first_launch_observation() -> Observation {
    let staging = Some(Store::Staging);
    Observation {
        processes: vec![
            process(Role::Cli, None, 5.0, 10_300.0),
            process(Role::Owner, None, 1_000.0, 10_300.0),
            process(Role::DoltVersion, None, 1_800.0, 2_600.0),
            process(Role::Supervisor, None, 2_950.0, 4_900.0),
            process(Role::SqlServer, staging, 2_970.0, 4_880.0),
            process(Role::Supervisor, None, 5_150.0, 7_540.0),
            process(Role::SqlServer, staging, 5_170.0, 7_390.0),
            process(Role::Supervisor, None, 7_700.0, 8_280.0),
            process(Role::SqlServer, staging, 7_720.0, 8_260.0),
            // Seen on one sample only: counted, never treated as an engine.
            process(Role::SqlServer, staging, 7_720.0, 7_720.0),
            process(Role::Supervisor, None, 8_530.0, 10_300.0),
            process(Role::SqlServer, Some(Store::Active), 8_540.0, 10_300.0),
        ],
        files: vec![
            file("cache/2.3.5/.install-<tmp>", 1_010.0, Some(2_900.0)),
            file("cache/2.3.5/aarch64-apple-darwin/dolt", 2_760.0, None),
            file("data/memory/<project>.staging-<id>", 2_920.0, Some(8_500.0)),
            file("data/memory/<project>", 8_500.0, None),
            file(
                "data/memory/<project>/data/.dolt/sql-server.info",
                8_600.0,
                None,
            ),
            file("data/memory/<project>/endpoint.json", 8_690.0, None),
            file(
                "data/memory/services/<project>/endpoint.json",
                9_460.0,
                None,
            ),
        ],
        files_observed: true,
        ticks: 500,
        interval_ms: 20.0,
        max_gap_ms: 31.0,
        max_bracket_ms: 33.0,
        mean_tick_cost_ms: 2.0,
    }
}

fn first_launch() -> Derived {
    let lines = [
        line(990.0, "Memory: waiting for project ownership…"),
        line(9_500.0, "Memory: ready."),
    ];
    derive(&first_launch_observation(), &lines, Some(10_320.0))
}

/// The partition property: consecutive stages whose sum is the open, each
/// within its own bounds.
fn assert_partitions(derived: &Derived) {
    let stages = &derived.stages;
    let ready = stages.ready_ms.expect("the run opened");
    assert!(
        (stages.partition_sum_ms() - ready).abs() < 0.05,
        "partition sums to {} for an open of {ready}: {:?}",
        stages.partition_sum_ms(),
        stages.partition
    );
    assert_eq!(stages.partition.len() + 1, stages.milestones.len());
    assert_eq!(stages.milestones.first().map(|m| m.at_ms), Some(0.0));
    assert_eq!(stages.milestones.last().map(|m| m.at_ms), Some(ready));
    for (stage, pair) in stages.partition.iter().zip(stages.milestones.windows(2)) {
        assert_eq!(stage.name, format!("{} → {}", pair[0].name, pair[1].name));
        assert!(
            stage.min_ms <= stage.ms && stage.ms <= stage.max_ms,
            "{stage:?}"
        );
    }
    for total in &stages.totals {
        let parts: f64 = stages
            .partition
            .iter()
            .filter(|stage| total.parts.contains(&stage.name))
            .map(|stage| stage.ms)
            .sum();
        assert!((parts - total.ms).abs() < 0.05, "{total:?}");
    }
}

#[test]
fn a_first_launch_is_partitioned_at_each_observed_milestone() {
    let derived = first_launch();
    let stages = &derived.stages;
    assert_partitions(&derived);
    assert_eq!(stages.cli_preamble_ms, Some(990.0));
    assert_eq!(stages.ready_ms, Some(9_500.0));
    let names: Vec<&str> = stages
        .milestones
        .iter()
        .map(|milestone| milestone.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "start",
            "progress line",
            "owner seen",
            "install stage appeared",
            "`dolt version` started",
            "`dolt version` exited",
            "engine activated",
            "install stage removed",
            "store stage appeared",
            "staging supervisor 1 started",
            "staging engine 1 started",
            "staging engine 1 exited",
            "staging supervisor 1 exited",
            "staging supervisor 2 started",
            "staging engine 2 started",
            "staging engine 2 exited",
            "staging supervisor 2 exited",
            "staging supervisor 3 started",
            "staging engine 3 started",
            "staging engine 3 exited",
            "staging supervisor 3 exited",
            "active store appeared",
            "active supervisor started",
            "active engine started",
            "active engine endpoint",
            "service endpoint",
            "`Memory: ready.`",
        ]
    );
    let stage = |name: &str| {
        stages
            .partition
            .iter()
            .find(|stage| stage.name == name)
            .map(|stage| (stage.ms, stage.min_ms, stage.max_ms))
    };
    // Extraction, the probe and activation no longer overlap.
    assert_eq!(
        stage("install stage appeared → `dolt version` started"),
        Some((790.0, 770.0, 800.0))
    );
    assert_eq!(
        stage("`dolt version` started → `dolt version` exited"),
        Some((820.0, 800.0, 840.0))
    );
    assert_eq!(
        stage("`dolt version` exited → engine activated"),
        Some((140.0, 130.0, 160.0))
    );
    // An engine's close and its supervisor's reap precede the rename.
    assert_eq!(
        stage("staging supervisor 3 exited → active store appeared"),
        Some((200.0, 190.0, 220.0))
    );
    assert_eq!(
        stage("service endpoint → `Memory: ready.`"),
        Some((40.0, 40.0, 50.0))
    );
    let total = |name: &str| {
        stages
            .totals
            .iter()
            .find(|total| total.name.starts_with(name))
            .map(|total| (total.ms, total.parts.len()))
    };
    assert_eq!(total("engine provisioning"), Some((1_890.0, 4)));
    assert_eq!(total("staged store creation"), Some((5_580.0, 13)));
    assert_eq!(total("active open"), Some((930.0, 3)));
    assert_eq!(total("owner seen to service endpoint"), Some((8_460.0, 23)));
    assert_eq!(total("staging supervisor 2 lifetime"), Some((2_410.0, 3)));
    // Lifetimes are brackets; engines carry no endpoint split any more.
    let probe = stages.probe.clone().unwrap();
    assert_eq!((probe.lower_ms, probe.upper_ms), (Some(800.0), Some(840.0)));
    assert_eq!(stages.engines.len(), 4);
    assert_eq!(stages.engines[0].lifetime.lower_ms, Some(1_910.0));
    assert_eq!(stages.turn_and_exit_ms, Some(820.0));
    assert_eq!(
        derived.counts,
        Counts {
            owners: 1,
            supervisors: 4,
            engine_starts: 4,
            transient_engine_processes: 1,
            staging_engine_starts: 3,
            active_engine_starts: 1,
            dolt_version: 1,
            engine_marker_intervals: 1,
        }
    );
    assert_eq!(derived.path, OwnerPath::SpawnedOwner);
}

#[test]
fn a_missing_milestone_merges_its_neighbours_and_the_partition_still_holds() {
    let lines = [
        line(990.0, "Memory: waiting for project ownership…"),
        line(9_500.0, "Memory: ready."),
    ];
    let full = first_launch_observation();
    // Remove each process and each name in turn.
    for index in 0..full.processes.len() {
        let mut observation = full.clone();
        observation.processes.remove(index);
        assert_partitions(&derive(&observation, &lines, Some(10_320.0)));
    }
    for index in 0..full.files.len() {
        let mut observation = full.clone();
        let removed = observation.files.remove(index);
        let derived = derive(&observation, &lines, Some(10_320.0));
        assert_partitions(&derived);
        if removed.key == "cache/2.3.5/aarch64-apple-darwin/dolt" {
            assert!(
                derived
                    .stages
                    .partition
                    .iter()
                    .any(|stage| stage.name == "`dolt version` exited → install stage removed"),
                "{:?}",
                derived.stages.partition
            );
        }
    }
    // Without the progress line the first stage runs from the start to the
    // owner.
    let derived = derive(&full, &lines[1..], Some(10_320.0));
    assert_partitions(&derived);
    assert_eq!(derived.stages.partition[0].name, "start → owner seen");
}

#[test]
fn a_single_sighting_has_no_lower_bound_and_a_bracketed_upper_bound() {
    let span = process(Role::DoltVersion, None, 1_000.0, 1_000.0);
    assert_eq!(span.samples, 1);
    assert_eq!(span.lifetime_lower_ms(), None);
    assert_eq!(span.lifetime_upper_ms(), Some(40.0));
    let observation = Observation {
        processes: vec![span],
        ..Observation::default()
    };
    let derived = derive(
        &observation,
        &[line(1_100.0, "Memory: ready.")],
        Some(1_200.0),
    );
    let probe = derived.stages.probe.clone().unwrap();
    assert_eq!((probe.lower_ms, probe.upper_ms), (None, Some(40.0)));
    assert_partitions(&derived);
}

/// Real observations from the local smoke run of the release binary at
/// a8ce468b (the first iteration of the main series and the control's first
/// launch), trimmed to what [`derive`] reads.
#[test]
fn recorded_observations_are_partitioned_exactly() {
    #[derive(serde::Deserialize)]
    struct Recorded {
        case: String,
        stderr: Vec<Line>,
        exit_ms: Option<f64>,
        observation: Observation,
    }
    let recorded: Vec<Recorded> = include_str!("testdata/recorded.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(recorded.len() >= 4);
    assert!(recorded.iter().any(|run| !run.observation.files_observed));
    let mut cases = HashSet::new();
    for run in &recorded {
        cases.insert(run.case.as_str());
        let derived = derive(&run.observation, &run.stderr, run.exit_ms);
        assert_partitions(&derived);
        // Sampled stages are off by less than one recorded bracket.
        for stage in &derived.stages.partition {
            assert!(
                stage.ms >= -run.observation.max_bracket_ms,
                "{} {stage:?}",
                run.case
            );
        }
        match run.case.as_str() {
            "first-launch" => {
                assert_eq!(derived.counts.staging_engine_starts, 3, "{}", run.case);
                // Without file observation the install stage is not seen,
                // and its stages merge into their process neighbours.
                assert_eq!(
                    derived
                        .stages
                        .totals
                        .iter()
                        .any(|total| total.name.starts_with("engine provisioning")),
                    run.observation.files_observed
                );
            }
            "warm-reopen" => assert_eq!(derived.path, OwnerPath::Attached),
            _ => assert_eq!(derived.counts.engine_starts, 1, "{}", run.case),
        }
    }
    assert_eq!(cases.len(), 3);
}

#[test]
fn a_warm_reopen_that_finds_its_owner_is_an_attach() {
    let mut owner = process(Role::Owner, None, 0.0, 400.0);
    owner.preexisting = true;
    owner.not_seen_ms = None;
    let mut engine = process(Role::SqlServer, Some(Store::Active), 0.0, 400.0);
    engine.preexisting = true;
    let mut endpoint = file("data/memory/services/<project>/endpoint.json", 0.0, None);
    endpoint.absent_at_ms = None;
    let observation = Observation {
        processes: vec![owner, engine, process(Role::Cli, None, 2.0, 380.0)],
        files: vec![endpoint],
        ..Observation::default()
    };
    let lines = [
        line(90.0, "Memory: waiting for project ownership…"),
        line(140.0, "Memory: ready."),
    ];
    let derived = derive(&observation, &lines, Some(390.0));
    assert_eq!(derived.path, OwnerPath::Attached);
    assert_eq!(derived.counts.engine_starts, 0);
    let names: Vec<&str> = derived
        .stages
        .milestones
        .iter()
        .map(|milestone| milestone.name.as_str())
        .collect();
    // No owner started, and an endpoint present before the run is not a
    // publication in this run.
    assert_eq!(names, ["start", "progress line", "`Memory: ready.`"]);
    assert_eq!(derived.stages.ready_ms, Some(140.0));
    assert_partitions(&derived);
}

#[test]
fn a_failed_open_keeps_its_partial_stages_and_no_ready_time() {
    let lines = [
        line(80.0, "Memory: waiting for project ownership…"),
        line(
            30_400.0,
            "Error: memory service readiness deadline exceeded",
        ),
    ];
    let derived = derive(&Observation::default(), &lines, Some(30_410.0));
    assert_eq!(derived.stages.ready_ms, None);
    assert_eq!(derived.stages.turn_and_exit_ms, None);
    assert_eq!(derived.stages.cli_preamble_ms, Some(80.0));
    assert_eq!(derived.path, OwnerPath::NoOwnerObserved);
    // The partial partition ends at the last milestone the run showed.
    assert_eq!(derived.stages.partition_sum_ms(), 80.0);
}

fn record(case: Case, iteration: usize, ready: Option<f64>, success: bool) -> Record {
    let mut record = Record::fixture(case, iteration);
    record.stages.ready_ms = ready;
    record.timings.exit_ms = ready.map(|ready| ready + 500.0);
    record.outcome.success = success;
    record.probe.cpu_ms = 100.0;
    record.mode.files = true;
    record.mode.interval_ms = 20.0;
    record.mode.retire_wait = true;
    record
}

#[test]
fn summary_lists_every_sample_with_median_extremes_and_spread_per_case() {
    let mut opened_then_failed = record(Case::WarmReopen, 2, Some(160.0), false);
    opened_then_failed.observation.max_bracket_ms = 47.0;
    opened_then_failed.observation.mean_tick_cost_ms = 6.0;
    let records = vec![
        record(Case::FirstLaunch, 1, Some(9_000.0), true),
        record(Case::FirstLaunch, 2, Some(11_000.0), true),
        record(Case::ColdExisting, 1, Some(1_200.0), true),
        record(Case::ColdExisting, 2, None, false),
        record(Case::WarmReopen, 1, Some(150.0), true),
        opened_then_failed,
    ];
    let text = summary(&records, "unit");
    assert!(
        text.contains(
            "| first-launch | open (to `Memory: ready.`) | 2 | 10000 | 9000 | 11000 | 2000 |"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "| cold-existing | open (to `Memory: ready.`) | 1 | 1200 | 1200 | 1200 | 0 |"
        ),
        "{text}"
    );
    assert!(text.contains("first-launch: 9000, 11000"), "{text}");
    assert!(text.contains("cold-existing: 1200, failed"), "{text}");
    assert!(
        text.contains("warm-reopen: 150, 160 (command failed after opening)"),
        "{text}"
    );
    // One definition of a failed open: no `Memory: ready.`.
    assert!(text.contains("1 of 6 runs failed to open"), "{text}");
    assert!(text.contains("1 commands failed after opening"), "{text}");
    assert!(text.contains("report only"), "{text}");
    // The error bound is the recorded bracket, with the tick cost and the
    // tool's own pacing.
    assert!(text.contains("largest recorded bracket 47 ms"), "{text}");
    assert!(text.contains("mean tick cost 1.0 ms"), "{text}");
    assert!(text.contains("sampled every 20 ms"), "{text}");
    assert!(text.contains("polling every 100 ms"), "{text}");
    // Ratios to the calibration probe compare runs of one label.
    assert!(
        text.contains(
            "| first-launch | open / CPU probe (compare within one label only) | 2 | 100.0 | 90.0 | 110.0 | 20.0 |"
        ),
        "{text}"
    );
}

#[test]
fn summary_names_the_control_and_ramp_modes() {
    let mut control = record(Case::FirstLaunch, 1, Some(9_000.0), true);
    control.mode.files = false;
    control.mode.retire_wait = false;
    let text = summary(&[control], "unit control");
    assert!(text.contains("processes only (control series"), "{text}");
    assert!(text.contains("Ramp series: no run waits"), "{text}");
}

#[test]
fn the_walk_lists_stage_directories_from_their_parents_only() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let hash = "a".repeat(64);
    for directory in [
        format!("memory/{hash}.staging-40abe117-205d-4fa8-9f78-3aef3e63e21d/data/.dolt"),
        format!("memory/{hash}/data/.dolt"),
        "memory/interrupted/old.staging-1/data".to_owned(),
        "memory/services".to_owned(),
    ] {
        std::fs::create_dir_all(data.join(directory)).unwrap();
    }
    std::fs::write(
        data.join(format!(
            "memory/{hash}.staging-40abe117-205d-4fa8-9f78-3aef3e63e21d/ready.json"
        )),
        b"{}",
    )
    .unwrap();
    std::fs::write(data.join(format!("memory/{hash}/endpoint.json")), b"{}").unwrap();
    let cache = root.path().join("cache");
    std::fs::create_dir_all(cache.join("2.3.5/.install-TDpOEW/private/probe")).unwrap();
    std::fs::create_dir_all(cache.join("2.3.5/aarch64-apple-darwin")).unwrap();
    std::fs::write(cache.join("2.3.5/aarch64-apple-darwin/dolt"), b"").unwrap();
    let mut present = HashSet::new();
    walk(&data, "data", 0, &mut present);
    walk(&cache, "cache", 0, &mut present);
    for key in [
        "data/memory/<project>.staging-<id>",
        "data/memory/<project>/endpoint.json",
        "data/memory/<project>/data/.dolt",
        "data/memory/interrupted",
        "cache/2.3.5/.install-<tmp>",
        "cache/2.3.5/aarch64-apple-darwin/dolt",
    ] {
        assert!(present.contains(key), "{key} missing from {present:?}");
    }
    for key in &present {
        assert!(
            !key.contains(".staging-<id>/")
                && !key.contains(".install-<tmp>/")
                && !key.contains("interrupted/"),
            "entered {key}"
        );
    }
    assert!(unentered(".install-x") && unentered("p.staging-1") && unentered("interrupted"));
    assert!(!unentered(".install.lock") && !unentered("staging"));
}

#[test]
fn listening_ports_are_read_from_each_platform_listing() {
    let proc_net = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:D431 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 91234 1 0000000000000000 100 0 0 10 0\n   1: 0100007F:D432 0100007F:0CEA 01 00000000:00000000 00:00000000 00000000  1000        0 91235 1 0000000000000000 20 4 30 10 -1\n";
    assert_eq!(parse_proc_net_tcp(proc_net), vec![(91_234, 0xD431)]);
    assert_eq!(socket_inode("socket:[91234]"), Some(91_234));
    assert_eq!(socket_inode("/dev/null"), None);
    let lsof = "p501\nf12\nn127.0.0.1:54321\nf13\nn[::1]:54322\np502\nf7\nn*:6000\n";
    let found = parse_lsof(lsof);
    assert_eq!(found[&501], vec![54_321, 54_322]);
    assert_eq!(found[&502], vec![6_000]);
    let netstat = "\nActive Connections\n\n  Proto  Local Address          Foreign Address        State           PID\n  TCP    127.0.0.1:52345        0.0.0.0:0              LISTENING       4242\n  TCP    127.0.0.1:52346        127.0.0.1:52345        ESTABLISHED     4242\n  TCP    [::1]:52347            [::]:0                 ABHÖREN         4243\n  UDP    0.0.0.0:5353           *:*                                    4242\n";
    let found = parse_netstat(netstat);
    assert_eq!(found[&4242], vec![52_345]);
    assert_eq!(found[&4243], vec![52_347]);
    assert_eq!(found.len(), 2);
}

#[test]
fn the_census_counts_processes_by_owner_and_stage_directories_by_iteration() {
    let seen = |role, ours, mine, open_files| Seen {
        pid: 1,
        role,
        store: None,
        ours,
        image: if role == Role::SqlServer {
            "dolt".into()
        } else {
            "kuru".into()
        },
        mine,
        open_files,
    };
    let counted = census::of(&[
        seen(Role::Owner, true, Some(true), Some(40)),
        seen(Role::SqlServer, true, Some(true), Some(60)),
        seen(Role::Supervisor, true, Some(true), None),
        seen(Role::SqlServer, false, Some(true), None),
        seen(Role::Owner, false, Some(false), None),
        seen(Role::Cli, false, None, None),
    ]);
    assert_eq!(counted.ours_total(), 3);
    assert_eq!(counted.open_files_total(), 100);
    assert_eq!(counted.open_files_unread, 1);
    assert_eq!(
        counted.foreign,
        BTreeMap::from([("dolt sql-server".to_owned(), 1)])
    );
    assert_eq!(counted.owner_unknown, 1);
    assert_eq!(counted.listening_total(), None);
    let root = tempfile::tempdir().unwrap();
    for directory in [
        "iteration-01/data/memory/p.staging-1",
        "iteration-01/data/memory/p",
        "iteration-02/data/memory/p.staging-2",
        "iteration-02/data/memory/interrupted/p.staging-0",
        "other/data/memory/p.staging-3",
    ] {
        std::fs::create_dir_all(root.path().join(directory)).unwrap();
    }
    assert_eq!(census::stages(root.path()), (2, 1));
}
