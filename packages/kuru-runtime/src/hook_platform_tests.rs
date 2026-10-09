//! Runtime lifecycle-hook contracts exercised with the same fake hooks on Unix
//! (`/bin/sh`) and native Windows (stock PowerShell 5.1). Timed hooks keep the
//! product default timeout; Windows engine cold start is warmed outside it.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{Provider, ProviderEvent, ProviderSink};
use kuru_core::{
    Completion, CompletionRequest, Config, HookCommand, LifecycleHooks, Message, Mode, ModelInfo,
    ToolCall,
};
use kuru_memory::{MemoryStore, PublicTranscriptEntry};
use serde_json::json;

use crate::{CancellationToken, Event, Harness, turn_was_cancelled};

#[cfg(unix)]
fn fake_hook(unix: &str, _windows: &str) -> HookCommand {
    HookCommand {
        command: "/bin/sh".into(),
        args: vec!["-c".into(), unix.into()],
        ..HookCommand::default()
    }
}

#[cfg(windows)]
fn fake_hook(_unix: &str, windows: &str) -> HookCommand {
    let powershell = kuru_platform::windows::process::system_directory()
        .unwrap()
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    HookCommand {
        command: powershell.to_string_lossy().into_owned(),
        args: [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            windows,
        ]
        .map(String::from)
        .to_vec(),
        ..HookCommand::default()
    }
}

async fn warm_hook_launch() {
    kuru_connectors::shell_warmup::warm_up_stock_powershell_hook_launch()
        .await
        .unwrap();
}

async fn wait_for_file(path: &std::path::Path, bound: Duration) -> bool {
    tokio::time::timeout(bound, async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

#[derive(Default)]
struct Recording {
    deliberation_call: bool,
    denied_file_call: bool,
    read_call: bool,
    requests: Mutex<Vec<CompletionRequest>>,
}

#[async_trait]
impl Provider for Recording {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let deliberate = request.instructions.contains("Phase: deliberate");
        let current = request
            .current_message_count
            .unwrap_or(request.messages.len());
        let saw_tool_result = request
            .messages
            .iter()
            .rev()
            .take(current)
            .any(|message| message.role == "tool");
        self.requests.lock().unwrap().push(request);
        let completion = if self.read_call && !deliberate && !saw_tool_result {
            Completion::from_legacy(
                "read the proposed file",
                vec![ToolCall {
                    id: "platform-root-check".into(),
                    name: "file_read".into(),
                    arguments: json!({"path":"first.txt"}),
                }],
                1,
                1,
            )
        } else if self.denied_file_call && !deliberate && !saw_tool_result {
            Completion::from_legacy(
                "proposed write",
                vec![ToolCall {
                    id: "platform-denied-file-call".into(),
                    name: "file_write".into(),
                    arguments: json!({"path":"owned.txt","content":"rejected mutation"}),
                }],
                1,
                1,
            )
        } else if deliberate && self.deliberation_call {
            Completion::from_legacy(
                "deliberated",
                vec![ToolCall {
                    id: "platform-deliberation-call".into(),
                    name: "remember".into(),
                    arguments: json!({"text":"original note"}),
                }],
                1,
                1,
            )
        } else {
            Completion::from_legacy("final answer", vec![], 1, 1)
        };
        sink.emit(ProviderEvent::Completed(completion)).await
    }
}

fn config(hooks: LifecycleHooks) -> Config {
    Config {
        mode: Mode::Ifs,
        provider: "demo".into(),
        model: "demo".into(),
        max_rounds: 1,
        dream_every: 0,
        dream_on_exit: false,
        hooks,
        ..Config::default()
    }
}

fn hook_seen(events: &[Event], hook_event: &str, outcome: &str, call_id: Option<&str>) -> bool {
    events.iter().any(|event| {
        matches!(event, Event::Hook { observation, .. }
            if observation.event == hook_event
                && observation.outcome == outcome
                && observation.call_id.as_deref() == call_id)
    })
}

#[tokio::test]
async fn speaker_stop_preserves_the_selected_peer_before_speaking_dispatch() -> Result<()> {
    kuru_memory::test_support::closing(async {
        use anyhow::ensure;

        warm_hook_launch().await;
        let project = tempfile::tempdir()?;
        let memory = MemoryStore::temporary().await?;
        let provider = Arc::new(Recording::default());
        let hooks = LifecycleHooks {
            speaker_selected: vec![fake_hook(
                r#"cat >/dev/null; printf x > speaker-hook-ran; printf '%s' '{"decision":"stop","reason":"fixture stopped speaker"}'"#,
                r#"$null = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('speaker-hook-ran', 'x'); [Console]::Out.Write('{"decision":"stop","reason":"fixture stopped speaker"}')"#,
            )],
            ..LifecycleHooks::default()
        };
        let opening = Harness::new(config(hooks), project.path(), memory.clone(), provider.clone(), None).await;
        let mut harness = match opening {
            Ok(harness) => harness,
            Err(error) => { memory.close().await?; return Err(error); }
        };
        let target = harness.topology.parts[0].id.clone();
        let checked = async {
            let original = serde_json::to_value(&harness.topology)?;
            let error = harness.run_for("stop before speaking", Some(&target)).await
                .err().ok_or_else(|| anyhow::anyhow!("speaker stop allowed a speaking dispatch"))?;
            ensure!(error.to_string().contains("fixture stopped speaker"), "{error:#}");
            let requests = provider.requests.lock().unwrap().clone();
            ensure!(requests.len() == 1 && requests[0].instructions.contains("Phase: deliberate"));
            ensure!(std::fs::read(project.path().join("speaker-hook-ran"))? == b"x");
            ensure!(serde_json::to_value(&harness.topology)? == original);
            ensure!(harness.history().await?.iter().any(|message|
                message == &Message::text("user", "stop before speaking")));
            ensure!(harness.hook_host().in_flight_hooks() == 0);
            Ok::<(), anyhow::Error>(())
        }.await;
        let shutdown = harness.shutdown(false).await;
        let closed = memory.close().await;
        shutdown?;
        closed?;
        checked
    }).await
}

#[tokio::test]
async fn speaker_observe_keeps_the_validated_peer_and_its_speaking_dispatch() -> Result<()> {
    kuru_memory::test_support::closing(async {
        use anyhow::ensure;

        warm_hook_launch().await;
        let project = tempfile::tempdir()?;
        let memory = MemoryStore::temporary().await?;
        let provider = Arc::new(Recording::default());
        let hooks = LifecycleHooks {
            speaker_selected: vec![fake_hook(
                r#"request=$(cat); case "$request" in *'"speaker"'*) ;; *) exit 9;; esac; case "$request" in *'"reason"'*) printf '%s' "$request" > speaker-observed; printf '%s' '{"decision":"observe"}';; *) exit 9;; esac"#,
                r#"$inputJson = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('speaker-observed', $inputJson); [Console]::Out.Write('{"decision":"observe"}')"#,
            )],
            ..LifecycleHooks::default()
        };
        let opening = Harness::new(config(hooks), project.path(), memory.clone(), provider.clone(), None).await;
        let mut harness = match opening {
            Ok(harness) => harness,
            Err(error) => { memory.close().await?; return Err(error); }
        };
        let target = harness.topology.parts[0].id.clone();
        let checked = async {
            let original = serde_json::to_value(&harness.topology)?;
            let output = harness.run_for("address the selected peer", Some(&target)).await?;
            ensure!(output.text == "final answer");
            let observed: serde_json::Value = serde_json::from_slice(&std::fs::read(project.path().join("speaker-observed"))?)?;
            ensure!(observed["actor"] == target && observed["payload"]["speaker"] == target);
            ensure!(observed["payload"]["reason"].as_str().is_some_and(|reason| !reason.is_empty()));
            ensure!(serde_json::to_value(&harness.topology)? == original);
            let requests = provider.requests.lock().unwrap().clone();
            ensure!(requests.len() == 2 && requests.iter().any(|request|
                request.instructions.contains("Phase: speak") && request.actor.contains(&target)));
            ensure!(hook_seen(&output.events, "speaker_selected", "observed", None));
            ensure!(harness.hook_host().in_flight_hooks() == 0);
            Ok::<(), anyhow::Error>(())
        }.await;
        let shutdown = harness.shutdown(false).await;
        let closed = memory.close().await;
        shutdown?;
        closed?;
        checked
    }).await
}

#[tokio::test]
async fn speaker_hook_cannot_rewrite_or_substitute_the_selected_peer() -> Result<()> {
    kuru_memory::test_support::closing(async {
        use anyhow::ensure;

        warm_hook_launch().await;
        // These are the existing Unix refusal shapes. No timing failure is
        // needed to prove that hook text cannot grant a different peer.
        for decision in [
            json!({"decision":"rewrite", "value":{"actor":"another-peer"}}),
            json!({"decision":"observe", "actor":"another-peer"}),
            json!({"decision":"observe", "reason":"switch selection"}),
            json!({"decision":"stop", "reason":"switch peer", "actor":"another-peer"}),
        ] {
            let project = tempfile::tempdir()?;
            let memory = MemoryStore::temporary().await?;
            let provider = Arc::new(Recording::default());
            let reply = serde_json::to_string(&decision)?;
            let unix = format!("cat >/dev/null; printf '%s' '{}'", reply);
            let windows = format!(
                "$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{}')",
                reply
            );
            let hooks = LifecycleHooks {
                speaker_selected: vec![fake_hook(&unix, &windows)],
                ..LifecycleHooks::default()
            };
            let opening = Harness::new(
                config(hooks),
                project.path(),
                memory.clone(),
                provider.clone(),
                None,
            )
            .await;
            let mut harness = match opening {
                Ok(harness) => harness,
                Err(error) => {
                    memory.close().await?;
                    return Err(error);
                }
            };
            let target = harness.topology.parts[0].id.clone();
            let checked =
                async {
                    let original = serde_json::to_value(&harness.topology)?;
                    let error = harness
                        .run_for("address the selected peer", Some(&target))
                        .await
                        .err()
                        .ok_or_else(|| {
                            anyhow::anyhow!("invalid speaker decision dispatched: {decision}")
                        })?;
                    ensure!(error.to_string().contains("hook"), "{error:#}");
                    ensure!(serde_json::to_value(&harness.topology)? == original);
                    let requests = provider.requests.lock().unwrap().clone();
                    ensure!(
                        requests.len() == 1
                            && requests[0].instructions.contains("Phase: deliberate")
                    );
                    ensure!(
                        harness.history().await?.iter().any(|message| message
                            == &Message::text("user", "address the selected peer"))
                    );
                    ensure!(harness.hook_host().in_flight_hooks() == 0);
                    Ok::<(), anyhow::Error>(())
                }
                .await;
            let shutdown = harness.shutdown(false).await;
            let closed = memory.close().await;
            shutdown?;
            closed?;
            checked?;
        }
        Ok(())
    })
    .await
}

#[tokio::test]
async fn rewritten_file_read_is_checked_against_the_final_root_before_execution() -> Result<()> {
    kuru_memory::test_support::closing(async {
        use anyhow::{Context as _, ensure};

        warm_hook_launch().await;
        let project = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        std::fs::write(project.path().join("first.txt"), "ORIGINAL_FILE_SENTINEL")?;
        let outside_path = outside.path().join("outside.txt");
        std::fs::write(&outside_path, "OUTSIDE_FILE_SENTINEL")?;
        let reply = serde_json::to_string(&json!({
            "decision":"rewrite", "value":{"name":"file_read", "arguments":{"path":outside_path}},
        }))?;
        let unix = format!(
            "cat >/dev/null; printf '%s' '{}'",
            reply.replace('\'', "'\\''")
        );
        let windows = format!(
            "$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{}')",
            reply.replace('\'', "''")
        );
        let hooks = LifecycleHooks {
            pre_tool: vec![fake_hook(&unix, &windows)],
            ..LifecycleHooks::default()
        };
        let memory = MemoryStore::temporary().await?;
        let provider = Arc::new(Recording {
            read_call: true,
            ..Recording::default()
        });
        let opening = Harness::new(
            config(hooks),
            project.path(),
            memory.clone(),
            provider.clone(),
            None,
        )
        .await;
        let mut harness = match opening {
            Ok(harness) => harness,
            Err(error) => {
                memory.close().await?;
                return Err(error);
            }
        };
        let target = harness.topology.parts[0].id.clone();
        let checked = async {
            let output = harness.run_for("read the file", Some(&target)).await?;
            ensure!(output.text == "final answer");
            let requests = provider.requests.lock().unwrap().clone();
            let continuation = requests
                .iter()
                .find(|request| {
                    request
                        .messages
                        .iter()
                        .any(|message| message.role == "tool")
                })
                .context("rewritten read produced no tool continuation")?;
            let receipt = crate::test_receipt(
                continuation
                    .messages
                    .iter()
                    .find(|message| message.role == "tool")
                    .context("missing settled read message")?,
            )
            .context("missing settled read receipt")?;
            ensure!(receipt["call_id"] == "platform-root-check" && receipt["is_error"] == true);
            let cards = harness.tool_card_feed().snapshot().0;
            let card = cards
                .iter()
                .find(|card| card.call_id == "platform-root-check")
                .context("missing rewritten read card")?;
            ensure!(card.name == "file_read");
            let arguments = card
                .arguments
                .as_deref()
                .context("missing rewritten card arguments")?;
            ensure!(arguments.contains("outside.txt") && !arguments.contains("first.txt"));
            ensure!(
                serde_json::from_str::<serde_json::Value>(arguments)?
                    == json!({"path":outside_path})
            );
            ensure!(card.state == crate::ToolCardState::Settled(crate::ToolOutcome::Error));
            ensure!(hook_seen(
                &output.events,
                "pre_tool",
                "rewritten",
                Some("platform-root-check")
            ));
            for request in &requests {
                let exposed = format!(
                    "{} {}",
                    request.instructions,
                    serde_json::to_string(&request.messages)?
                );
                ensure!(
                    !exposed.contains("ORIGINAL_FILE_SENTINEL")
                        && !exposed.contains("OUTSIDE_FILE_SENTINEL")
                );
            }
            ensure!(std::fs::read(project.path().join("first.txt"))? == b"ORIGINAL_FILE_SENTINEL");
            ensure!(std::fs::read(&outside_path)? == b"OUTSIDE_FILE_SENTINEL");
            ensure!(harness.hook_host().in_flight_hooks() == 0);
            Ok::<(), anyhow::Error>(())
        }
        .await;
        let shutdown = harness.shutdown(false).await;
        let closed = memory.close().await;
        shutdown?;
        closed?;
        checked
    })
    .await
}

#[tokio::test]
async fn denied_pre_tool_hook_preserves_original_call_and_receipt_without_file_effects()
-> Result<()> {
    kuru_memory::test_support::closing(async {
        use anyhow::ensure;
        use kuru_core::ContentBlock;

        warm_hook_launch().await;
        let project = tempfile::tempdir()?;
        let original = b"private file body sk-abcdefghijklmnop";
        std::fs::write(project.path().join("owned.txt"), original)?;
        std::fs::write(project.path().join("adjacent.txt"), b"unrelated bytes")?;
        let provider = Arc::new(Recording { denied_file_call: true, ..Recording::default() });
        let hooks = LifecycleHooks {
            pre_tool: vec![
                fake_hook(
                    r#"cat >/dev/null; printf '%s' '{"decision":"deny","reason":"api_key=fixture-secret denied by policy"}'"#,
                    r#"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{"decision":"deny","reason":"api_key=fixture-secret denied by policy"}')"#,
                ),
                fake_hook(
                    r#"cat >/dev/null; printf x > later-hook-ran; printf '%s' '{"decision":"allow"}'"#,
                    r#"$null = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('later-hook-ran', 'x'); [Console]::Out.Write('{"decision":"allow"}')"#,
                ),
            ],
            ..LifecycleHooks::default()
        };
        let mut options = config(hooks);
        // This fixture offers the real native write; the pre-tool denial must
        // settle its original call before any otherwise permitted effect.
        options.allow_write = true;
        let mut harness = Harness::new(options, project.path(), MemoryStore::temporary().await?,
            provider.clone(), None).await?;
        let target = harness.topology.parts[0].id.clone();
        let result = harness.run_for("propose the checked file write", Some(&target)).await;
        let requests = provider.requests.lock().unwrap().clone();
        let in_flight = harness.hook_host().in_flight_hooks();
        let shutdown = harness.shutdown(false).await;
        shutdown?;
        let output = result?;

        ensure!(output.text == "final answer");
        ensure!(in_flight == 0);
        ensure!(hook_seen(&output.events, "pre_tool", "denied", Some("platform-denied-file-call")));
        ensure!(std::fs::read(project.path().join("owned.txt"))? == original);
        ensure!(std::fs::read(project.path().join("adjacent.txt"))? == b"unrelated bytes");
        ensure!(!project.path().join("later-hook-ran").try_exists()?);
        let original_arguments = json!({"path":"owned.txt","content":"rejected mutation"});
        ensure!(requests.iter().any(|request| {
            let call = request.messages.iter().flat_map(|message| &message.blocks).any(|block|
                matches!(block, ContentBlock::ToolUse { id, name, arguments }
                    if id == "platform-denied-file-call" && name == "file_write" && arguments == &original_arguments));
            let receipt = request.messages.iter().flat_map(|message| &message.blocks).any(|block|
                matches!(block, ContentBlock::ToolResult { call_id, output, is_error }
                    if call_id == "platform-denied-file-call" && *is_error
                        && output.to_string().contains("denied")
                        && output.to_string().contains("[REDACTED:recognized-secret]")));
            call && receipt
        }), "the next provider request dropped or rebound the denied original call/result");
        for request in &requests {
            let exposed = format!("{} {}", request.instructions, serde_json::to_string(&request.messages)?);
            ensure!(!exposed.contains("fixture-secret") && !exposed.contains("sk-abcdefghijklmnop")
                && !exposed.contains("private file body"), "provider request exposed private file or hook secret bytes");
        }
        Ok(())
    })
    .await
}

#[tokio::test]
async fn pre_turn_rewrite_reaches_the_provider_without_its_durable_hook_provenance() {
    kuru_memory::test_support::closing(async {
        warm_hook_launch().await;
        let project = tempfile::tempdir().unwrap();
        let provider = Arc::new(Recording::default());
        // Only the first input is rewritten, so every later projection of it must
        // come from the durable rewrite rather than a fresh hook run.
        let hooks = LifecycleHooks {
            pre_turn: vec![fake_hook(
                r#"input=$(cat); case "$input" in *"platform original"*) printf '%s' '{"decision":"rewrite","value":{"input":"platform rewrite"}}' ;; *) printf '%s' '{"decision":"allow"}' ;; esac"#,
                r#"$i = [Console]::In.ReadToEnd(); if ($i.Contains('platform original')) { [Console]::Out.Write('{"decision":"rewrite","value":{"input":"platform rewrite"}}') } else { [Console]::Out.Write('{"decision":"allow"}') }"#,
            )],
            ..LifecycleHooks::default()
        };
        let mut harness = Harness::new(
            config(hooks),
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        let other = harness.topology.parts[1].id.clone();
        let source = harness.session.id.clone();
        let output = harness
            .run_for("platform original", Some(&target))
            .await
            .unwrap();
        assert!(hook_seen(&output.events, "pre_turn", "rewritten", None));
        // The turn-scoped rewrite record carries the rewritten input, never the
        // original, under the turn's primary public node.
        let page = harness
            .memory
            .public_transcript_page(&source, None, 16)
            .await
            .unwrap();
        let [PublicTranscriptEntry::Turn { record }] = page.records.as_slice() else {
            panic!("rewritten turn is not the only public turn: {page:?}");
        };
        let rewrite = harness
            .memory
            .get(&crate::engine::pre_turn_rewrite_key(
                &harness.scope,
                &record.node_id,
            ))
            .await
            .unwrap()
            .expect("rewritten turn lacks its turn-scoped rewrite record");
        assert_eq!(rewrite["input"], "platform rewrite");
        assert!(!rewrite.to_string().contains("platform original"));
        let first_turn = provider.requests.lock().unwrap().len();
        // A later turn to an actor that never saw the rewritten turn projects the
        // public transcript; so do compaction, a resumed session and a fork.
        harness
            .run_for("platform follow-up", Some(&other))
            .await
            .unwrap();
        let follow_up = provider.requests.lock().unwrap().len();
        let compacted_from = follow_up;
        harness
            .compact_controlled(Some(&target), &CancellationToken::new())
            .await
            .unwrap();
        assert!(
            provider.requests.lock().unwrap().len() > compacted_from,
            "compaction made no provider request"
        );
        let fresh = harness.new_session().await.unwrap();
        assert_ne!(fresh, source);
        harness.resume_session(&source).await.unwrap();
        let resumed_from = provider.requests.lock().unwrap().len();
        harness
            .run_for("platform resumed", Some(&target))
            .await
            .unwrap();
        let resumed = provider.requests.lock().unwrap().len();
        let head = harness
            .memory
            .session_catalog_record(&source)
            .await
            .unwrap()
            .unwrap()
            .head_node_id
            .unwrap();
        let child = harness
            .fork_session(&source, &head, "platform fork")
            .await
            .unwrap();
        harness
            .run_for("platform forked", Some(&other))
            .await
            .unwrap();
        let requests = provider.requests.lock().unwrap().clone();
        assert!(requests[..first_turn].iter().any(|request| {
            request
                .messages
                .iter()
                .any(|message| message == &Message::text("user", "platform rewrite"))
        }));
        // Every later public-transcript projection — the non-participant's later
        // turn, the resumed session and the fork — carries the rewritten input in
        // place of the original.
        for (label, window) in [
            ("later turn", &requests[first_turn..follow_up]),
            ("resumed turn", &requests[resumed_from..resumed]),
            ("forked turn", &requests[resumed..]),
        ] {
            assert!(!window.is_empty(), "{label} made no provider request");
            assert!(
                window
                    .iter()
                    .all(|request| request.instructions.contains("platform rewrite")),
                "{label} did not project the rewritten input"
            );
        }
        // The provider sees only the rewritten request: neither the original text
        // (in messages or instructions) nor the private provenance record (nor any
        // hook record: no post hooks are configured) reaches any projection.
        assert!(requests.iter().all(|request| {
            !request.instructions.contains("platform original")
                && !request
                    .instructions
                    .contains("rewritten by a pre_turn hook")
                && !request.messages.iter().any(|message| {
                    message.role == "kuru-hook"
                        || crate::engine::is_pre_turn_rewrite_record(message)
                        || message.text_projection().contains("platform original")
                        || message
                            .text_projection()
                            .contains("rewritten by a pre_turn hook")
                })
        }));
        let private = harness.memory_for(&target).await.unwrap();
        let rewritten = private
            .iter()
            .position(|message| message == &Message::text("user", "platform rewrite"))
            .expect("the rewritten current input was not retained");
        let provenance = rewritten
            .checked_sub(1)
            .map(|previous| &private[previous])
            .expect("rewritten input lacks its preceding hook record");
        assert_eq!(provenance.role, "kuru-hook");
        assert!(crate::engine::is_pre_turn_rewrite_record(provenance));
        // The user-facing record keeps the original input in the fork's inherited
        // public transcript and in the source session.
        assert_eq!(harness.session.id, child);
        let inherited = harness
            .memory
            .public_transcript_page(&child, None, 16)
            .await
            .unwrap();
        assert!(inherited.records.iter().any(|entry| matches!(
            entry,
            PublicTranscriptEntry::Turn { record }
                if record.user_entry == Some(Message::text("user", "platform original"))
        )));
        harness.resume_session(&source).await.unwrap();
        assert_eq!(
            harness.history().await.unwrap().first(),
            Some(&Message::text("user", "platform original"))
        );
        harness.shutdown(false).await.unwrap();
    })
    .await
}

#[tokio::test]
async fn pre_tool_tool_substitution_is_refused_before_dispatch() {
    kuru_memory::test_support::closing(async {
        warm_hook_launch().await;
        let project = tempfile::tempdir().unwrap();
        let provider = Arc::new(Recording {
            deliberation_call: true,
            ..Recording::default()
        });
        let hooks = LifecycleHooks {
            pre_tool: vec![fake_hook(
                r#"cat >/dev/null; printf '%s' '{"decision":"rewrite","value":{"name":"a2a_send","arguments":{"agent":"reviewer","message":"substituted"}}}'"#,
                r#"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{"decision":"rewrite","value":{"name":"a2a_send","arguments":{"agent":"reviewer","message":"substituted"}}}')"#,
            )],
            ..LifecycleHooks::default()
        };
        let mut harness = Harness::new(
            config(hooks),
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider,
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        let output = harness.run_for("deliberate", Some(&target)).await.unwrap();
        assert!(hook_seen(
            &output.events,
            "pre_tool",
            "failed",
            Some("platform-deliberation-call")
        ));
        assert!(!output.events.iter().any(|event| matches!(
            event,
            Event::ToolStarted { name, .. } | Event::ToolSettled {
                observation: crate::ToolObservation { name, .. },
                ..
            } if name == "a2a_send"
        )));
        harness.shutdown(false).await.unwrap();
    })
    .await
}

#[tokio::test]
async fn cancelled_pre_turn_hook_is_reaped_before_the_turn_returns() {
    kuru_memory::test_support::closing(async {
        warm_hook_launch().await;
        let project = tempfile::tempdir().unwrap();
        let started = project.path().join("hook-started");
        let provider = Arc::new(Recording::default());
        let hook = fake_hook(
            "cat >/dev/null; : > hook-started; sleep 30",
            "$null = [Console]::In.ReadToEnd(); [System.IO.File]::WriteAllText([System.IO.Path]::Combine([Environment]::CurrentDirectory, 'hook-started'), 'x'); [Threading.Thread]::Sleep(30000)",
        );
        // The hook marks its start before its own timeout or not at all.
        let budget_wait = Duration::from_millis(hook.timeout_ms);
        let hooks = LifecycleHooks {
            pre_turn: vec![hook],
            ..LifecycleHooks::default()
        };
        let mut harness = Harness::new(
            config(hooks),
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        let cancellation = CancellationToken::new();
        let (result, observed) = tokio::join!(
            harness.run_local_controlled(
                "cancel during the pre-turn hook",
                Some(&target),
                "platform-cancel",
                &cancellation
            ),
            async {
                let observed = wait_for_file(&started, budget_wait).await;
                cancellation.cancel();
                observed
            }
        );
        assert!(observed, "pre-turn hook did not start before its timeout");
        if let Err(error) = &result {
            assert!(turn_was_cancelled(error), "{error:#}");
        }
        // The cancelled turn returned only after its owned hook tree was reaped.
        assert_eq!(harness.hook_host().in_flight_hooks(), 0);
        assert!(provider.requests.lock().unwrap().is_empty());
        harness.shutdown(false).await.unwrap();
    })
    .await
}

#[tokio::test]
async fn lost_post_hook_reply_keeps_private_annotations_and_exact_retry_without_replaying_effects()
-> Result<()> {
    kuru_memory::test_support::closing(async {
        use anyhow::{Context as _, ensure};

        warm_hook_launch().await;
        let project = tempfile::tempdir()?;
        let project_path = project.path().canonicalize()?;
        let data = kuru_memory::test_support::tempdir()?;
        let options = kuru_memory::test_support::warmed_open_options(
            data.path().to_owned(), crate::project_scope(&project_path)?,
        ).await?;
        let executable = options.supervisor.clone().context("managed hook fixture supervisor missing")?;
        let memory = MemoryStore::open_managed_observed(
            options.clone(), project_path.clone(), executable.clone(),
        ).1.await?;
        let sibling = MemoryStore::open_managed_observed(
            options.clone(), project_path.clone(), executable,
        ).1.await?;
        let provider = Arc::new(Recording { deliberation_call: true, ..Recording::default() });
        let hooks = LifecycleHooks {
            post_tool: vec![fake_hook(
                r#"cat >/dev/null; printf x >> post-hook-ran; printf '%s' '{"decision":"annotate","annotation":"PRIVATE-POST-TOOL"}'"#,
                r#"$null = [Console]::In.ReadToEnd(); [System.IO.File]::AppendAllText([System.IO.Path]::Combine([Environment]::CurrentDirectory, 'post-hook-ran'), 'x'); [Console]::Out.Write('{"decision":"annotate","annotation":"PRIVATE-POST-TOOL"}')"#,
            )],
            post_turn: vec![
                fake_hook(r#"cat >/dev/null; printf '{'"#,
                    r#"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{')"#),
                fake_hook(r#"cat >/dev/null; printf '%s' '{"decision":"annotate","annotation":"PRIVATE-POST-TURN"}'"#,
                    r#"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{"decision":"annotate","annotation":"PRIVATE-POST-TURN"}')"#),
            ],
            ..LifecycleHooks::default()
        };
        let mut harness = Harness::new(config(hooks), &project_path, memory.clone(), provider.clone(), None).await?;
        let target = harness.topology.parts[0].id.clone();
        let other = harness.topology.parts[1].id.clone();
        let namespace = harness.namespace(&target);
        let session = harness.session.id.clone();
        let barrier = kuru_memory::test_support::ReplyBarrier::default();
        let (lose_reply, cancelled_reply) = tokio::sync::oneshot::channel();
        *harness.annotation_reply_pause.lock().unwrap() = Some((barrier.clone(), cancelled_reply));
        let cancellation = CancellationToken::new();
        let mut events = harness.subscribe();
        let annotation = |message: &Message| -> Option<serde_json::Value> {
            if message.role != "kuru-hook" { return None; }
            serde_json::from_str(message.plain_text()?).ok()
        };
        let checked = async {
            let (first, observed_commit) = tokio::join!(
                harness.run_local_controlled("remember once", Some(&target), "platform-lost-hook-reply", &cancellation),
                async {
                    let observed = tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                        barrier.wait_sent().await;
                        loop {
                            let history = sibling.session_history_window(&namespace, &session, 32).await?;
                            if history.messages.iter().any(|message| annotation(message).is_some_and(|record| {
                                record["call_id"] == "platform-deliberation-call"
                                    && record["session_id"] == session
                                    && record["actor"] == target
                                    && record["annotation"] == "PRIVATE-POST-TOOL"
                            })) { break Ok::<(), anyhow::Error>(()); }
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    }).await.context("managed owner did not publish the exact private hook annotation")
                        .and_then(std::convert::identity);
                    // Drop the held reply only after its independent commit proof;
                    // any failed observation cancels the joined turn before cleanup.
                    let _ = lose_reply.send(());
                    if observed.is_err() { cancellation.cancel(); }
                    observed
                }
            );
            observed_commit?;
            let first = first?;
            ensure!(!first.reused && first.output.text == "final answer");
            ensure!(hook_seen(&first.output.events, "post_tool", "annotated", Some("platform-deliberation-call")));
            ensure!(!first.output.events.iter().any(|event| matches!(event, Event::Hook { observation, .. }
                if observation.event == "post_turn")), "post-turn observations changed the immutable answer output");
            let live = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
            ensure!(hook_seen(&live, "post_turn", "failed", None)
                && hook_seen(&live, "post_turn", "annotated", None), "post-turn failure suppressed the following annotation");
            let notes = harness.notes_for(&target, 16).await?;
            ensure!(notes.notes.iter().filter(|note| note.content == "original note").count() == 1,
                "lost annotation reply replayed its settled remember effect");
            let private = harness.memory_for(&target).await?;
            let records = private.iter().filter_map(annotation).collect::<Vec<_>>();
            ensure!(records.len() == 2 && records.iter().all(|record| record["actor"] == target && record["session_id"] == session));
            ensure!(records.iter().any(|record| record["annotation"] == "PRIVATE-POST-TURN"));
            let public = harness.history().await?;
            ensure!(public == [Message::text("user", "remember once"), Message::text("assistant", "final answer")]);
            let requests = provider.requests.lock().unwrap().clone();
            ensure!(requests.iter().any(|request| request.messages.iter().any(|message|
                annotation(message).is_some_and(|record| record["annotation"] == "PRIVATE-POST-TOOL"))),
                "accepted annotation never reached its actor's actual provider request");
            ensure!(requests.iter().any(|request| request.messages.iter().any(|message|
                message.role == "tool" && message.text_projection().contains("stored in your private durable notes"))),
                "post-hook changed or removed the original settled tool receipt");
            let calls = requests.len();
            let effect = std::fs::read(project.path().join("post-hook-ran"))?;
            ensure!(effect == b"x", "post-tool hook ran more than once for its settled call");
            let replay = harness.run_local_controlled("remember once", Some(&target), "platform-lost-hook-reply", &CancellationToken::new()).await?;
            ensure!(replay.reused && replay.output.text == first.output.text);
            ensure!(provider.requests.lock().unwrap().len() == calls);
            ensure!(std::fs::read(project.path().join("post-hook-ran"))? == effect);
            ensure!(harness.memory_for(&target).await?.iter().filter_map(annotation).count() == 2);
            ensure!(!std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(event,
                Event::Hook { observation, .. } if observation.event == "post_turn")));
            harness.run_for("another actor's own context", Some(&other)).await?;
            let requests = provider.requests.lock().unwrap();
            ensure!(requests.len() > calls && requests[calls..].iter().all(|request|
                !request.messages.iter().any(|message| annotation(message).is_some_and(|record| record["actor"] == target))),
                "private hook annotation escaped into another actor's provider request");
            Ok::<(), anyhow::Error>(())
        }.await;
        let shutdown = harness.shutdown(false).await;
        drop(harness);
        let memory_closed = memory.close().await;
        let sibling_closed = sibling.close().await;
        let quiet = kuru_memory::test_support::await_managed_quiescence(&options).await;
        checked?;
        shutdown?;
        memory_closed?;
        sibling_closed?;
        quiet
    }).await
}
