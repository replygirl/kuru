//! Native composer recall preserves literal prompts and the unsent draft.

use super::*;
use crossterm::event::{KeyCode as Key, KeyModifiers as Modifiers};

const LITERAL_DRAFT: &str = "literal draft e\u{301} 猫";
const NATIVE_DRAFT_PROJECTION: &str = "literal draft e 猫";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_literal_combining_input_is_observed_before_composer() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let temporary = tempfile::tempdir()?;
    let directory = temporary.path().join("input-events");
    let mut terminal = Terminal::spawn(
        &directory,
        json!({
            "mode":"input-events", "binary":env!("CARGO_BIN_EXE_kuru"),
            "args":[], "environment":{}, "cwd":temporary.path(),
        }),
        30,
        120,
    )?;
    let outcome = async {
        terminal.wait("native raw input initialization", READY, |_| {
            directory.join("ready.json").is_file()
        })?;
        terminal.committed_text(LITERAL_DRAFT)?;
        terminal.committed_key(Key::Enter, Modifiers::NONE)?;
        terminal.finish(EXIT)
    }
    .await;
    // Finish joins the real console/output cleanup. Even an error drops the
    // retained native fixture before any event-byte comparison is evaluated.
    drop(terminal);
    let removed = temporary.close();
    let report = outcome?;
    removed.context("settle native input fixture directory")?;
    ensure!(
        report["status"] == 0,
        "native input fixture failed: {report}"
    );
    let observed = &report["input_events"];
    let raw_characters = observed["all_characters"]
        .as_str()
        .context("raw character receipt absent")?;
    let filtered = observed["filtered_text"]
        .as_str()
        .context("filtered character receipt absent")?;
    let expected_scalars: Vec<u32> = LITERAL_DRAFT.chars().map(u32::from).collect();
    let raw_scalars: Vec<u32> = raw_characters.chars().map(u32::from).collect();
    let filtered_scalars: Vec<u32> = filtered.chars().map(u32::from).collect();
    ensure!(
        filtered == LITERAL_DRAFT,
        "native pre-composer input differs: expected={LITERAL_DRAFT:?}/{} bytes/scalars={expected_scalars:?}; raw={raw_characters:?}/{} bytes/scalars={raw_scalars:?}; filtered={filtered:?}/{} bytes/scalars={filtered_scalars:?}; keys={}",
        LITERAL_DRAFT.len(),
        raw_characters.len(),
        filtered.len(),
        observed["keys"]
    );
    let keys = observed["keys"]
        .as_array()
        .context("native key receipts absent")?;
    let accent_kinds: Vec<&str> = keys
        .iter()
        .filter(|key| key["scalar"] == u32::from('\u{301}'))
        .filter_map(|key| key["kind"].as_str())
        .collect();
    ensure!(
        accent_kinds == ["Press", "Release"],
        "committed accent must retain exactly one down/up pair: {observed}"
    );
    let expected_raw: String = LITERAL_DRAFT.chars().flat_map(|ch| [ch, ch]).collect();
    ensure!(
        raw_characters == expected_raw,
        "ordinary committed releases must remain visible and excluded from text: {observed}"
    );
    Ok(())
}

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
                terminal.committed_text(prompt)?;
                terminal.composer(prompt)?;
                terminal.committed_key(Key::Enter, Modifiers::NONE)?;
                settled_turn(&mut terminal, index + 1)?;
            }

            let draft = LITERAL_DRAFT;
            terminal.committed_text(draft)?;
            draft_frame(&mut terminal, draft, &["enter send"])?;
            for (key, prompt, label) in [
                (Key::Up, newest, "history 1/2"),
                (Key::Down, draft, "enter send"),
                (Key::Up, newest, "history 1/2"),
                (Key::Up, oldest, "history 2/2"),
                (Key::Down, newest, "history 1/2"),
                (Key::Esc, draft, "enter send"),
            ] {
                terminal.committed_key(key, Modifiers::NONE)?;
                draft_frame(&mut terminal, prompt, &[label])?;
            }

            terminal.committed_key(Key::Char('r'), Modifiers::CONTROL)?;
            terminal.committed_text("recall")?;
            draft_frame(&mut terminal, newest, &["reverse search", "1/2"])?;
            terminal.committed_key(Key::Char('r'), Modifiers::CONTROL)?;
            draft_frame(&mut terminal, oldest, &["reverse search", "2/2"])?;
            terminal.committed_key(Key::Char('r'), Modifiers::CONTROL)?;
            draft_frame(&mut terminal, newest, &["reverse search", "1/2"])?;
            for _ in 0..6 {
                terminal.committed_key(Key::Backspace, Modifiers::NONE)?;
            }
            terminal.committed_text("NO_NATIVE_RECALL_MATCH")?;
            draft_frame(&mut terminal, draft, &["reverse search", "0/0"])?;
            terminal.committed_key(Key::Esc, Modifiers::NONE)?;
            draft_frame(&mut terminal, draft, &["enter send"])?;
            // Submit the restored decomposed draft so the durable byte check
            // proves the accent survived independently of ConPTY's projection.
            terminal.committed_key(Key::Enter, Modifiers::NONE)?;
            settled_turn(&mut terminal, 3)?;
            terminal.committed_key(Key::Char('r'), Modifiers::CONTROL)?;
            terminal.committed_text("oldest")?;
            draft_frame(&mut terminal, oldest, &["reverse search", "1/1"])?;
            // Enter accepts a recalled draft; the next Enter submits it.
            terminal.committed_key(Key::Enter, Modifiers::NONE)?;
            draft_frame(&mut terminal, oldest, &["enter send"])?;
            terminal.committed_key(Key::Enter, Modifiers::NONE)?;
            settled_turn(&mut terminal, 4)?;

            let prefix = "saved 猫pre";
            let insertion = "remove_me";
            let edited_draft = format!("{prefix}{insertion}post");
            // crossterm's native Windows backend reads console key records,
            // not atomic bracketed-paste events. Exercise actual key transport;
            // paste-chip controls remain covered by Unix PTYs and event tests.
            terminal.committed_text(&edited_draft)?;
            draft_frame(&mut terminal, &edited_draft, &["enter send"])?;
            terminal.committed_key(Key::Up, Modifiers::NONE)?;
            draft_frame(&mut terminal, oldest, &["history 1/4"])?;
            terminal.committed_key(Key::Down, Modifiers::NONE)?;
            draft_frame(&mut terminal, &edited_draft, &["enter send"])?;
            terminal.committed_key(Key::Char('r'), Modifiers::CONTROL)?;
            terminal.committed_text("newest")?;
            draft_frame(&mut terminal, newest, &["reverse search", "1/1"])?;
            terminal.committed_key(Key::Esc, Modifiers::NONE)?;
            draft_frame(&mut terminal, &edited_draft, &["enter send"])?;

            for _ in 0..4 {
                terminal.committed_key(Key::Left, Modifiers::NONE)?;
            }
            draft_frame(
                &mut terminal,
                &format!("{prefix}{insertion}"),
                &[&edited_draft],
            )?;
            for _ in 0..insertion.len() {
                terminal.committed_key(Key::Backspace, Modifiers::NONE)?;
            }
            let removed = format!("{prefix}post");
            draft_frame(&mut terminal, prefix, &[&removed])?;
            terminal.committed_key(Key::Char('e'), Modifiers::CONTROL)?;
            terminal.composer(&removed)?;
            terminal.committed_key(Key::Enter, Modifiers::NONE)?;
            settled_turn(&mut terminal, 5)?;

            let long_text = "日本語猫".repeat(50);
            let literal = format!("literal-input:{long_text}:end");
            terminal.committed_text(&literal)?;
            draft_frame(&mut terminal, ":end", &["enter send"])?;
            terminal.committed_key(Key::Enter, Modifiers::NONE)?;
            settled_turn(&mut terminal, 6)?;
            // Submitted history carries canonical Unicode text. Its wrapped
            // final row owns the cursor, independently of logical-line joining.
            terminal.committed_key(Key::Up, Modifiers::NONE)?;
            draft_frame(&mut terminal, ":end", &["history 1/6"])?;
            terminal.committed_text(":again")?;
            terminal.composer(":end:again")?;
            terminal.committed_key(Key::Enter, Modifiers::NONE)?;
            settled_turn(&mut terminal, 7)?;
            terminal.committed_text("/quit")?;
            terminal.committed_key(Key::Enter, Modifiers::NONE)?;
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
