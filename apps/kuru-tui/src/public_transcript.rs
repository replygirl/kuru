//! Selected public reads at the app adapter boundary, never in render models.

use anyhow::Result;
use kuru_memory::{MemoryStore, PublicTranscriptCursor, PublicTranscriptPage};
use kuru_runtime::Harness;

#[derive(Clone)]
pub(crate) struct Source {
    memory: MemoryStore,
    pub(crate) session: String,
    pub(crate) view: String,
}

impl Source {
    pub(crate) fn capture(harness: &Harness) -> Self {
        Self {
            memory: harness.memory().clone(),
            session: harness.session.id.clone(),
            view: harness.memory().selected_view_name().to_owned(),
        }
    }

    pub(crate) async fn page(
        &self,
        cursor: Option<&PublicTranscriptCursor>,
    ) -> Result<PublicTranscriptPage> {
        self.memory
            .public_transcript_page(&self.session, cursor, 32)
            .await
    }
}
