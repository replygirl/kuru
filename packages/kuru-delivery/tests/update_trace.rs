#[path = "support/update_trace.rs"]
mod update_trace;

const START: &str = "trusted update helper: phase=started elapsed_ms=0\n";

#[test]
fn helper_trace_accepts_complete_interruption_prefixes_and_finished_paths() {
    let partial = format!("{START}trusted update helper: phase=copy_candidate elapsed_ms=12\n");
    let records = update_trace::parse(partial.as_bytes()).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1],
        update_trace::Record {
            phase: "copy_candidate",
            elapsed_ms: 12
        }
    );
    for final_phase in ["complete", "parent_still_alive_cleanup_pending"] {
        let finished =
            format!("{partial}trusted update helper: phase={final_phase} elapsed_ms=12\n");
        assert_eq!(
            update_trace::parse(finished.as_bytes())
                .unwrap()
                .last()
                .unwrap()
                .phase,
            final_phase
        );
        assert!(update_trace::parse(finished.replace('\n', "\r\n").as_bytes()).is_ok());
    }
}

#[test]
fn helper_trace_rejects_actual_errors_unknown_phases_and_extra_text() {
    for invalid in [
        "Error: publication failed; recovery receipt retained\n",
        "trusted update helper: phase=unexpected elapsed_ms=1\n",
        "trusted update helper: phase=complete elapsed_ms=1 error=failed\n",
        "trusted update helper: phase=complete elapsed_ms=1\nError: broken pipe\n",
        "prefix trusted update helper: phase=complete elapsed_ms=1\n",
        "\n",
    ] {
        assert!(
            update_trace::parse(format!("{START}{invalid}").as_bytes()).is_err(),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn helper_trace_rejects_incomplete_invalid_utf8_or_noncanonical_timing_records() {
    for invalid in [
        "",
        "+1",
        "-1",
        "1.0",
        " 1",
        "01",
        "1 ",
        "340282366920938463463374607431768211456",
    ] {
        let trace = format!("{START}trusted update helper: phase=complete elapsed_ms={invalid}\n");
        assert!(
            update_trace::parse(trace.as_bytes()).is_err(),
            "accepted {invalid:?}"
        );
    }
    for invalid in [
        "",
        "trusted update helper: phase=started elapsed_ms=0",
        "trusted update helper: phase=complete elapsed_ms=1\n",
        "trusted update helper: phase=started elapsed_ms=2\ntrusted update helper: phase=complete elapsed_ms=1\n",
        "trusted update helper: phase=started elapsed_ms=0\ntrusted update helper: phase=started elapsed_ms=1\n",
    ] {
        assert!(
            update_trace::parse(invalid.as_bytes()).is_err(),
            "accepted {invalid:?}"
        );
    }
    assert!(update_trace::parse(b"\xff\n").is_err());
    assert!(update_trace::parse(&vec![b'\n'; 64 * 1024 + 1]).is_err());
}
