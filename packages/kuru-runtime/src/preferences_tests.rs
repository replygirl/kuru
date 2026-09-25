use kuru_memory::MemoryStore;
use std::sync::Arc;

use kuru_connectors::DemoProvider;
use kuru_core::{Config, Mode, ProjectPreferences, SelectionOverrides};
use serde_json::json;

use crate::Harness;

fn config() -> Config {
    Config {
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}

fn memory_options(data: &std::path::Path, project: &std::path::Path) -> kuru_memory::OpenOptions {
    kuru_memory::test_support::open_options(data.to_owned(), crate::project_scope(project).unwrap())
        .unwrap()
}

#[tokio::test]
async fn preferences_survive_reopening_without_resuming_chats_or_crossing_project_boundaries() {
    let project = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let state = kuru_memory::test_support::tempdir().unwrap();
    let options = memory_options(state.path(), project.path());
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory,
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    let session = harness.session.id.clone();
    harness
        .run("A conversation to retain separately")
        .await
        .unwrap();
    harness.set_mode(Mode::Jungian).await.unwrap();
    harness
        .set_model("future-demo", Some("ultra".into()))
        .await
        .unwrap();
    harness.set_effort(None).await.unwrap();
    harness.shutdown(false).await.unwrap();
    drop(harness);

    let memory = MemoryStore::open(options).await.unwrap();
    let preferences = Harness::load_preferences(&memory, project.path())
        .await
        .unwrap();
    assert_eq!(preferences.mode, Some(Mode::Jungian));
    assert_eq!(preferences.providers["demo"].model, "future-demo");
    assert_eq!(preferences.providers["demo"].effort, None);
    assert_eq!(
        Harness::load_preferences(&memory, other.path())
            .await
            .unwrap(),
        ProjectPreferences::default()
    );
    let startup = Config::load_with_preferences(
        None,
        project.path(),
        None,
        &preferences,
        SelectionOverrides {
            provider: Some("demo"),
            ..SelectionOverrides::default()
        },
    )
    .unwrap();
    let fresh = Harness::new(
        startup,
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    assert_ne!(fresh.session.id, session);
    assert_eq!(fresh.config.mode, Mode::Jungian);
    assert_eq!(fresh.config.model, "future-demo");
    assert!(fresh.history().await.unwrap().is_empty());
    assert_eq!(fresh.session.turns, 0);
    drop(fresh);

    // An invocation override and resumption must not rewrite remembered choices.
    let mut temporary = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    let temporary_session = temporary.session.id.clone();
    assert_eq!(temporary.config.mode, Mode::Ifs);
    temporary.shutdown(false).await.unwrap();
    drop(temporary);
    let resumed = Harness::new(
        Config {
            mode: Mode::Freudian,
            ..config()
        },
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        Some(&temporary_session),
    )
    .await
    .unwrap();
    assert_eq!(resumed.config.mode, Mode::Ifs);
    assert_eq!(
        Harness::load_preferences(&memory, project.path())
            .await
            .unwrap(),
        preferences
    );
}

#[tokio::test]
async fn model_choices_are_provider_specific_and_mode_changes_preserve_all_pairs() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    for (provider, model, effort) in [
        ("demo", "demo", None),
        ("codex", "account-model", Some("ultra")),
        ("responses", "api-model", Some("high")),
    ] {
        let mut harness = Harness::new(
            Config {
                provider: provider.into(),
                ..config()
            },
            project.path(),
            memory.clone(),
            Arc::new(DemoProvider),
            None,
        )
        .await
        .unwrap();
        harness
            .set_model(model, effort.map(str::to_owned))
            .await
            .unwrap();
        harness.set_mode(Mode::Polyvagal).await.unwrap();
    }
    let preferences = Harness::load_preferences(&memory, project.path())
        .await
        .unwrap();
    assert_eq!(preferences.providers.len(), 3);
    assert_eq!(preferences.providers["demo"].model, "demo");
    assert_eq!(preferences.providers["demo"].effort, None);
    assert_eq!(preferences.providers["codex"].model, "account-model");
    assert_eq!(
        preferences.providers["codex"].effort.as_deref(),
        Some("ultra")
    );
    assert_eq!(preferences.providers["responses"].model, "api-model");
    assert_eq!(
        preferences.providers["responses"].effort.as_deref(),
        Some("high")
    );
    assert_eq!(preferences.mode, Some(Mode::Polyvagal));
}

#[tokio::test]
async fn failed_preference_updates_leave_live_choices_topology_and_saved_preferences_intact() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    harness.set_mode(Mode::Freudian).await.unwrap();
    harness
        .set_model("first-model", Some("high".into()))
        .await
        .unwrap();
    let before_config = harness.config.clone();
    let before_topology = serde_json::to_value(&harness.topology).unwrap();
    let before_actors = harness.actors.keys().cloned().collect::<Vec<_>>();
    let before_preferences = Harness::load_preferences(&memory, project.path())
        .await
        .unwrap();
    assert!(harness.set_model("", None).await.is_err());
    assert!(harness.set_effort(Some("".into())).await.is_err());
    for update in 0..3 {
        memory.reject_next_state_write_for_test();
        let error = match update {
            0 => harness.set_mode(Mode::Jungian).await.unwrap_err(),
            1 => harness.set_model("next-model", None).await.unwrap_err(),
            _ => harness.set_effort(None).await.unwrap_err(),
        };
        assert!(
            format!("{error:#}").contains("injected state-save refusal before request send"),
            "unexpected failed preference update: {error:#}"
        );
    }
    assert_eq!(harness.config, before_config);
    assert_eq!(harness.session.mode, before_config.mode);
    assert_eq!(
        serde_json::to_value(&harness.topology).unwrap(),
        before_topology
    );
    assert_eq!(
        harness.actors.keys().cloned().collect::<Vec<_>>(),
        before_actors
    );
    assert_eq!(
        Harness::load_preferences(&memory, project.path())
            .await
            .unwrap(),
        before_preferences
    );
}

#[tokio::test]
async fn corrupt_preferences_are_reported_instead_of_silently_reset_or_partially_applied() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    for broken in [
        json!({"mode":"wrong"}),
        json!({"providers":{"demo":{"model":"","effort":null}}}),
    ] {
        memory
            .put(&format!("{}/preferences", harness.scope), &broken)
            .await
            .unwrap();
        assert!(
            format!(
                "{:#}",
                Harness::load_preferences(&memory, project.path())
                    .await
                    .unwrap_err()
            )
            .contains("saved project preferences")
        );
        assert!(harness.set_mode(Mode::Jungian).await.is_err());
        assert!(harness.set_model("demo", None).await.is_err());
        assert_eq!(harness.config, config());
        assert_eq!(
            memory
                .get(&format!("{}/preferences", harness.scope))
                .await
                .unwrap(),
            Some(broken)
        );
    }
}
