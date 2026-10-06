//! Owner-local conversation claims. Catalog checks and replacement take the
//! existing store write guard; connection drop removes only its exact handle.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex as StdMutex, Weak},
    time::Instant,
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    MemoryStore, Mutation, SessionCatalogRecord, SessionLifecycleMutation, SessionLifecycleState,
};

/// Request-local authenticated identity and the caller's published claim.
/// Internal storage workers have no attached caller; remote requests always
/// carry this context, including the explicit absence of a driver proof.
#[derive(Clone, Debug)]
pub(super) struct SessionCaller {
    pub client: Uuid,
    pub proof: Option<SessionDriverProof>,
    local_proof: Option<Weak<StdMutex<Option<SessionDriverProof>>>>,
}

impl SessionCaller {
    fn current_proof(&self) -> Result<Option<SessionDriverProof>> {
        match &self.local_proof {
            None => Ok(self.proof.clone()),
            Some(proof) => Ok(proof
                .upgrade()
                .ok_or(SessionDriverRejected(SessionDriverRefusal::NotDriven))?
                .lock()
                .map_err(|_| anyhow::anyhow!("local driver proof state is unavailable"))?
                .clone()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDriverProof {
    pub session_id: String,
    pub claim_id: Uuid,
    pub service_generation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "expectation",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SessionDriverTarget {
    Absent(String),
    Catalog(Box<SessionCatalogRecord>),
}

impl SessionDriverTarget {
    pub(crate) fn session_id(&self) -> &str {
        match self {
            Self::Absent(id) => id,
            Self::Catalog(record) => &record.session_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDriverSelection {
    pub expected: Option<SessionDriverProof>,
    pub target: SessionDriverTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "outcome",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SessionDriverOutcome {
    InFlight,
    Selected(SessionDriverProof),
    NotSelected(Option<SessionDriverProof>),
    StillUncertain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "refusal", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionDriverRefusal {
    AlreadyDriven { session_id: String, claim_id: Uuid },
    Draining { session_id: String },
    OldClaimChanged,
    CatalogChanged,
    Removed,
    NotDriven,
    SelectionNotAccepted,
}

#[derive(Debug)]
pub struct SessionDriverRejected(pub SessionDriverRefusal);

impl std::fmt::Display for SessionDriverRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            SessionDriverRefusal::AlreadyDriven { session_id, .. } => write!(formatter, "session {session_id} is driven by another Kuru instance; select another session or start a fresh conversation"),
            SessionDriverRefusal::Draining { session_id } => write!(formatter, "session {session_id} still has checked local ownership or draining work; it is not available for another driver"),
            SessionDriverRefusal::OldClaimChanged => formatter.write_str("session driver ownership changed; reconcile the pending selection before continuing"),
            SessionDriverRefusal::CatalogChanged => formatter.write_str("session catalog changed while selecting; inspect the session before trying again"),
            SessionDriverRefusal::Removed => formatter.write_str("session is removed; restore it before resuming"),
            SessionDriverRefusal::NotDriven => formatter.write_str("this instance does not own the session driver; no private session mutation was accepted"),
            SessionDriverRefusal::SelectionNotAccepted => formatter.write_str("the completed session selection did not replace the previous driver; the old session remains selected, inspect the target before retrying"),
        }
    }
}

impl std::error::Error for SessionDriverRejected {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSessionDriver {
    pub proof: SessionDriverProof,
    pub age_millis: u64,
    pub reserved: bool,
}

#[derive(Debug)]
struct Entry {
    client: Uuid,
    connection: Uuid,
    proof: SessionDriverProof,
    selection: SessionDriverSelection,
    created: Instant,
    reserved: bool,
    handle: Weak<ClaimHandleInner>,
}

#[derive(Debug, Default)]
pub(super) struct SessionClaims {
    entries: StdMutex<BTreeMap<String, Entry>>,
}

/// The presence connection's sole guard. A superseded guard cannot clear a
/// transferred connection or a newer claim for the same session.
#[derive(Debug)]
pub(crate) struct SessionClaimHandle {
    inner: Arc<ClaimHandleInner>,
}

#[derive(Debug)]
struct ClaimHandleInner {
    claims: Arc<SessionClaims>,
    connection: Uuid,
    pub(crate) proof: SessionDriverProof,
}

impl SessionClaimHandle {
    pub(crate) fn proof(&self) -> &SessionDriverProof {
        &self.inner.proof
    }
}

impl Drop for ClaimHandleInner {
    fn drop(&mut self) {
        let mut entries = self
            .claims
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if entries
            .get(&self.proof.session_id)
            .is_some_and(|entry| entry.connection == self.connection && entry.proof == self.proof)
        {
            entries.remove(&self.proof.session_id);
        }
    }
}

impl MemoryStore {
    pub(super) fn note_session_catalog_outcome(
        &self,
        outcome: &super::SessionLifecycleOutcome,
    ) -> Result<()> {
        let mut entries = self
            .shared
            .claims
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session claim state is unavailable"))?;
        if let Some(entry) = entries.get_mut(&outcome.session_id) {
            entry.reserved = false;
        }
        Ok(())
    }
    pub(crate) fn with_request_caller(&self, source: &Self) -> Self {
        let mut view = self.clone();
        view.session_caller = source.session_caller.clone();
        view
    }
    pub(crate) fn with_session_caller(
        &self,
        client: Uuid,
        proof: Option<SessionDriverProof>,
    ) -> Self {
        let mut view = self.clone();
        view.session_caller = Some(SessionCaller {
            client,
            proof,
            local_proof: None,
        });
        view
    }

    pub(crate) fn with_local_session_caller(
        &self,
        client: Uuid,
        proof: &Arc<StdMutex<Option<SessionDriverProof>>>,
    ) -> Self {
        let mut view = self.clone();
        view.session_caller = Some(SessionCaller {
            client,
            proof: None,
            local_proof: Some(Arc::downgrade(proof)),
        });
        view
    }

    // The caller holds Shared.write. A claim's own catalog updates do not
    // invalidate its identity; catalog generation is checked at selection.
    fn check_session_claim(&self, session: &str) -> Result<()> {
        let Some(caller) = &self.session_caller else {
            return Ok(());
        };
        let proof = caller.current_proof()?;
        let entries = self
            .shared
            .claims
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session claim state is unavailable"))?;
        let owned = entries.get(session).is_some_and(|entry| {
            entry.client == caller.client && Some(&entry.proof) == proof.as_ref()
        });
        if !owned {
            return Err(SessionDriverRejected(SessionDriverRefusal::NotDriven).into());
        }
        Ok(())
    }

    fn check_state_claims(&self, values: &[(String, String)]) -> Result<()> {
        let prefix = format!("{}/session/", self.shared.project_scope);
        for (key, _) in values {
            if let Some(tail) = key.strip_prefix(&prefix) {
                self.check_session_claim(tail.split('/').next().unwrap_or(tail))?;
            }
        }
        Ok(())
    }

    fn check_reasoning_claims(&self, records: &[(String, String)]) -> Result<()> {
        for (_, value) in records {
            let record: super::ReasoningSummaryRecord = serde_json::from_str(value)?;
            self.check_session_claim(&record.session_id)?;
        }
        Ok(())
    }

    pub(super) fn check_mutation_claim(&self, mutation: &Mutation) -> Result<()> {
        // A previously claimed driver cannot continue shared writes either.
        if let Some(proof) = self
            .session_caller
            .as_ref()
            .map(SessionCaller::current_proof)
            .transpose()?
            .flatten()
        {
            self.check_session_claim(&proof.session_id)?;
        }
        match mutation {
            Mutation::Append {
                namespace,
                session_id,
                ..
            } => {
                if let Some(session) = session_id {
                    self.check_session_claim(session)?;
                }
                self.check_transcript_claim(namespace)
            }
            Mutation::State(values) | Mutation::StateConditional { values, .. } => {
                self.check_state_claims(values)
            }
            Mutation::PrivateReasoningSummaries(records) => self.check_reasoning_claims(records),
            Mutation::Checkpoint {
                namespace,
                session_id,
                values,
                ..
            } => {
                if let Some(session) = session_id {
                    self.check_session_claim(session)?;
                }
                self.check_transcript_claim(namespace)?;
                self.check_state_claims(values)
            }
            Mutation::ContextSummary {
                record,
                private_reasoning,
                ..
            } => {
                self.check_session_claim(&record.session_id)?;
                self.check_reasoning_claims(private_reasoning)
            }
            Mutation::Clear(namespace) | Mutation::ForgetNote { namespace, .. } => {
                self.check_transcript_claim(namespace)
            }
        }
    }

    fn check_transcript_claim(&self, namespace: &str) -> Result<()> {
        if let Some(session) =
            namespace.strip_prefix(&format!("{}/transcript/", self.shared.project_scope))
        {
            self.check_session_claim(session)?;
        }
        Ok(())
    }

    pub(super) fn check_session_lifecycle_claim(
        &self,
        mutation: &SessionLifecycleMutation,
    ) -> Result<()> {
        let (session, forbid_driven) = match mutation {
            SessionLifecycleMutation::Create { session_id, .. }
            | SessionLifecycleMutation::Rename { session_id, .. }
            | SessionLifecycleMutation::Restore { session_id, .. } => (session_id, false),
            SessionLifecycleMutation::Remove { session_id, .. } => (session_id, true),
            SessionLifecycleMutation::Fork {
                source_session_id, ..
            } => (source_session_id, false),
        };
        let entries = self
            .shared
            .claims
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session claim state is unavailable"))?;
        if let Some(entry) = entries.get(session) {
            let owned = match &self.session_caller {
                Some(caller) => {
                    caller.client == entry.client
                        && caller.current_proof()?.as_ref() == Some(&entry.proof)
                }
                None => false,
            };
            if forbid_driven || !owned {
                return Err(SessionDriverRejected(SessionDriverRefusal::AlreadyDriven {
                    session_id: session.clone(),
                    claim_id: entry.proof.claim_id,
                })
                .into());
            }
        }
        Ok(())
    }

    pub(crate) async fn select_session_driver(
        &self,
        client: Uuid,
        connection: Uuid,
        generation: &str,
        request_id: Uuid,
        selection: &SessionDriverSelection,
    ) -> Result<SessionClaimHandle> {
        self.writable()?;
        ensure!(
            self.branch == "main",
            "session selection requires the live memory view"
        );
        validate_selection(selection)?;
        validate_generation(generation)?;
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        {
            let entries = self
                .shared
                .claims
                .entries
                .lock()
                .map_err(|_| anyhow::anyhow!("session claim state is unavailable"))?;
            if let Some(entry) = entries.values().find(|entry| entry.client == client)
                && entry.proof.claim_id == request_id
                && entry.proof.service_generation == generation
                && entry.connection == connection
                && entry.selection == *selection
            {
                let inner = entry
                    .handle
                    .upgrade()
                    .ok_or(SessionDriverRejected(SessionDriverRefusal::OldClaimChanged))?;
                return Ok(SessionClaimHandle { inner });
            }
            if entries
                .values()
                .any(|entry| entry.proof.claim_id == request_id)
            {
                return Err(SessionDriverRejected(SessionDriverRefusal::OldClaimChanged).into());
            }
        }
        let catalog = self
            .session_catalog_record(selection.target.session_id())
            .await?;
        match (&selection.target, &catalog) {
            (SessionDriverTarget::Absent(_), None) => {}
            (SessionDriverTarget::Catalog(expected), Some(actual))
                if expected.as_ref() == actual =>
            {
                if actual.lifecycle_state != SessionLifecycleState::Active {
                    return Err(SessionDriverRejected(SessionDriverRefusal::Removed).into());
                }
            }
            _ => return Err(SessionDriverRejected(SessionDriverRefusal::CatalogChanged).into()),
        }
        let mut entries = self
            .shared
            .claims
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session claim state is unavailable"))?;
        let old = entries.values().find(|entry| entry.client == client);
        if old.map(|entry| &entry.proof) != selection.expected.as_ref()
            || old.is_some_and(|entry| {
                entry.connection != connection || entry.proof.service_generation != generation
            })
        {
            return Err(SessionDriverRejected(SessionDriverRefusal::OldClaimChanged).into());
        }
        if let Some(entry) = entries.get(selection.target.session_id())
            && entry.client != client
        {
            return Err(SessionDriverRejected(SessionDriverRefusal::AlreadyDriven {
                session_id: entry.proof.session_id.clone(),
                claim_id: entry.proof.claim_id,
            })
            .into());
        }
        if let Some(old) = &selection.expected {
            entries.remove(&old.session_id);
        }
        let proof = SessionDriverProof {
            session_id: selection.target.session_id().to_owned(),
            claim_id: request_id,
            service_generation: generation.to_owned(),
        };
        let inner = Arc::new(ClaimHandleInner {
            claims: self.shared.claims.clone(),
            connection,
            proof: proof.clone(),
        });
        entries.insert(
            proof.session_id.clone(),
            Entry {
                client,
                connection,
                proof,
                selection: selection.clone(),
                created: Instant::now(),
                reserved: catalog.is_none(),
                handle: Arc::downgrade(&inner),
            },
        );
        Ok(SessionClaimHandle { inner })
    }

    pub(crate) fn live_session_drivers(&self) -> Result<Vec<LiveSessionDriver>> {
        self.readable()?;
        let entries = self
            .shared
            .claims
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session claim state is unavailable"))?;
        Ok(entries
            .values()
            .map(|entry| LiveSessionDriver {
                proof: entry.proof.clone(),
                age_millis: u64::try_from(entry.created.elapsed().as_millis()).unwrap_or(u64::MAX),
                reserved: entry.reserved,
            })
            .collect())
    }

    /// Positive full-tuple evidence only. An absent connection-local claim
    /// cannot prove the result of a completed switch after its owner changed.
    pub(crate) async fn session_driver_outcome(
        &self,
        client: Uuid,
        generation: &str,
        id: Uuid,
        selection: &SessionDriverSelection,
    ) -> Result<SessionDriverOutcome> {
        validate_selection(selection)?;
        validate_generation(generation)?;
        self.readable()?;
        let _guard = self.shared.write.lock().await;
        let entries = self
            .shared
            .claims
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session claim state is unavailable"))?;
        let Some(entry) = entries.values().find(|entry| entry.client == client) else {
            return Ok(SessionDriverOutcome::StillUncertain);
        };
        if entry.proof.service_generation != generation {
            return Ok(SessionDriverOutcome::StillUncertain);
        }
        if entry.proof.claim_id == id && entry.selection == *selection {
            Ok(SessionDriverOutcome::Selected(entry.proof.clone()))
        } else if Some(&entry.proof) == selection.expected.as_ref() {
            Ok(SessionDriverOutcome::NotSelected(Some(entry.proof.clone())))
        } else {
            Ok(SessionDriverOutcome::StillUncertain)
        }
    }

    /// A checked successor presence replaces only the exact original tuple.
    /// The prior connection's later EOF cannot clear the transferred claim.
    pub(crate) async fn reattach_session_driver(
        &self,
        client: Uuid,
        connection: Uuid,
        generation: &str,
        id: Uuid,
        selection: &SessionDriverSelection,
    ) -> Result<SessionClaimHandle> {
        self.writable()?;
        validate_selection(selection)?;
        validate_generation(generation)?;
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        let mut entries = self
            .shared
            .claims
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session claim state is unavailable"))?;
        let entry = entries
            .values_mut()
            .find(|entry| entry.client == client)
            .context("original session presence is unavailable; selection remains uncertain")?;
        let selected = entry.proof.claim_id == id && entry.selection == *selection;
        let unchanged = selection.expected.as_ref() == Some(&entry.proof);
        ensure!(
            entry.client == client
                && entry.proof.service_generation == generation
                && (selected || unchanged),
            "session presence reattachment did not match the exact original selection"
        );
        let inner = Arc::new(ClaimHandleInner {
            claims: self.shared.claims.clone(),
            connection,
            proof: entry.proof.clone(),
        });
        entry.connection = connection;
        entry.handle = Arc::downgrade(&inner);
        Ok(SessionClaimHandle { inner })
    }
}

pub(crate) fn validate_selection(selection: &SessionDriverSelection) -> Result<()> {
    super::session_identity("selected session", selection.target.session_id(), 128)?;
    if let Some(expected) = &selection.expected {
        super::session_identity("previous session", &expected.session_id, 128)?;
        validate_generation(&expected.service_generation)?;
    }
    if let SessionDriverTarget::Catalog(record) = &selection.target {
        super::validate_session_catalog(record)?;
    }
    Ok(())
}

fn validate_generation(generation: &str) -> Result<()> {
    ensure!(
        Uuid::parse_str(generation)?.to_string() == generation,
        "invalid session claim service generation"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate as kuru_memory;
    use kuru_core::{Message, Mode};
    use serde_json::json;

    #[test]
    fn session_claim_async_fixtures_use_the_closing_scope() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        crate::test_support::assert_async_tests_run_in_closing(
            &root,
            &[root.join("store/session_claims.rs")],
        );
    }

    async fn fixture() -> Result<MemoryStore> {
        let store = MemoryStore::temporary().await?;
        crate::test_support::closing::register(store.server_for_teardown());
        Ok(store)
    }

    async fn target(store: &MemoryStore, id: &str) -> Result<SessionDriverTarget> {
        Ok(SessionDriverTarget::Catalog(Box::new(
            store.session_catalog_record(id).await?.unwrap(),
        )))
    }

    #[tokio::test]
    async fn session_claim_switch_is_atomic_and_private_mutations_are_fenced() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let store = fixture().await?;
            for id in ["first", "target"] {
                store.create_session_catalog(id, Mode::Ifs, id).await?;
            }
            let generation = Uuid::new_v4().to_string();
            let client = Uuid::new_v4();
            let connection = Uuid::new_v4();
            let selection = SessionDriverSelection {
                expected: None,
                target: target(&store, "first").await?,
            };
            let claim = store
                .select_session_driver(client, connection, &generation, Uuid::new_v4(), &selection)
                .await?;
            let driven = store.with_session_caller(client, Some(claim.proof().clone()));
            let before = store.revision().await?;
            let error = store
                .select_session_driver(
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    &generation,
                    Uuid::new_v4(),
                    &selection,
                )
                .await
                .unwrap_err();
            assert!(matches!(
                error.downcast_ref::<SessionDriverRejected>().unwrap().0,
                SessionDriverRefusal::AlreadyDriven { .. }
            ));
            assert_eq!(store.revision().await?, before);
            let first = store.session_catalog_record("first").await?.unwrap();
            assert!(
                store
                    .rename_session_catalog("first", first.lifecycle_generation, "foreign")
                    .await
                    .is_err()
            );
            driven
                .rename_session_catalog("first", first.lifecycle_generation, "own rename")
                .await?;
            let renamed = store.session_catalog_record("first").await?.unwrap();
            assert!(
                driven
                    .remove_session_catalog("first", renamed.lifecycle_generation)
                    .await
                    .is_err()
            );
            driven
                .append_session_message(
                    "actor/history",
                    "first",
                    &Message::text("user", "first private sentinel"),
                )
                .await?;

            let target_snapshot = store.session_catalog_record("target").await?.unwrap();
            let switch = SessionDriverSelection {
                expected: Some(claim.proof().clone()),
                target: SessionDriverTarget::Catalog(Box::new(target_snapshot.clone())),
            };
            store
                .rename_session_catalog("target", target_snapshot.lifecycle_generation, "changed")
                .await?;
            let before = store.revision().await?;
            let error = store
                .select_session_driver(client, connection, &generation, Uuid::new_v4(), &switch)
                .await
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<SessionDriverRejected>().unwrap().0,
                SessionDriverRefusal::CatalogChanged
            );
            assert_eq!(store.revision().await?, before);
            driven
                .put(
                    &format!("{}/session/first", store.shared.project_scope),
                    &json!({"owned":true}),
                )
                .await?;

            let switch = SessionDriverSelection {
                target: target(&store, "target").await?,
                ..switch
            };
            let request = Uuid::new_v4();
            let next = store
                .select_session_driver(client, connection, &generation, request, &switch)
                .await?;
            let duplicate = store
                .select_session_driver(client, connection, &generation, request, &switch)
                .await?;
            assert_eq!(next.proof(), duplicate.proof());
            drop(duplicate);
            drop(claim); // The old connection guard cannot remove the new claim.
            assert_eq!(store.live_session_drivers()?.len(), 1);
            let before = store.revision().await?;
            let error = driven
                .checkpoint_session(
                    "actor/history",
                    "first",
                    &[Message::text("assistant", "rejected sentinel")],
                    &[("companion".into(), json!(true))],
                )
                .await
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<SessionDriverRejected>().unwrap().0,
                SessionDriverRefusal::NotDriven
            );
            assert_eq!(store.revision().await?, before);
            assert!(store.get("companion").await?.is_none());
            assert_eq!(
                store
                    .session_history_window("actor/history", "first", 8)
                    .await?
                    .messages
                    .len(),
                1
            );
            let unclaimed = store.with_session_caller(Uuid::new_v4(), None);
            assert!(
                unclaimed
                    .put(
                        &format!("{}/session/target/turn/x", store.shared.project_scope),
                        &json!({})
                    )
                    .await
                    .is_err()
            );
            let selected = store.with_session_caller(client, Some(next.proof().clone()));
            selected
                .append_session_message(
                    "actor/history",
                    "target",
                    &Message::text("user", "target sentinel"),
                )
                .await?;
            drop(next);
            assert!(store.live_session_drivers()?.is_empty());
            store.close().await?;
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn session_claim_exact_reattachment_survives_prior_connection_eof() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let store = fixture().await?;
            let generation = Uuid::new_v4().to_string();
            let client = Uuid::new_v4();
            let request = Uuid::new_v4();
            let selection = SessionDriverSelection {
                expected: None,
                target: SessionDriverTarget::Absent("reserved".into()),
            };
            let original = store
                .select_session_driver(client, Uuid::new_v4(), &generation, request, &selection)
                .await?;
            let proof = original.proof().clone();
            assert_eq!(
                store
                    .session_driver_outcome(client, &generation, request, &selection)
                    .await?,
                SessionDriverOutcome::Selected(proof.clone())
            );
            assert!(
                store
                    .reattach_session_driver(
                        client,
                        Uuid::new_v4(),
                        &generation,
                        Uuid::new_v4(),
                        &selection
                    )
                    .await
                    .is_err()
            );
            let restored = store
                .reattach_session_driver(client, Uuid::new_v4(), &generation, request, &selection)
                .await?;
            drop(original);
            assert_eq!(store.live_session_drivers()?[0].proof, proof);
            let bound = store.with_session_caller(client, Some(proof));
            bound
                .create_session_catalog("reserved", Mode::Ifs, "reserved")
                .await?;
            let repeat = store
                .select_session_driver(
                    client,
                    restored.inner.connection,
                    &generation,
                    request,
                    &selection,
                )
                .await?;
            drop(repeat); // Catalog creation does not invalidate idempotence.
            drop(restored);
            assert!(store.live_session_drivers()?.is_empty());
            assert_eq!(
                store
                    .session_driver_outcome(client, &generation, request, &selection)
                    .await?,
                SessionDriverOutcome::StillUncertain
            );
            store.close().await?;
            Ok(())
        })
        .await
    }
}
