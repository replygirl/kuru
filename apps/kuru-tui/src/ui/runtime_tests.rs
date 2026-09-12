use std::sync::Arc;

use kuru_connectors::DemoProvider;
use kuru_core::{Config, Mode, ModelInfo};
use kuru_memory::MemoryStore;
use kuru_runtime::Harness;

use super::{DispatchOutcome, dispatch};

async fn fixture() -> (tempfile::TempDir, Harness, Vec<ModelInfo>) {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new(
        Config {
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        directory.path(),
        MemoryStore::temporary().await.unwrap(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    let models = vec![ModelInfo {
        id: "demo".into(),
        name: "Demo".into(),
        efforts: vec!["low".into(), "high".into()],
        default_effort: Some("low".into()),
    }];
    (directory, harness, models)
}

fn command_text(outcome: DispatchOutcome) -> String {
    match outcome {
        DispatchOutcome::Command(text) => text,
        DispatchOutcome::Turn(_) => panic!("expected command feedback"),
    }
}

#[tokio::test]
async fn slash_commands_change_real_runtime_state_and_validate_errors() {
    let (_dir, mut h, models) = fixture().await;
    assert!(command_text(dispatch(&mut h, &models, "/parts").await.unwrap()).contains("manager"));
    dispatch(&mut h, &models, "/mode freudian").await.unwrap();
    assert_eq!(h.config.mode, Mode::Freudian);
    dispatch(&mut h, &models, "/model demo").await.unwrap();
    assert_eq!(h.config.effort.as_deref(), Some("low"));
    dispatch(&mut h, &models, "/effort high").await.unwrap();
    assert_eq!(h.config.effort.as_deref(), Some("high"));
    assert!(
        dispatch(&mut h, &models, "/effort impossible")
            .await
            .is_err()
    );
    dispatch(&mut h, &models, "/effort default").await.unwrap();
    assert!(h.config.effort.is_none());
    let ids = h.topology.parts[..2]
        .iter()
        .map(|p| p.id.clone())
        .collect::<Vec<_>>();
    dispatch(&mut h, &models, &format!("/focus {}", ids[0]))
        .await
        .unwrap();
    assert!(h.topology.focus.is_some());
    dispatch(&mut h, &models, "/focus auto").await.unwrap();
    assert!(h.topology.focus.is_none());
    dispatch(
        &mut h,
        &models,
        &format!("/relate alliance {},{}", ids[0], ids[1]),
    )
    .await
    .unwrap();
    assert_eq!(h.topology.relationships.len(), 1);
    assert!(matches!(
        dispatch(&mut h, &models, "hello").await.unwrap(),
        DispatchOutcome::Turn(_)
    ));
    assert!(
        command_text(
            dispatch(&mut h, &models, &format!("/memory {}", ids[0]))
                .await
                .unwrap()
        )
        .contains("hello")
    );
    assert!(command_text(dispatch(&mut h, &models, "/dream").await.unwrap()).contains("summaries"));
    h.apply_dream(vec![kuru_runtime::DreamProposal::Add {
        name: "Extra".into(),
        role: h.topology.parts[0].role.clone(),
        instruction: "Complement".into(),
    }])
    .await
    .unwrap();
    dispatch(&mut h, &models, "/undo-dream").await.unwrap();
    let status: serde_json::Value = serde_json::from_str(&command_text(
        dispatch(&mut h, &models, "/memory-status").await.unwrap(),
    ))
    .unwrap();
    assert_eq!(status["engine"], "dolt");
    let revisions: serde_json::Value = serde_json::from_str(&command_text(
        dispatch(&mut h, &models, "/memory-history").await.unwrap(),
    ))
    .unwrap();
    assert!(
        revisions
            .as_array()
            .is_some_and(|revisions| !revisions.is_empty())
    );
    for bad in [
        "/unknown",
        "/relate",
        "/mode unknown",
        "/model",
        "/effort",
        "/memory missing",
    ] {
        assert!(dispatch(&mut h, &models, bad).await.is_err(), "{bad}");
    }
}
