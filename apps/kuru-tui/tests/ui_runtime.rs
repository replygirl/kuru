use std::sync::Arc;

use kuru::ui::{View, draw, project_initial_view, project_runtime_snapshot};
use kuru_connectors::DemoProvider;
use kuru_core::{Config, Mode, RelationshipKind};
use kuru_memory::MemoryStore;
use kuru_runtime::{Event, Harness, PeerMessage};
use ratatui::{Terminal, backend::TestBackend};

async fn fixture() -> (tempfile::TempDir, Harness) {
    let project = tempfile::tempdir().unwrap();
    let harness = Harness::new(
        Config {
            provider: "demo".into(),
            model: "demo".into(),
            mode: Mode::Freudian,
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    (project, harness)
}

fn screen(view: &View) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
    terminal.draw(|frame| draw(frame, view)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[tokio::test]
async fn runtime_adapter_projects_real_relationship_completion_and_sanitized_route() {
    let (project, mut harness) = fixture().await;
    let seeded = harness.run("history seed").await.unwrap();
    let members = harness.topology.parts[..2]
        .iter()
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    let relation = harness
        .relate(RelationshipKind::Alliance, members.clone())
        .await
        .unwrap();
    let initial = project_initial_view(&harness).await.unwrap();
    assert_eq!(
        initial.transcript,
        vec![
            ("user".into(), "history seed".into()),
            (seeded.speaker, seeded.text),
        ]
    );
    assert_eq!(initial.session, harness.session.id);
    assert_eq!(
        initial.project,
        project.path().file_name().unwrap().to_string_lossy()
    );
    assert_eq!(initial.runtime.turns, harness.session.turns);
    assert_eq!(initial.runtime.mode, "freudian");
    assert_eq!(initial.runtime.model, "demo");
    assert_eq!(initial.runtime.effort, "default");
    assert_eq!(
        initial.runtime.parts,
        harness
            .topology
            .parts
            .iter()
            .filter(|part| part.active)
            .map(|part| (part.id.clone(), format!("{} · {}", part.name, part.role)))
            .collect::<Vec<_>>()
    );
    let snapshot = project_runtime_snapshot(&harness);
    assert_eq!(snapshot.turns, harness.session.turns);
    assert_eq!(snapshot.mode, harness.config.mode.to_string());
    assert_eq!(snapshot.model, harness.config.model);
    assert_eq!(snapshot.effort, "default");
    assert_eq!(snapshot.relationships, vec![relation.clone()]);
    assert_eq!(snapshot.focus.as_deref(), Some(relation.id.as_str()));

    let mut view = View::from_initial(project_initial_view(&harness).await.unwrap(), vec![]);
    view.motion = false;
    view.transcript
        .push(("user".into(), "Shared design".into()));
    let output = harness.run("Shared design").await.unwrap();
    assert_eq!(output.speaker, relation.id);
    let returned_text = output.text.clone();
    let returned_tokens = (output.input_tokens, output.output_tokens);
    for event in output.events.clone() {
        view.event(event);
    }
    view.complete_turn(output);
    view.apply_runtime(project_runtime_snapshot(&harness));

    let route = PeerMessage::new(
        &members[0],
        &members[1],
        &harness.session.id,
        "PRIVATE_PEER_TEXT_MUST_STAY_INTERNAL",
    )
    .unwrap();
    view.event(Event::Peer {
        actor: members[0].clone(),
        envelope: route.rpc(),
    });
    let rendered = screen(&view);
    for expected in [
        "Shared design",
        "alliance",
        "Relationships",
        "Peer exchange",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}:\n{rendered}"
        );
    }
    assert!(rendered.contains(&format!(
        "{} → {}",
        harness.topology.parts[0].name, harness.topology.parts[1].name
    )));
    assert!(!rendered.contains("PRIVATE_PEER_TEXT_MUST_STAY_INTERNAL"));
    assert!(rendered.contains(&format!("{} input tokens", returned_tokens.0)));
    assert!(rendered.contains(&format!("{} output tokens", returned_tokens.1)));
    assert_eq!(view.speaker_id, relation.id);
    assert_eq!(view.transcript.last().unwrap().1, returned_text);
    assert_eq!(view.turns, harness.session.turns);
    assert_eq!(view.mode, harness.config.mode.to_string());
    assert_eq!(view.model, harness.config.model);
    assert_eq!(view.effort, "default");
    assert_eq!(view.parts, project_runtime_snapshot(&harness).parts);
}
