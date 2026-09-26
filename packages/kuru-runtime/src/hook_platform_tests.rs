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
    requests: Mutex<Vec<CompletionRequest>>,
}

#[async_trait]
impl Provider for Recording {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let deliberate = request.instructions.contains("Phase: deliberate");
        self.requests.lock().unwrap().push(request);
        let completion = if deliberate && self.deliberation_call {
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
async fn pre_turn_rewrite_reaches_the_provider_without_its_durable_hook_provenance() {
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
}

#[tokio::test]
async fn pre_tool_tool_substitution_is_refused_before_dispatch() {
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
}

#[tokio::test]
async fn cancelled_pre_turn_hook_is_reaped_before_the_turn_returns() {
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
}
