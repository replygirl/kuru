//! Native composer recall preserves literal prompts and the unsent draft.

use super::*;

const LITERAL_DRAFT: &str = "literal draft e\u{301} 猫";
const NATIVE_DRAFT_PROJECTION: &str = "literal draft e 猫";

fn draft_frame(terminal: &mut Terminal, cursor_after: &str, labels: &[&str]) -> Result<()> {
    terminal.text(labels, READY)?;
    // The text can precede the final cursor update. Search and paste states
    // remain stable until another key, so bind the cursor before sending it.
    if cursor_after == LITERAL_DRAFT {
        terminal.composer_projection(cursor_after, NATIVE_DRAFT_PROJECTION)
    } else {
        terminal.composer(cursor_after)
    }
}

fn settled_turn(terminal: &mut Terminal, count: usize) -> Result<()> {
    // The count comes from the refreshed durable runtime projection. A final
    // answer can appear in provisional preview before the turn is settled.
    terminal.frame_text(&[&format!("{count} turns"), "enter send"], READY)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_recall_search_and_key_history_preserve_literal_prompts() -> Result<()> {
    let _serial = SERIAL.lock().await;
    for columns in [120, 80] {
        let sandbox = Sandbox::warmed().await?;
        let outcome = async {
            let mut terminal = sandbox.start(
                "recall",
                "app",
                &["--mode", "freudian", "-c", "max_rounds=1"],
                true,
                "demo",
                &[],
            )?;
            terminal.resize(30, columns)?;
            terminal.frame_text(&["enter send"], sandbox.startup)?;
            let oldest = "recall oldest 猫";
            let newest = "recall newest 日本語";
            for (index, prompt) in [oldest, newest].into_iter().enumerate() {
                terminal.send(prompt.as_bytes())?;
                terminal.composer(prompt)?;
                terminal.send(b"\r")?;
                settled_turn(&mut terminal, index + 1)?;
            }

            let draft = LITERAL_DRAFT;
            terminal.send(draft.as_bytes())?;
            draft_frame(&mut terminal, draft, &["enter send"])?;
            for (key, prompt, label) in [
                (b"\x1b[A".as_slice(), newest, "history 1/2"),
                (b"\x1b[B".as_slice(), draft, "enter send"),
                (b"\x1b[A".as_slice(), newest, "history 1/2"),
                (b"\x1b[A".as_slice(), oldest, "history 2/2"),
                (b"\x1b[B".as_slice(), newest, "history 1/2"),
                (b"\x1b".as_slice(), draft, "enter send"),
            ] {
                terminal.send(key)?;
                draft_frame(&mut terminal, prompt, &[label])?;
            }

            terminal.send(b"\x12recall")?;
            draft_frame(&mut terminal, newest, &["reverse search", "1/2"])?;
            terminal.send(b"\x12")?;
            draft_frame(&mut terminal, oldest, &["reverse search", "2/2"])?;
            terminal.send(b"\x12")?;
            draft_frame(&mut terminal, newest, &["reverse search", "1/2"])?;
            terminal.send(&[127; 6])?;
            terminal.send(b"NO_NATIVE_RECALL_MATCH")?;
            draft_frame(&mut terminal, draft, &["reverse search", "0/0"])?;
            terminal.send(b"\x1b")?;
            draft_frame(&mut terminal, draft, &["enter send"])?;
            // Submit the restored decomposed draft so the durable byte check
            // proves the accent survived independently of ConPTY's projection.
            terminal.send(b"\r")?;
            settled_turn(&mut terminal, 3)?;
            terminal.send(b"\x12oldest")?;
            draft_frame(&mut terminal, oldest, &["reverse search", "1/1"])?;
            // Enter accepts a recalled draft; the next Enter submits it.
            terminal.send(b"\r")?;
            draft_frame(&mut terminal, oldest, &["enter send"])?;
            terminal.send(b"\r")?;
            settled_turn(&mut terminal, 4)?;

            let prefix = "saved 猫pre";
            let insertion = "remove_me";
            let edited_draft = format!("{prefix}{insertion}post");
            // crossterm's native Windows backend reads console key records,
            // not atomic bracketed-paste events. Exercise actual key transport;
            // paste-chip controls remain covered by Unix PTYs and event tests.
            terminal.send(edited_draft.as_bytes())?;
            draft_frame(&mut terminal, &edited_draft, &["enter send"])?;
            terminal.send(b"\x1b[A")?;
            draft_frame(&mut terminal, oldest, &["history 1/4"])?;
            terminal.send(b"\x1b[B")?;
            draft_frame(&mut terminal, &edited_draft, &["enter send"])?;
            terminal.send(b"\x12newest")?;
            draft_frame(&mut terminal, newest, &["reverse search", "1/1"])?;
            terminal.send(b"\x1b")?;
            draft_frame(&mut terminal, &edited_draft, &["enter send"])?;

            terminal.send(b"\x1b[D\x1b[D\x1b[D\x1b[D")?;
            draft_frame(
                &mut terminal,
                &format!("{prefix}{insertion}"),
                &[&edited_draft],
            )?;
            terminal.send(&vec![127; insertion.len()])?;
            let removed = format!("{prefix}post");
            draft_frame(&mut terminal, prefix, &[&removed])?;
            terminal.send(b"\x05")?;
            terminal.composer(&removed)?;
            terminal.send(b"\r")?;
            settled_turn(&mut terminal, 5)?;

            let long_text = "日本語猫".repeat(50);
            let literal = format!("literal-input:{long_text}:end");
            terminal.send(literal.as_bytes())?;
            draft_frame(&mut terminal, ":end", &["enter send"])?;
            terminal.send(b"\r")?;
            settled_turn(&mut terminal, 6)?;
            // Submitted history carries canonical Unicode text. Its wrapped
            // final row owns the cursor, independently of logical-line joining.
            terminal.send(b"\x1b[A")?;
            draft_frame(&mut terminal, ":end", &["history 1/6"])?;
            terminal.send(b":again")?;
            terminal.composer(":end:again")?;
            terminal.send(b"\r")?;
            settled_turn(&mut terminal, 7)?;
            terminal.send(b"/quit\r")?;
            ensure!(terminal.finish(EXIT)?["status"] == 0);
            drop(terminal);

            let sessions: Vec<kuru_runtime::Session> =
                serde_json::from_str(&sandbox.output("sessions")?)?;
            ensure!(sessions.len() == 1 && sessions[0].turns == 7);
            let output = BlockingCommand::new(env!("CARGO_BIN_EXE_kuru"))
                .fixture_allow_independent_service()
                .args(sandbox.args("demo"))
                .args(["sessions", "export", &sessions[0].id, "--format", "jsonl"])
                .env_clear()
                .envs(&sandbox.environment)
                .output()?;
            ensure!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let mut prompts = Vec::new();
            for line in std::str::from_utf8(&output.stdout)?.lines().skip(1) {
                let entry: kuru_memory::PublicTranscriptEntry = serde_json::from_str(line)?;
                let kuru_memory::PublicTranscriptEntry::Turn { record } = entry else {
                    anyhow::bail!("native recall created an unexpected legacy record");
                };
                ensure!(record.settlement == kuru_memory::PublicTurnSettlement::Completed);
                let user = record.user_entry.context("settled native prompt absent")?;
                prompts.push(
                    user.plain_text()
                        .context("native prompt is not text")?
                        .to_owned(),
                );
            }
            let expected = [
                oldest.to_owned(),
                newest.to_owned(),
                draft.to_owned(),
                oldest.to_owned(),
                removed,
                literal.clone(),
                format!("{literal}:again"),
            ];
            ensure!(
                prompts.len() == expected.len(),
                "native recall prompt count: {}",
                prompts.len()
            );
            for (index, (actual, expected)) in prompts.iter().zip(&expected).enumerate() {
                ensure!(
                    actual == expected,
                    "native recall changed canonical prompt bytes at {columns} columns, turn {}: \
                     expected={expected:?} ({} bytes, scalars {:?}); \
                     actual={actual:?} ({} bytes, scalars {:?})",
                    index + 1,
                    expected.len(),
                    expected.chars().map(u32::from).collect::<Vec<_>>(),
                    actual.len(),
                    actual.chars().map(u32::from).collect::<Vec<_>>()
                );
            }
            Ok(())
        }
        .await;
        sandbox.release(outcome)?;
    }
    Ok(())
}
