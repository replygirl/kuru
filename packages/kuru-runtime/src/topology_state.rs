//! Split persistence codec and coherent reconstruction of the public topology.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use kuru_core::{ModeProfile, Part, Relationship, StateKeys};
use kuru_memory::{MemoryStore, StateExpectation, VersionedValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{StateReport, Topology, engine::Session};

pub(crate) const MEMBERSHIP_FORMAT: &str = "membership.v1";
const REPORT_FORMAT: &str = "state_report.v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MembershipRecord {
    #[serde(default)]
    pub record_format: String,
    pub parts: Vec<Part>,
    pub relationships: Vec<Relationship>,
}

impl MembershipRecord {
    pub fn from_topology(topology: &Topology) -> Self {
        Self {
            record_format: MEMBERSHIP_FORMAT.into(),
            parts: topology.parts.clone(),
            relationships: topology.relationships.clone(),
        }
    }

    pub fn decode(value: Value, legacy_undo: bool) -> Result<Self> {
        let mut record: Self = serde_json::from_value(value)?;
        ensure!(
            record.record_format == MEMBERSHIP_FORMAT
                || (legacy_undo && record.record_format.is_empty()),
            "unsupported membership record format"
        );
        record.record_format = MEMBERSHIP_FORMAT.into();
        Ok(record)
    }
}

#[derive(Serialize, Deserialize)]
struct ReportRecord {
    record_format: String,
    identity: String,
    report: Value,
}

pub(crate) fn report_value(identity: &str, report: &StateReport) -> Result<Value> {
    serde_json::to_value(ReportRecord {
        record_format: REPORT_FORMAT.into(),
        identity: identity.into(),
        report: serde_json::to_value(report)?,
    })
    .map_err(Into::into)
}

pub(crate) fn decode_report(
    keys: &StateKeys,
    key: &str,
    value: Value,
) -> Result<(String, StateReport)> {
    let record: ReportRecord = serde_json::from_value(value)?;
    ensure!(
        record.record_format == REPORT_FORMAT,
        "unsupported state report record format"
    );
    ensure!(
        keys.state_report(&record.identity) == key,
        "state report identity differs from its storage key"
    );
    Ok((record.identity, serde_json::from_value(record.report)?))
}

/// A complete inventory comes only from a full prefix scan. Thereafter runtime
/// reports can create rows only for admitted part/relationship IDs; migration
/// preserves the existing extras. Include every retained member ID in a warm
/// batch, not just active actors. Mode/view switches invalidate this inventory.
#[derive(Clone, Default)]
pub(crate) struct ReportInventory {
    membership_key: String,
    report_keys: BTreeSet<String>,
}

pub(crate) struct TopologySnapshot {
    pub topology: Topology,
    pub expectation: StateExpectation,
    pub inventory: ReportInventory,
}

impl TopologySnapshot {
    fn assemble(
        profile: &ModeProfile,
        keys: &StateKeys,
        membership: Option<VersionedValue>,
        session: Option<VersionedValue>,
        reports: Vec<(String, VersionedValue)>,
    ) -> Result<Self> {
        let expectation = membership.as_ref().map_or(StateExpectation::Absent, |row| {
            StateExpectation::Version(row.version)
        });
        let members = membership
            .map(|row| MembershipRecord::decode(row.value, false))
            .transpose()?
            .unwrap_or_else(|| MembershipRecord {
                record_format: MEMBERSHIP_FORMAT.into(),
                parts: profile.roles.seeds(),
                relationships: vec![],
            });
        let session: Option<Session> = session
            .map(|row| serde_json::from_value(row.value))
            .transpose()?;
        let mut inventory = ReportInventory {
            membership_key: keys.membership.clone(),
            report_keys: BTreeSet::new(),
        };
        let mut states = BTreeMap::new();
        for (key, row) in reports {
            let (identity, report) = decode_report(keys, &key, row.value)?;
            ensure!(
                states.insert(identity, report).is_none(),
                "duplicate retained state report identity"
            );
            inventory.report_keys.insert(key);
        }
        let mut topology = Topology {
            parts: members.parts,
            relationships: members.relationships,
            states,
            focus: session.as_ref().and_then(|session| session.focus.clone()),
        };
        clear_inactive_focus(&mut topology);
        Ok(Self {
            topology,
            expectation,
            inventory,
        })
    }
}

pub(crate) fn clear_inactive_focus(topology: &mut Topology) {
    if topology.focus.as_ref().is_some_and(|focus| {
        focus.remaining == 0
            || !(topology
                .parts
                .iter()
                .any(|part| part.active && part.id == focus.id)
                || topology.relationships.iter().any(|relation| {
                    relation.id == focus.id
                        && relation.members.iter().all(|id| {
                            topology
                                .parts
                                .iter()
                                .any(|part| part.active && &part.id == id)
                        })
                }))
    }) {
        topology.focus = None;
    }
}

pub(crate) async fn load(
    memory: &MemoryStore,
    keys: &StateKeys,
    profile: &ModeProfile,
    session_key: Option<&str>,
    inventory: Option<&ReportInventory>,
) -> Result<TopologySnapshot> {
    if let Some(inventory) =
        inventory.filter(|inventory| inventory.membership_key == keys.membership)
    {
        let header = memory.get_versioned(&keys.membership).await?;
        let members = header
            .as_ref()
            .map(|row| MembershipRecord::decode(row.value.clone(), false))
            .transpose()?;
        let mut report_keys = inventory.report_keys.clone();
        if let Some(members) = members {
            report_keys.extend(members.parts.iter().map(|part| keys.state_report(&part.id)));
            report_keys.extend(
                members
                    .relationships
                    .iter()
                    .map(|relation| keys.state_report(&relation.id)),
            );
        }
        let mut request = vec![keys.membership.clone()];
        if let Some(key) = session_key {
            request.push(key.into());
        }
        request.extend(report_keys);
        if request.len() <= 256 {
            // The read is one operation; a size refusal falls back to a cut.
            // A failed transport/corrupt row remains an error if the cut also
            // cannot read it; no mutation is retried here.
            if let Ok(rows) = memory.get_many_versioned(&request).await {
                let mut rows = rows.into_iter();
                let (_, membership) = rows.next().context("membership batch result missing")?;
                if membership == header {
                    let session = if session_key.is_some() {
                        rows.next().context("session batch result missing")?.1
                    } else {
                        None
                    };
                    return TopologySnapshot::assemble(
                        profile,
                        keys,
                        membership,
                        session,
                        rows.filter_map(|(key, value)| value.map(|value| (key, value)))
                            .collect(),
                    );
                }
            }
        }
    }
    let cut = memory.begin_state_read_cut().await?;
    let result = async {
        let session = if let Some(key) = session_key {
            cut.get_versioned(key).await?
        } else {
            None
        };
        if cut.provenance().schema_version < 10 {
            let mut topology: Topology = cut
                .get_versioned(&keys.topology)
                .await?
                .map(|row| serde_json::from_value(row.value))
                .transpose()?
                .unwrap_or_else(|| Topology {
                    parts: profile.roles.seeds(),
                    relationships: vec![],
                    states: BTreeMap::new(),
                    focus: None,
                });
            // Historical views remain inspectable; legacy project focus is
            // never assigned to a session and this path does not materialize.
            let session: Option<Session> = session
                .map(|row| serde_json::from_value(row.value))
                .transpose()?;
            topology.focus = session.and_then(|session| session.focus);
            clear_inactive_focus(&mut topology);
            return Ok(TopologySnapshot {
                topology,
                expectation: StateExpectation::Absent,
                inventory: ReportInventory::default(),
            });
        }
        let membership = cut.get_versioned(&keys.membership).await?;
        let mut reports = vec![];
        let mut cursor = None;
        loop {
            let page = cut.page(&keys.state_prefix, cursor).await?;
            reports.extend(page.values);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        TopologySnapshot::assemble(profile, keys, membership, session, reports)
    }
    .await;
    let closed = cut.close().await;
    match result {
        Ok(snapshot) => {
            closed?;
            Ok(snapshot)
        }
        Err(error) => {
            let _ = closed;
            Err(error)
        }
    }
}
