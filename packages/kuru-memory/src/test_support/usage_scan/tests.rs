use super::*;
use crate::test_support::{
    FixtureDeadline, await_managed_quiescence, fixture_deadline, tempdir, warm_runtime_cache,
};

/// A timeline as a gated owner writes it for an existing-store open (the
/// shape `open_timeline::Timeline::encode` produces), with the first usage
/// scan from 600 to 4,600 µs.
const RECORDED: &str = r#"{"format":"kuru.open-timeline","format_version":1,"kuru_version":"0.9.0","service_generation":"0b5ad3b4-6f0e-4b52-9d43-5d3a2f0c4e11","anchor_unix_ns":1790000000000000000,"events":[{"event":"owner-main","ns":0},{"event":"owner-lock","ns":40000},{"event":"cache-verify-start","ns":50000},{"event":"cache-verify-end","ns":90000},{"event":"supervisor-ready","ns":200000},{"event":"probe-verified","ns":300000},{"event":"main-pool","ns":400000},{"event":"version-read","ns":450000},{"event":"validate-active","ns":500000},{"event":"candidate-recovery","ns":550000},{"event":"usage-pool","ns":600000},{"event":"usage-scan-1","ns":4600000},{"event":"usage-upgrade","ns":4700000},{"event":"usage-validate","ns":4800000},{"event":"usage-scan-2","ns":8300000},{"event":"store-ready","ns":8400000},{"event":"listener-bound","ns":8500000},{"event":"endpoint-published","ns":9000000}],"counts":{"usage_rows":4000},"dropped":0,"late":0}"#;

fn recorded() -> serde_json::Value {
    serde_json::from_str(RECORDED).unwrap()
}

fn bytes(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn inputs(root: &Path) -> KeyInputs<'_> {
    KeyInputs {
        fixture_format: 1,
        seed: 1,
        turns: 1,
        sizes: &[1_000, 5_000],
        report_format: 1,
        schema: 8,
        usage_schema: 4,
        engine_version: "2.3.5",
        archive_sha256: "a",
        executable_sha256: "b",
        target: "x86_64-unknown-linux-gnu",
        root,
    }
}

#[test]
fn the_key_is_stable_for_equal_inputs_and_changes_with_each_input() {
    let root = Path::new("/r");
    let base = compose_key(&inputs(root));
    assert_eq!(base, compose_key(&inputs(root)));
    let shape = base.strip_prefix("usage-scan-fixture-v1-").unwrap();
    assert_eq!(shape.len(), 64);
    assert!(
        shape
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    let other = Path::new("/s");
    let variants: Vec<(&str, KeyInputs<'_>)> = vec![
        (
            "fixture format",
            KeyInputs {
                fixture_format: 2,
                ..inputs(root)
            },
        ),
        (
            "seed",
            KeyInputs {
                seed: 2,
                ..inputs(root)
            },
        ),
        (
            "turns",
            KeyInputs {
                turns: 2,
                ..inputs(root)
            },
        ),
        (
            "sizes",
            KeyInputs {
                sizes: &[1_000, 5_001],
                ..inputs(root)
            },
        ),
        (
            "size count",
            KeyInputs {
                sizes: &[1_000],
                ..inputs(root)
            },
        ),
        (
            "report format",
            KeyInputs {
                report_format: 2,
                ..inputs(root)
            },
        ),
        (
            "schema",
            KeyInputs {
                schema: 9,
                ..inputs(root)
            },
        ),
        (
            "usage schema",
            KeyInputs {
                usage_schema: 5,
                ..inputs(root)
            },
        ),
        (
            "engine version",
            KeyInputs {
                engine_version: "2.3.6",
                ..inputs(root)
            },
        ),
        (
            "archive digest",
            KeyInputs {
                archive_sha256: "c",
                ..inputs(root)
            },
        ),
        (
            "executable digest",
            KeyInputs {
                executable_sha256: "c",
                ..inputs(root)
            },
        ),
        (
            "target",
            KeyInputs {
                target: "aarch64-apple-darwin",
                ..inputs(root)
            },
        ),
        (
            "root",
            KeyInputs {
                root: other,
                ..inputs(root)
            },
        ),
    ];
    let mut seen = std::collections::BTreeSet::from([base.clone()]);
    for (name, variant) in variants {
        let key = compose_key(&variant);
        assert_ne!(key, base, "{name} did not change the key");
        assert!(seen.insert(key), "{name} collided with another variant");
    }
    // Framing keeps adjacent fields from trading bytes.
    assert_ne!(
        compose_key(&KeyInputs {
            archive_sha256: "ab",
            executable_sha256: "",
            ..inputs(root)
        }),
        compose_key(&KeyInputs {
            archive_sha256: "a",
            executable_sha256: "b",
            ..inputs(root)
        })
    );
}

#[test]
fn the_compiled_key_reads_this_builds_constants() {
    let root = Path::new("/fixture");
    let [schema, usage_schema] = crate::store::SCHEMA_VERSIONS;
    let key = compiled_key(&CI, root);
    assert_eq!(
        key,
        compose_key(&KeyInputs {
            fixture_format: FIXTURE_FORMAT_VERSION,
            seed: 1,
            turns: 1,
            sizes: &[1_000, 5_000],
            report_format: REPORT_FORMAT_VERSION,
            schema,
            usage_schema,
            engine_version: crate::catalog::DOLT_VERSION,
            archive_sha256: crate::catalog::BUNDLED_ASSET.archive_sha256,
            executable_sha256: crate::catalog::BUNDLED_ASSET.executable_sha256,
            target: crate::catalog::BUNDLED_ASSET.target,
            root,
        })
    );
    assert_ne!(key, compiled_key(&CI, Path::new("/elsewhere")));
    assert_eq!(
        (CI.expected_rows(1_000), CI.expected_rows(5_000)),
        (4_000, 20_000)
    );
}

#[test]
fn a_recorded_timeline_parses_into_its_intervals() -> Result<()> {
    let sample = parse_timeline(RECORDED.as_bytes())?;
    assert_eq!(
        sample,
        Sample {
            generation: "0b5ad3b4-6f0e-4b52-9d43-5d3a2f0c4e11".into(),
            scan1_ns: 4_000_000,
            scan2_ns: 3_500_000,
            usage_block_ns: 7_700_000,
            open_ns: 9_000_000,
            usage_rows: 4_000,
        }
    );
    Ok(())
}

#[test]
fn ambiguous_timelines_are_refused() {
    let mut cases: Vec<(&str, serde_json::Value, &str)> = Vec::new();
    let mut value = recorded();
    value["format"] = json!("kuru.other");
    cases.push(("format", value, "is not kuru.open-timeline v1"));
    let mut value = recorded();
    value["format_version"] = json!(2);
    cases.push(("version", value, "is not kuru.open-timeline v1"));
    let mut value = recorded();
    value["dropped"] = json!(3);
    cases.push(("dropped", value, "dropped 3 stamps"));
    let mut value = recorded();
    value["counts"]["usage_rows"] = serde_json::Value::Null;
    cases.push(("rows", value, "no usage row count"));
    let mut value = recorded();
    value["events"]
        .as_array_mut()
        .unwrap()
        .retain(|stamp| stamp["event"] != "usage-scan-1");
    cases.push(("missing", value, "no usage-scan-1 event"));
    let mut value = recorded();
    value["events"][11]["ns"] = json!(1);
    cases.push(("decreasing", value, "offsets decrease"));
    let mut value = recorded();
    let events = value["events"].as_array_mut().unwrap();
    events.insert(11, json!({"event": "usage-pool", "ns": 600_000}));
    cases.push(("repeated", value, "repeats usage-pool"));
    for (name, value, expected) in cases {
        let error = parse_timeline(&bytes(&value)).unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{name}: {error:#}");
    }
    let error = parse_timeline(b"not json").unwrap_err();
    assert!(format!("{error:#}").contains("not JSON"), "{error:#}");
}

#[test]
fn the_age_report_is_the_one_aged_store_line_of_the_task_output() -> Result<()> {
    let line = super::super::aged_store::report_line(Counts::expected(&CI.plan(1_000)), 1234)?;
    let output = format!("/target/kuru-bundles/ad1e.archive\n{line}\n");
    let report = age_report(output.as_bytes())?;
    assert_eq!(
        (report.conversations, report.usage_rows, report.elapsed_ms),
        (1_000, 4_000, 1234)
    );
    for (output, expected) in [
        (
            "/target/kuru-bundles/ad1e.archive\n".to_owned(),
            "no kuru.aged-store line",
        ),
        (format!("{line}\n{line}\n"), "several"),
        (
            "{\"format\":\"other\"}\n".to_owned(),
            "no kuru.aged-store line",
        ),
    ] {
        let error = age_report(output.as_bytes()).unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
    assert!(age_report(&[0xff]).is_err());
    Ok(())
}

#[test]
fn the_median_needs_an_odd_sample() {
    assert_eq!(median(&[5, 1, 3]).unwrap(), 3);
    assert_eq!(median(&[7]).unwrap(), 7);
    assert!(median(&[]).is_err());
    assert!(median(&[1, 2]).is_err());
}

fn sample(scan1_ms: f64, rows: u64) -> Sample {
    Sample {
        generation: String::new(),
        scan1_ns: (scan1_ms * NS_PER_MS) as u64,
        scan2_ns: (scan1_ms * NS_PER_MS) as u64,
        usage_block_ns: (2.0 * scan1_ms * NS_PER_MS) as u64,
        open_ns: (3.0 * scan1_ms * NS_PER_MS) as u64,
        usage_rows: rows,
    }
}

/// Two sizes with three samples each around the given medians.
fn sizes(small_ms: f64, large_ms: f64) -> Vec<SizeSamples> {
    [(1_000, 4_000, small_ms), (5_000, 20_000, large_ms)]
        .into_iter()
        .map(|(conversations, rows, median)| SizeSamples {
            conversations,
            expected_rows: rows,
            warmups: vec![sample(median * 3.0, rows)],
            samples: vec![
                sample(median * 1.1, rows),
                sample(median, rows),
                sample(median * 0.9, rows),
            ],
            bound: vec![sample(20.0, 0), sample(21.0, 0), sample(19.0, 0)],
        })
        .collect()
}

fn check<'a>(verdict: &'a Verdict, name: &str) -> &'a Check {
    verdict
        .checks
        .iter()
        .find(|check| check.name == name)
        .unwrap()
}

#[test]
fn linear_growth_passes_and_quadratic_growth_fails_with_both_numbers() -> Result<()> {
    let linear = evaluate(&sizes(120.0, 450.0), CALIBRATED, CALIBRATED_LABEL)?;
    assert!(linear.pass, "{}", render_lines(&linear));
    assert!((linear.ratio - 3.75).abs() < 1e-6, "{}", linear.ratio);
    assert_eq!(linear.sizes[0].scan1_median_ns, 120_000_000);
    assert_eq!(linear.sizes[1].scan1_ns.len(), 3);
    assert_eq!(linear.sizes[1].bound_scan1_ns.len(), 3);
    assert_eq!(linear.sizes[1].bound_scan1_median_ns, 20_000_000);
    let quadratic = evaluate(&sizes(500.0, 12_500.0), CALIBRATED, CALIBRATED_LABEL)?;
    assert!(!quadratic.pass);
    let ratio = check(&quadratic, "ratio");
    assert!(!ratio.pass);
    assert_eq!(
        ratio.detail,
        "usage scan grew faster than linear: T(20000 rows)=12500.0 ms, T(4000 rows)=500.0 ms, \
         ratio 25.00 > 6 (calibrated bound; floor 100 ms)"
    );
    let ceiling = check(&quadratic, "ceiling");
    assert!(!ceiling.pass);
    assert_eq!(
        ceiling.detail,
        "usage scan exceeded its ceiling: T(20000 rows)=12500.0 ms > 1000 ms (calibrated bound)"
    );
    assert!(check(&quadratic, "rows").pass);
    Ok(())
}

#[test]
fn the_floor_keeps_a_fast_small_size_from_failing_the_ratio() -> Result<()> {
    // 550 / max(10, 100) = 5.5 passes; 650 / 100 = 6.5 fails; neither is quadratic.
    let under = evaluate(&sizes(10.0, 550.0), CALIBRATED, CALIBRATED_LABEL)?;
    assert!(under.pass, "{}", render_lines(&under));
    assert!((under.ratio - 5.5).abs() < 1e-6);
    let over = evaluate(&sizes(10.0, 650.0), CALIBRATED, CALIBRATED_LABEL)?;
    assert!(!check(&over, "ratio").pass);
    assert!(check(&over, "ceiling").pass);
    // Above the floor the measured small size is the denominator.
    let measured = evaluate(&sizes(200.0, 1_000.0), CALIBRATED, CALIBRATED_LABEL)?;
    assert!((measured.ratio - 5.0).abs() < 1e-6);
    Ok(())
}

#[test]
fn a_ceiling_breach_alone_fails() -> Result<()> {
    let verdict = evaluate(&sizes(300.0, 1_500.0), CALIBRATED, CALIBRATED_LABEL)?;
    assert!(check(&verdict, "ratio").pass);
    assert!(!check(&verdict, "ceiling").pass);
    assert!(!verdict.pass);
    Ok(())
}

#[test]
fn a_row_count_mismatch_fails_in_a_warmup_or_a_sample() -> Result<()> {
    let mut measured = sizes(100.0, 500.0);
    measured[0].warmups[0].usage_rows = 3_999;
    measured[1].samples[2].usage_rows = 0;
    let verdict = evaluate(&measured, CALIBRATED, CALIBRATED_LABEL)?;
    let rows = check(&verdict, "rows");
    assert!(!rows.pass && !verdict.pass);
    assert_eq!(
        rows.detail,
        "usage scan row count differs (a forced full open must decode every owned row): \
         1000 conversations open 0 decoded 3999 rows, expected 4000; \
         5000 conversations open 3 decoded 0 rows, expected 20000"
    );
    assert!(check(&verdict, "bound-rows").pass);
    Ok(())
}

#[test]
fn a_recorded_reopen_that_decodes_rows_fails_by_name() -> Result<()> {
    let clean = evaluate(&sizes(100.0, 500.0), CALIBRATED, CALIBRATED_LABEL)?;
    let bound = check(&clean, "bound-rows");
    assert!(bound.pass, "{}", render_lines(&clean));
    assert_eq!(bound.detail, "every recorded reopen decoded 0 usage rows");
    let mut measured = sizes(100.0, 500.0);
    measured[0].bound[1].usage_rows = 4_000;
    measured[1].bound[2].usage_rows = 1;
    let verdict = evaluate(&measured, CALIBRATED, CALIBRATED_LABEL)?;
    let bound = check(&verdict, "bound-rows");
    assert!(!bound.pass && !verdict.pass);
    assert_eq!(
        bound.detail,
        "a recorded reopen decoded usage rows (the validation record did not bind it): \
         1000 conversations recorded reopen 1 decoded 4000 rows, expected 0; \
         5000 conversations recorded reopen 2 decoded 1 rows, expected 0"
    );
    // Growth and full-open rows are judged apart from the recorded series.
    assert!(check(&verdict, "ratio").pass && check(&verdict, "rows").pass);
    // The 0-rows check does not look at the recorded series' timing.
    let mut slow = sizes(100.0, 500.0);
    slow[1].bound[0] = sample(900.0, 0);
    assert!(evaluate(&slow, CALIBRATED, CALIBRATED_LABEL)?.pass);
    Ok(())
}

#[test]
fn evaluation_needs_two_ascending_sizes_with_odd_samples() {
    let mut one = sizes(1.0, 2.0);
    one.pop();
    assert!(evaluate(&one, CALIBRATED, CALIBRATED_LABEL).is_err());
    let mut reversed = sizes(1.0, 2.0);
    reversed.reverse();
    assert!(evaluate(&reversed, CALIBRATED, CALIBRATED_LABEL).is_err());
    let mut even = sizes(1.0, 2.0);
    even[0].samples.pop();
    assert!(evaluate(&even, CALIBRATED, CALIBRATED_LABEL).is_err());
    let mut even_bound = sizes(1.0, 2.0);
    even_bound[1].bound.pop();
    assert!(evaluate(&even_bound, CALIBRATED, CALIBRATED_LABEL).is_err());
    let mut no_bound = sizes(1.0, 2.0);
    no_bound[0].bound.clear();
    assert!(evaluate(&no_bound, CALIBRATED, CALIBRATED_LABEL).is_err());
}

#[test]
fn every_report_names_the_calibrated_bounds_their_derivation_rows_and_ratio() -> Result<()> {
    for verdict in [
        evaluate(&sizes(120.0, 600.0), CALIBRATED, CALIBRATED_LABEL)?,
        evaluate(&sizes(500.0, 12_500.0), CALIBRATED, CALIBRATED_LABEL)?,
    ] {
        let lines = render_lines(&verdict);
        assert!(
            lines.starts_with(
                "usage-scan check: CALIBRATED bounds (K=6, floor=100 ms, ceiling=1000 ms at the largest size); derivation: 4 Ubuntu runs"
            ),
            "{lines}"
        );
        assert!(lines.contains(DERIVATION), "{lines}");
        assert!(!lines.contains("PROVISIONAL") && !lines.contains("not yet asserted"));
        assert!(lines.contains("1000   4000   "), "{lines}");
        assert!(lines.contains("5000   20000  "), "{lines}");
        assert!(
            lines.contains("recorded reopens (no row decoded"),
            "{lines}"
        );
        assert!(lines.ends_with(&format!("verdict  {}", verdict_word(verdict.pass))));
        for name in ["rows", "ratio", "ceiling", "bound-rows"] {
            assert!(lines.contains(&format!("\n{name:<10} ")), "{lines}");
        }
        let summary = render_summary(&verdict);
        assert!(summary.contains("Bounds are **calibrated**"), "{summary}");
        assert!(summary.contains(DERIVATION), "{summary}");
        assert!(summary.contains("| bound-rows | "), "{summary}");
        assert!(summary.contains("| 5000 | 20000 |"), "{summary}");
        assert!(summary.contains("| ratio | "), "{summary}");
    }
    Ok(())
}

fn os(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn command_lines_parse_and_refuse_malformed_input() -> Result<()> {
    assert_eq!(
        parse_measure(os(&["--root", "/r", "--evidence", "/e", "--assert"]))?,
        MeasureArguments {
            root: "/r".into(),
            samples: 5,
            assert: true,
            evidence: "/e".into(),
            summary: None,
        }
    );
    assert_eq!(
        parse_measure(os(&[
            "--samples",
            "3",
            "--summary",
            "/s",
            "--evidence",
            "/e",
            "--root",
            "/r"
        ]))?,
        MeasureArguments {
            root: "/r".into(),
            samples: 3,
            assert: false,
            evidence: "/e".into(),
            summary: Some("/s".into()),
        }
    );
    assert_eq!(
        parse_fixture(os(&["create", "--conversations", "1000", "--root", "/r"]))?,
        FixtureCommand::Create {
            root: "/r".into(),
            conversations: 1_000,
        }
    );
    assert_eq!(
        parse_fixture(os(&["seal", "--root", "/r"]))?,
        FixtureCommand::Seal { root: "/r".into() }
    );
    for (input, expected) in [
        (vec!["--evidence", "/e"], "needs --root"),
        (vec!["--root", "/r"], "needs --evidence"),
        (vec!["--root", "r", "--evidence", "/e"], "absolute"),
        (
            vec!["--root", "/r", "--evidence", "/e", "--samples", "4"],
            "odd",
        ),
        (
            vec!["--root", "/r", "--evidence", "/e", "--samples", "101"],
            "at most",
        ),
        (
            vec!["--root", "/r", "--evidence", "/e", "--samples", "x"],
            "non-negative integer",
        ),
        (vec!["--root", "/r", "--root", "/s"], "twice"),
        (vec!["--assert", "--assert"], "twice"),
        (vec!["--root"], "needs a value"),
        (vec!["--bogus"], "does not accept"),
    ] {
        let error = parse_measure(os(&input)).unwrap_err();
        assert!(
            format!("{error:#}").contains(expected),
            "{input:?}: {error:#}"
        );
    }
    for (input, expected) in [
        (vec![], "needs create or seal"),
        (vec!["age"], "not age"),
        // The cache key subcommand left with the fixture cache.
        (vec!["key", "--root", "/r"], "not key"),
        (vec!["create", "--root", "/r"], "needs --conversations"),
        (vec!["seal"], "needs --root"),
        (vec!["seal", "--root", "/r", "--assert"], "does not accept"),
    ] {
        let error = parse_fixture(os(&input)).unwrap_err();
        assert!(
            format!("{error:#}").contains(expected),
            "{input:?}: {error:#}"
        );
    }
    {
        use std::os::unix::ffi::OsStringExt as _;
        let error = parse_measure(vec![OsString::from_vec(vec![0xff])]).unwrap_err();
        assert!(format!("{error:#}").contains("UTF-8"), "{error:#}");
    }
    Ok(())
}

#[test]
fn a_seal_from_another_root_build_or_plan_is_refused() -> Result<()> {
    let root = tempfile::tempdir()?;
    let layout = Layout::open(&root.path().join("fixture"))?;
    assert!(layout.root().is_absolute() && layout.root().is_dir());
    let error = read_seal(&layout, &CI).unwrap_err();
    assert!(format!("{error:#}").contains("fixture.json"), "{error:#}");
    let write =
        |sealed: &Sealed| std::fs::write(layout.seal(), serde_json::to_vec(sealed).unwrap());
    let good = || Sealed {
        format: SEAL_FORMAT.into(),
        format_version: FIXTURE_FORMAT_VERSION,
        key: compiled_key(&CI, layout.root()),
        root: layout.root().to_path_buf(),
        sizes: CI
            .sizes
            .iter()
            .map(|&n| SealedSize {
                conversations: n,
                project: layout.root().join(format!("project-{n}")),
                scope: String::new(),
                usage_rows: CI.expected_rows(n),
                age_elapsed_ms: 1,
            })
            .collect(),
    };
    write(&good())?;
    read_seal(&layout, &CI)?;
    let elsewhere = Sealed {
        root: "/elsewhere".into(),
        ..good()
    };
    write(&elsewhere)?;
    let error = read_seal(&layout, &CI).unwrap_err();
    assert!(
        format!("{error:#}").contains("sealed fixture was built for root /elsewhere"),
        "{error:#}"
    );
    write(&Sealed {
        key: "usage-scan-fixture-v1-0".into(),
        ..good()
    })?;
    let error = read_seal(&layout, &CI).unwrap_err();
    assert!(
        format!("{error:#}").contains("differs from this build's"),
        "{error:#}"
    );
    let mut fewer = good();
    fewer.sizes.pop();
    write(&fewer)?;
    let error = read_seal(&layout, &CI).unwrap_err();
    assert!(format!("{error:#}").contains("sizes differ"), "{error:#}");
    write(&Sealed {
        format_version: 0,
        ..good()
    })?;
    assert!(read_seal(&layout, &CI).is_err());
    assert!(Layout::open(Path::new("relative")).is_err());
    Ok(())
}

#[test]
fn data_directories_are_created_private_and_a_widened_one_is_refused() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir()?;
    let layout = Layout::open(&root.path().join("fixture"))?;
    let data = layout.private_data(1_000)?;
    assert_eq!(data, layout.data(1_000));
    assert_eq!(
        std::fs::metadata(&data)?.permissions().mode() & 0o777,
        0o700
    );
    // As a copied or hand-made fixture root could leave it.
    let widened = layout.data(5_000);
    std::fs::create_dir(&widened)?;
    std::fs::set_permissions(&widened, std::fs::Permissions::from_mode(0o755))?;
    let error = layout.private_data(5_000).unwrap_err();
    assert!(format!("{error:#}").contains("owner-private"), "{error:#}");
    assert_eq!(
        std::fs::metadata(&widened)?.permissions().mode() & 0o777,
        0o755
    );
    Ok(())
}

/// The spec the real-engine test ages: 2 and 6 conversations, so 8 and 24
/// owned usage rows at one turn.
const TINY: Spec = Spec {
    seed: 1,
    turns: 1,
    sizes: &[2, 6],
};

/// The row-count assertions against real Dolt: two tiny stores created with
/// an ungated owner, aged through the real write paths and sealed. An unforced
/// gated open of an aged store is bound and decodes no row, because its last
/// write recorded the validated state. The measurement then forces the full
/// validation before each cycle: its warm-up and full opens decode the plan's
/// rows and the recorded reopen after each decodes none. The owners inherit
/// the gate only through the task-local owner environment, never the
/// runner's own.
#[tokio::test]
async fn gated_opens_of_sealed_aged_stores_count_the_planned_rows() -> Result<()> {
    warm_runtime_cache().await?;
    let root = tempdir()?;
    let layout = Layout::open(&root.path().join("fixture"))?;
    let engine = Engine {
        executable: crate::store::test_supervisor()?,
        supervisor: crate::store::test_supervisor()?,
        cache_dir: Some(crate::store::test_cache()),
    };
    // Per size: a creating owner, the direct ageing open, one unforced gated
    // owner, two forcing opens and three gated owners; then a forcing open and
    // one ungated owner.
    let count = TINY.sizes.len() as u32;
    let deadline = FixtureDeadline::start(
        fixture_deadline(count, 7 * count + 2),
        "usage scan row count fixture",
    );
    let evidence = root.path().join("evidence");
    let outcome = deadline
        .run(async {
            // Only exactly `1` enables the timeline: pinned ungated whatever
            // the runner's own environment holds.
            let ungated = || vec![(OsString::from("KURU_OPEN_TIMELINE"), OsString::from("0"))];
            for &n in TINY.sizes {
                crate::service::activity::with_owner_environment(
                    ungated(),
                    create(&layout, &TINY, n, &engine),
                )
                .await?;
                let names = timeline_files(&layout.data(n), &project_scope(&layout.project(n)?))?;
                ensure!(names.is_empty(), "an ungated owner wrote {names:?}");
                let gate = crate::spawn_gate::spawning().await;
                // Claiming takes the owner lock right after the creating
                // owner's release: exclude sibling spawns across it.
                let (claim, gate) = crate::spawn_gate::excluding_spawns(gate, async {
                    super::super::aged_store::claim(&layout.data(n)).await
                })
                .await?;
                drop(gate);
                let counts = super::super::aged_store::age_claimed(
                    claim,
                    &TINY.plan(n),
                    engine.supervisor.clone(),
                    engine.cache_dir.clone(),
                    &mut |_, _| {},
                )
                .await?;
                std::fs::write(
                    layout.age_report(n),
                    super::super::aged_store::report_line(counts, 5)?,
                )?;
            }
            let error = create(&layout, &TINY, 6, &engine).await.unwrap_err();
            ensure!(
                format!("{error:#}").contains("already holds a store"),
                "{error:#}"
            );
            let error = create(&layout, &TINY, 3, &engine).await.unwrap_err();
            ensure!(format!("{error:#}").contains("not 3"), "{error:#}");
            seal(&layout, &TINY)?;
            let gate = vec![(OsString::from("KURU_OPEN_TIMELINE"), OsString::from("1"))];
            // Unforced, the aged store is bound: no row is decoded.
            for &n in TINY.sizes {
                let project = layout.project(n)?;
                let options = engine.options(layout.data(n), project_scope(&project));
                let log = evidence.join(format!("unforced-{n}.log"));
                std::fs::create_dir_all(&evidence)?;
                let (unforced, _) = crate::service::activity::with_owner_environment(
                    gate.clone(),
                    measured_open(&options, &project, &engine.executable, &log),
                )
                .await?;
                ensure!(
                    unforced.usage_rows == 0,
                    "an unforced open of the aged store with {n} conversations decoded {} rows",
                    unforced.usage_rows
                );
            }
            let (measured, opened) = crate::service::activity::with_owner_environment(
                gate,
                measure(&layout, &TINY, &engine, 1, 1, &evidence),
            )
            .await?;
            ensure!(opened.len() == 6, "{} opens", opened.len());
            for (size, rows) in measured.iter().zip([8, 24]) {
                ensure!(size.expected_rows == rows);
                ensure!(size.expected_rows == TINY.expected_rows(size.conversations));
                ensure!(size.warmups.len() == 1 && size.samples.len() == 1);
                for sample in size.warmups.iter().chain(&size.samples) {
                    ensure!(
                        sample.usage_rows == rows,
                        "{} conversations decoded {} rows on a forced full open",
                        size.conversations,
                        sample.usage_rows
                    );
                }
                ensure!(size.bound.len() == 1);
                for sample in &size.bound {
                    ensure!(
                        sample.usage_rows == 0,
                        "{} conversations decoded {} rows on a recorded reopen",
                        size.conversations,
                        sample.usage_rows
                    );
                }
            }
            let verdict = evaluate(&measured, CALIBRATED, CALIBRATED_LABEL)?;
            ensure!(check(&verdict, "rows").pass, "{}", render_lines(&verdict));
            ensure!(
                check(&verdict, "bound-rows").pass,
                "{}",
                render_lines(&verdict)
            );
            ensure!(
                verdict
                    .sizes
                    .iter()
                    .map(|size| size.conversations)
                    .eq([2, 6])
            );
            let records = std::fs::read_to_string(evidence.join("records.jsonl"))?;
            ensure!(records.lines().count() == 6, "{records}");
            let series = records
                .lines()
                .map(|line| {
                    let record: serde_json::Value = serde_json::from_str(line)?;
                    Ok(record["series"].as_str().unwrap_or_default().to_owned())
                })
                .collect::<Result<Vec<_>>>()?;
            ensure!(
                series == ["full", "full", "bound", "full", "full", "bound"],
                "{series:?}"
            );
            for open in &opened {
                ensure!(std::fs::read(&open.timeline_path)? == open.timeline);
            }
            // A seal refuses a store that now holds timelines.
            let error = seal(&layout, &TINY).unwrap_err();
            ensure!(
                format!("{error:#}").contains("holds open timelines"),
                "{error:#}"
            );
            // Without the gate an owner writes no timeline, and the
            // measurement fails by name.
            let error = crate::service::activity::with_owner_environment(
                ungated(),
                measure(&layout, &TINY, &engine, 0, 1, &evidence),
            )
            .await
            .unwrap_err();
            ensure!(
                format!("{error:#}").contains("wrote no open timeline"),
                "{error:#}"
            );
            for &n in TINY.sizes {
                let project = layout.project(n)?;
                await_managed_quiescence(&engine.options(layout.data(n), project_scope(&project)))
                    .await?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
    root.release(outcome)
}
