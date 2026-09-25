use anyhow::{Context, Result};
use kuru_core::Message;
use kuru_memory::{MemoryStore, PublicTurnSettlement, SessionTurnCheckpoint};

/// Seed the same settled public boundary that an ordinary turn publishes.
/// Direct raw transcript appends after v7 have no catalog provenance and are
/// intentionally absent from selected-session public context.
pub(crate) async fn settled_public_turn(
    memory: &MemoryStore,
    scope: &str,
    session_id: &str,
    turn_id: &str,
    speaker_id: &str,
    user: &str,
    answer: &str,
) -> Result<()> {
    let catalog = memory
        .session_catalog_record(session_id)
        .await?
        .context("public fixture session is absent from the catalog")?;
    let namespace = format!("{scope}/transcript/{session_id}");
    let expected_transcript_rows = memory.history_window(&namespace, 0).await?.total_rows;
    memory
        .checkpoint_session_turn(
            &namespace,
            session_id,
            &[Message::text("user", user)],
            &[],
            &SessionTurnCheckpoint::Admit {
                expected_generation: catalog.lifecycle_generation,
                turn_id: turn_id.into(),
                label: None,
                expected_transcript_rows: Some(expected_transcript_rows),
            },
        )
        .await?;
    memory
        .checkpoint_session_turn(
            &namespace,
            session_id,
            &[Message::text("assistant", answer)],
            &[],
            &SessionTurnCheckpoint::Settle {
                expected_generation: catalog.lifecycle_generation,
                turn_id: turn_id.into(),
                settlement: PublicTurnSettlement::Completed,
                speaker_id: speaker_id.into(),
            },
        )
        .await
}
