//! A tiny completed chain proof, owned by one view/attachment's clones.
//! It retains coordinates only; every read still observes selected HEAD.

use super::*;

const MAX_PROOF_BYTES: usize = 2 * 1024;
const MAX_PROOF_PAGES: usize = 2;

#[derive(Clone, Debug, Serialize)]
pub(super) struct PublicTranscriptProof {
    branch: String,
    session: String,
    revision: String,
    head: Option<String>,
    pub(super) turn_count: u64,
    pages: Vec<[Option<PublicTranscriptPosition>; 2]>,
}

impl PublicTranscriptProof {
    pub(super) fn matches(
        &self,
        branch: &str,
        session: &str,
        revision: &str,
        head: &Option<String>,
    ) -> bool {
        self.branch == branch
            && self.session == session
            && self.revision == revision
            && &self.head == head
    }

    pub(super) fn admits(&self, requested: &Option<PublicTranscriptPosition>) -> bool {
        let head = self
            .head
            .as_ref()
            .map(|node_id| PublicTranscriptPosition::Turn {
                node_id: node_id.clone(),
            });
        requested == &head
            || self
                .pages
                .iter()
                .flatten()
                .any(|position| position == requested)
    }

    pub(super) fn completed(
        previous: Option<Self>,
        page: &PublicTranscriptPage,
        entry: Option<PublicTranscriptPosition>,
        turn_count: u64,
    ) -> Option<Self> {
        let mut proof = previous
            .filter(|proof| {
                proof.matches(
                    &page.view,
                    &page.session_id,
                    &page.revision,
                    &page.head_node_id,
                )
            })
            .unwrap_or_else(|| Self {
                branch: page.view.clone(),
                session: page.session_id.clone(),
                revision: page.revision.clone(),
                head: page.head_node_id.clone(),
                turn_count,
                pages: Vec::new(),
            });
        proof.turn_count = turn_count;
        if proof.pages.len() == MAX_PROOF_PAGES {
            proof.pages.remove(0);
        }
        proof
            .pages
            .push([entry, page.next.as_ref().map(|cursor| cursor.next.clone())]);
        // Failure/oversize only disables reuse, never rejects the page.
        (serde_json::to_vec(&proof).ok()?.len() <= MAX_PROOF_BYTES).then_some(proof)
    }
}

impl MemoryStore {
    /// The owner store serves many clients. Each authenticated attachment
    /// gets its own coordinates-only proof slot; no authority/pool changes.
    pub(crate) fn independent_public_reader(&self) -> Self {
        let mut view = self.clone();
        view.public_transcript_proof = Arc::default();
        view
    }
}
