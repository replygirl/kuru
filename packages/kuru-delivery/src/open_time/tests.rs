use super::observe::{FileSpan, Observation, ProcessSpan, Role, Store, classify, normalize};
use super::report::{Counts, Engine, OwnerPath, derive, stats, summary};
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
    }
}

fn file(key: &str, appeared: f64, vanished: Option<f64>) -> FileSpan {
    FileSpan {
        key: key.to_owned(),
        appeared_by_ms: appeared,
        absent_at_ms: Some(appeared - 10.0),
        vanished_by_ms: vanished,
    }
}

fn line(t_ms: f64, text: &str) -> Line {
    Line {
        t_ms,
        text: text.to_owned(),
    }
}

fn first_launch_observation() -> Observation {
    let staging = Some(Store::Staging);
    Observation {
        processes: vec![
            process(Role::Cli, None, 5.0, 10_300.0),
            process(Role::Owner, None, 1_000.0, 10_300.0),
            process(Role::DoltVersion, None, 1_800.0, 2_600.0),
            process(Role::Supervisor, None, 2_950.0, 4_900.0),
            process(Role::SqlServer, staging, 2_970.0, 4_900.0),
            process(Role::Supervisor, None, 5_150.0, 7_540.0),
            process(Role::SqlServer, staging, 5_170.0, 7_390.0),
            process(Role::Supervisor, None, 7_700.0, 8_280.0),
            process(Role::SqlServer, staging, 7_720.0, 8_280.0),
            // Seen on one sample only: counted, never treated as an engine.
            process(Role::SqlServer, staging, 7_720.0, 7_720.0),
            process(Role::Supervisor, None, 8_530.0, 10_300.0),
            process(Role::SqlServer, Some(Store::Active), 8_540.0, 10_300.0),
        ],
        files: vec![
            file("cache/2.3.5/.install-<tmp>", 1_010.0, Some(2_760.0)),
            file("cache/2.3.5/aarch64-apple-darwin/dolt", 2_760.0, None),
            file(
                "data/memory/<project>.staging-<id>/ready.json",
                8_410.0,
                Some(8_540.0),
            ),
            file("data/memory/<project>", 8_540.0, None),
            file(
                "data/memory/<project>.staging-<id>/data/.dolt/sql-server.info",
                4_780.0,
                Some(4_900.0),
            ),
            file(
                "data/memory/services/<project>/endpoint.json",
                9_460.0,
                None,
            ),
            file(
                "data/memory/<project>.staging-<id>/endpoint.json",
                4_220.0,
                Some(4_350.0),
            ),
            file(
                "data/memory/<project>.staging-<id>/endpoint.json",
                5_550.0,
                Some(7_400.0),
            ),
            file("data/memory/<project>/endpoint.json", 8_690.0, None),
        ],
        ticks: 500,
        interval_ms: 20.0,
        max_gap_ms: 31.0,
        mean_tick_cost_ms: 2.0,
    }
}

#[test]
fn a_first_launch_separates_provisioning_each_engine_and_publication() {
    let lines = [
        line(990.0, "Memory: waiting for project ownership…"),
        line(9_500.0, "Memory: ready."),
    ];
    let derived = derive(&first_launch_observation(), &lines, Some(10_320.0));
    let stages = &derived.stages;
    assert_eq!(stages.cli_preamble_ms, Some(990.0));
    assert_eq!(stages.ready_ms, Some(9_500.0));
    assert_eq!(stages.owner_seen_ms, Some(1_000.0));
    assert_eq!(stages.extract_ms, Some(1_750.0));
    assert_eq!(stages.probe_seen_ms, Some(800.0));
    assert_eq!(stages.owner_to_first_engine_ms, Some(1_970.0));
    let staging = |lifetime_ms, ready_ms, served_ms| Engine {
        store: Some(Store::Staging),
        lifetime_ms,
        ready_ms,
        served_ms,
    };
    assert_eq!(
        stages.engines,
        vec![
            staging(1_930.0, Some(1_250.0), Some(130.0)),
            staging(2_220.0, Some(380.0), Some(1_850.0)),
            // No endpoint record was seen for this one: its readiness is unknown.
            staging(560.0, None, None),
            Engine {
                store: Some(Store::Active),
                lifetime_ms: 1_760.0,
                ready_ms: Some(150.0),
                served_ms: None,
            },
        ]
    );
    assert_eq!(stages.stage_to_active_ms, Some(130.0));
    assert_eq!(stages.active_open_ms, Some(920.0));
    assert_eq!(stages.publish_to_ready_ms, Some(40.0));
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
    assert_eq!(derived.stages.owner_seen_ms, None);
    // An endpoint present before the run is not a publication in this run.
    assert_eq!(derived.stages.publish_to_ready_ms, None);
    assert_eq!(derived.stages.ready_ms, Some(140.0));
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
}

fn record(case: Case, iteration: usize, ready: Option<f64>, success: bool) -> Record {
    let mut record = Record::fixture(case, iteration);
    record.stages.ready_ms = ready;
    record.timings.exit_ms = ready.map(|ready| ready + 500.0);
    record.outcome.success = success;
    record.probe.cpu_ms = 100.0;
    record
}

#[test]
fn summary_lists_every_sample_with_median_extremes_and_spread_per_case() {
    let records = vec![
        record(Case::FirstLaunch, 1, Some(9_000.0), true),
        record(Case::FirstLaunch, 2, Some(11_000.0), true),
        record(Case::ColdExisting, 1, Some(1_200.0), true),
        record(Case::ColdExisting, 2, None, false),
        record(Case::WarmReopen, 1, Some(150.0), true),
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
    assert!(text.contains("1 of 5 runs failed to open"), "{text}");
    assert!(text.contains("report only"), "{text}");
    // Ratios to the calibration probe let runners be compared.
    assert!(
        text.contains("| first-launch | open / CPU probe | 2 | 100.0 | 90.0 | 110.0 | 20.0 |"),
        "{text}"
    );
}
