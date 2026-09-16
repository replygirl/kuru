use anyhow::Result;
use kuru_core::{Framework, Mode, Relationship, RelationshipKind};
use kuru_memory::{MemoryStore, StoredNote};
use serde_json::to_value;
use tempfile::TempDir;

use crate::{NotesView, Topology, forget_note, project_scope, read_notes};

async fn seeded(mode: Mode) -> (TempDir, MemoryStore, String, Topology) {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let scope = project_scope(project.path()).unwrap();
    let topology = Topology {
        parts: Framework::builtin(mode).parts,
        relationships: vec![],
        states: Default::default(),
        focus: None,
    };
    memory
        .put(
            &format!("{scope}/{mode}/topology"),
            &to_value(&topology).unwrap(),
        )
        .await
        .unwrap();
    (project, memory, scope, topology)
}

fn notes_key(scope: &str, mode: Mode, identity: &str) -> String {
    format!("{scope}/{mode}/identity/{identity}/notes")
}

#[tokio::test]
async fn notes_view_is_provider_free_and_separate_from_conversation() {
    let (project, memory, scope, topology) = seeded(Mode::Ifs).await;
    let identity = topology.parts[0].id.clone();
    memory
        .append(
            &format!("{scope}/ifs/identity/{identity}"),
            "user",
            "CONVERSATION-MARKER",
        )
        .await
        .unwrap();
    memory
        .append(
            &notes_key(&scope, Mode::Ifs, &identity),
            "note",
            "NOTE-MARKER",
        )
        .await
        .unwrap();

    let view = read_notes(&memory, project.path(), Mode::Ifs, &identity, 100)
        .await
        .unwrap();
    assert_eq!(
        view,
        NotesView {
            mode: Mode::Ifs,
            identity,
            notes: vec![StoredNote {
                sequence: view.notes[0].sequence,
                role: "note".into(),
                content: "NOTE-MARKER".into(),
            }],
            requested_limit: 100,
            truncated: false,
        }
    );
}

#[tokio::test]
async fn forgetting_exact_current_note_keeps_dream_rows_and_other_namespaces() -> Result<()> {
    let (project, memory, scope, topology) = seeded(Mode::Ifs).await;
    let identity = topology.parts[0].id.clone();
    let notes = notes_key(&scope, Mode::Ifs, &identity);
    let transcript = format!("{scope}/ifs/identity/{identity}");
    memory.append(&notes, "note", "KEEP-NOTE").await?;
    memory.append(&notes, "dream", "REMOVE-DREAM").await?;
    memory
        .append(&transcript, "user", "CONVERSATION-REMAINS")
        .await?;
    let before = read_notes(&memory, project.path(), Mode::Ifs, &identity, 100).await?;
    let dream = before
        .notes
        .iter()
        .find(|row| row.role == "dream")
        .expect("dream-authored notes are visible");
    let deleted = forget_note(
        &memory,
        project.path(),
        Mode::Ifs,
        &identity,
        dream.sequence,
    )
    .await?;
    assert_eq!(deleted.identity, identity);
    assert_eq!(deleted.sequence, dream.sequence);
    assert!(deleted.history_retained);
    let after = read_notes(&memory, project.path(), Mode::Ifs, &identity, 100).await?;
    assert_eq!(after.notes.len(), 1);
    assert_eq!(after.notes[0].role, "note");
    assert_eq!(after.notes[0].content, "KEEP-NOTE");
    assert_eq!(
        memory.history(&transcript, 10).await?[0].text_projection(),
        "CONVERSATION-REMAINS"
    );
    let committed = memory.revision().await?;
    assert!(
        forget_note(
            &memory,
            project.path(),
            Mode::Ifs,
            &identity,
            dream.sequence
        )
        .await
        .is_err()
    );
    assert_eq!(memory.revision().await?, committed);
    Ok(())
}

#[tokio::test]
async fn notes_view_reads_exact_archived_part_and_relationship_only() {
    let (project, memory, scope, mut topology) = seeded(Mode::Ifs).await;
    let archived = topology.parts[1].id.clone();
    topology.parts[1].active = false;
    let relationship = Relationship::new(
        RelationshipKind::Alliance,
        topology.parts[..2]
            .iter()
            .map(|part| part.id.clone())
            .collect(),
    )
    .unwrap();
    topology.relationships.push(relationship.clone());
    memory
        .put(
            &format!("{scope}/ifs/topology"),
            &to_value(&topology).unwrap(),
        )
        .await
        .unwrap();
    for identity in [&archived, &relationship.id] {
        memory
            .append(&notes_key(&scope, Mode::Ifs, identity), "note", identity)
            .await
            .unwrap();
        let view = read_notes(&memory, project.path(), Mode::Ifs, identity, 1)
            .await
            .unwrap();
        assert_eq!(view.identity, *identity);
        assert_ne!(view.notes[0].sequence, 0);
        assert_eq!(view.notes[0].role, "note");
        assert_eq!(view.notes[0].content, *identity);
    }
    let archived_name = topology.parts[1].name.clone();
    assert!(
        read_notes(&memory, project.path(), Mode::Ifs, &archived_name, 1)
            .await
            .is_err()
    );
    assert!(
        read_notes(&memory, project.path(), Mode::Ifs, "unknown", 1)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn notes_view_reports_n_plus_one_and_validates_its_live_mode() -> Result<()> {
    let (project, memory, scope, topology) = seeded(Mode::Ifs).await;
    let identity = topology.parts[0].id.clone();
    for note in ["old", "middle", "new"] {
        memory
            .append(&notes_key(&scope, Mode::Ifs, &identity), "note", note)
            .await?;
    }
    let view = read_notes(&memory, project.path(), Mode::Ifs, &identity, 2).await?;
    assert!(view.truncated);
    assert_eq!(
        view.notes
            .iter()
            .map(|note| note.content.as_str())
            .collect::<Vec<_>>(),
        ["middle", "new"]
    );
    let complete = read_notes(&memory, project.path(), Mode::Ifs, &identity, 3).await?;
    assert!(!complete.truncated);
    assert_eq!(complete.notes.len(), 3);
    let max = read_notes(&memory, project.path(), Mode::Ifs, &identity, 1000).await?;
    assert_eq!(max.requested_limit, 1000);
    assert!(!max.truncated);
    for limit in [0, 1001] {
        assert!(
            read_notes(&memory, project.path(), Mode::Ifs, &identity, limit)
                .await
                .is_err()
        );
    }
    let revision = memory.revision().await?;
    let missing_key = format!("{scope}/freudian/topology");
    assert!(memory.get(&missing_key).await?.is_none());
    assert!(
        read_notes(&memory, project.path(), Mode::Freudian, &identity, 1)
            .await
            .is_err()
    );
    assert!(memory.get(&missing_key).await?.is_none());
    assert_eq!(memory.revision().await?, revision);

    let candidate = memory.begin_candidate("notes-reader-test").await?;
    let candidate_revision = candidate.view().revision().await?;
    assert!(
        read_notes(&candidate.view(), project.path(), Mode::Ifs, &identity, 1)
            .await
            .is_err()
    );
    assert_eq!(candidate.view().revision().await?, candidate_revision);
    assert_eq!(memory.revision().await?, revision);
    Ok(())
}
