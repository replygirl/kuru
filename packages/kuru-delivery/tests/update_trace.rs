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

#[cfg(unix)]
#[tokio::test]
async fn unix_update_owned_child_interruption_recovers_checked_checkpoint_evidence()
-> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use kuru_platform::unix::{
        GroupPresence, OwnedProcessGroup, Reap, RootState, StdioPlan, StdioSlot,
    };
    use std::{
        io::{BufRead, Read},
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    const OLD: &[u8] = b"#!/bin/sh\nprintf old-must-not-execute\n";
    const NEW: &[u8] = b"#!/bin/sh\nprintf new-must-not-execute\n";
    for checkpoint in ["prepared", "publication", "cleanup"] {
        let fixture = tempfile::tempdir()?;
        let parent = fixture.path().canonicalize()?;
        std::fs::write(parent.join("kuru"), OLD)?;
        std::fs::set_permissions(parent.join("kuru"), std::fs::Permissions::from_mode(0o555))?;
        let candidate = parent.join("read-only-build-input");
        std::fs::write(&candidate, NEW)?;
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o555))?;
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
        command
            .arg("unix-update-observed")
            .arg(&parent)
            .arg(&candidate)
            .arg(checkpoint);
        let mut owner = OwnedProcessGroup::spawn(
            command,
            StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Pipe),
        )?;
        let stdin = owner.take_stdin()?;
        let out = owner.take_stdout()?;
        let mut err = owner.take_stderr()?;
        let (ready_sender, ready) = tokio::sync::oneshot::channel();
        let (out_sender, out_result) = std::sync::mpsc::channel();
        let out_reader = std::thread::spawn(move || {
            let result = (|| -> std::io::Result<Vec<u8>> {
                let mut reader = std::io::BufReader::new(out);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                let _ = ready_sender.send(line.clone());
                let mut bytes = line.into_bytes();
                reader.take(65537).read_to_end(&mut bytes)?;
                if bytes.len() > 65536 {
                    return Err(std::io::Error::other("checkpoint stdout exceeds bound"));
                }
                Ok(bytes)
            })();
            let _ = out_sender.send(result);
        });
        let (err_sender, err_result) = std::sync::mpsc::channel();
        let err_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = err
                .by_ref()
                .take(65537)
                .read_to_end(&mut bytes)
                .and_then(|_| {
                    if bytes.len() > 65536 {
                        Err(std::io::Error::other("checkpoint stderr exceeds bound"))
                    } else {
                        Ok(bytes)
                    }
                });
            let _ = err_sender.send(result);
        });
        let body = async {
            let line = tokio::time::timeout(Duration::from_secs(30), ready)
                .await
                .context("checkpoint child never acknowledged durable boundary")??;
            ensure!(
                line == format!("Unix update checkpoint {checkpoint}\n"),
                "unexpected checkpoint acknowledgement: {line:?}"
            );
            ensure!(matches!(owner.root_state(), RootState::Running));
            // Retained owner authority, never a numeric PID; interrupt before
            // the observer's blocked stdin permits further publication.
            owner.terminate_before_reap();
            Ok::<_, anyhow::Error>(())
        }
        .await;
        owner.terminate_before_reap();
        let cleanup_deadline = Instant::now() + Duration::from_secs(10);
        owner
            .wait_pre_reap(Duration::from_millis(10), cleanup_deadline)
            .await;
        let reaped = owner.reap_if_exited();
        let mut listing = owner.permission_listing(cleanup_deadline);
        let presence = listing.resolve(owner.presence_after_reap()).await;
        drop(stdin);
        let out = out_result.recv_timeout(Duration::from_secs(10));
        let err = err_result.recv_timeout(Duration::from_secs(10));
        out_reader
            .join()
            .map_err(|_| anyhow::anyhow!("checkpoint stdout reader panicked"))?;
        err_reader
            .join()
            .map_err(|_| anyhow::anyhow!("checkpoint stderr reader panicked"))?;
        body?;
        ensure!(
            matches!(reaped, Reap::Reaped(_)),
            "checkpoint root not reaped: {reaped:?}"
        );
        ensure!(
            matches!(presence, GroupPresence::Absent | GroupPresence::Recycled),
            "checkpoint group not absent: {presence:?}"
        );
        let out = out??;
        let err = err??;
        ensure!(
            err.is_empty(),
            "checkpoint child diagnostic: {}",
            String::from_utf8_lossy(&err)
        );
        ensure!(out == format!("Unix update checkpoint {checkpoint}\n").as_bytes());
        let recovered = kuru_delivery::unix_update::recover(&parent)?
            .context("missing actual interrupted receipt")?;
        let published = checkpoint != "prepared";
        ensure!(recovered.published == published);
        ensure!(std::fs::read(parent.join("kuru"))? == if published { NEW } else { OLD });
        ensure!(std::fs::read(&candidate)? == NEW);
        let names = std::fs::read_dir(parent.join(".kuru-update"))?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        ensure!(
            names == vec![std::ffi::OsString::from("install.lock")],
            "settled rollback images remain: {names:?}"
        );
        let installation = kuru_delivery::ownership::installed(
            &parent.join("kuru"),
            &kuru_delivery::ownership::OwnershipEnv::default(),
        )?;
        ensure!(
            kuru_delivery::unix_update::transact(kuru_delivery::unix_update::Request {
                installation,
                candidate: kuru_delivery::unix_update::Candidate::BuildInput(candidate),
                version: None,
                target: kuru_delivery::archive::host_target()?.into(),
                support: None,
            })?
            .published
        );
    }
    Ok(())
}
