//! Pure reference decisions for the four persisted framework modes.

use crate::{Mode, Part, Relationship, RelationshipKind};
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub trait RolesPolicy: Send + Sync {
    fn seeds(&self) -> Vec<Part>;
    fn authored_order(&self) -> Vec<String> {
        self.seeds().into_iter().map(|part| part.id).collect()
    }
    fn required_roles(&self) -> BTreeSet<String> {
        self.seeds().into_iter().map(|part| part.role).collect()
    }
    fn accepts_role(&self, role: &str) -> bool {
        self.required_roles().contains(role)
    }
}

pub trait PeeringPolicy: Send + Sync {
    fn allows_direct(&self, sender: &str, recipient: &str, active: &BTreeSet<String>) -> bool;
    fn relationship(
        &self,
        origin: &RelationshipOrigin,
        kind: RelationshipKind,
        members: Vec<String>,
        active_parts: &BTreeSet<String>,
    ) -> Result<Relationship>;
}

/// The checked source of a relationship proposal.
///
/// `Peer` is constructed by the runtime only after it has resolved the live
/// actor. A part supplies an empty `sender_members`; a relationship actor
/// supplies its canonical member list. The policy still validates that context
/// before applying the participation rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationshipOrigin {
    User,
    Peer {
        sender: String,
        sender_members: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contribution {
    pub sender: String,
    pub text: String,
}

pub trait FlowPolicy: Send + Sync {
    fn initial_recipients(&self, target: Option<&str>, active_parts: &[String]) -> Vec<String>;
    fn next_recipients(&self, pending: &BTreeSet<String>) -> Vec<String>;
    fn consultation_recipients(
        &self,
        recipient: &str,
        live_identities: &BTreeSet<String>,
    ) -> Vec<String>;
    fn shared_contributions(
        &self,
        speaker: &str,
        relationship: Option<&Relationship>,
        drafts: &BTreeMap<String, String>,
    ) -> Vec<Contribution>;
}

pub struct FacingInput<'a> {
    pub target: Option<&'a str>,
    pub focus: Option<(&'a str, usize)>,
    pub live: &'a BTreeSet<String>,
    pub drafts: &'a BTreeSet<String>,
    pub activation: &'a BTreeMap<String, f64>,
    pub previous_completed: Option<&'a str>,
    pub authored_order: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacingDecision {
    pub speaker: String,
    pub reason: &'static str,
}

pub trait FacingPolicy: Send + Sync {
    fn choose(&self, input: FacingInput<'_>) -> Result<FacingDecision>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorPhase {
    Deliberate,
    Speak,
    Consult,
    Dream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextSource {
    OwnHistory(String),
    OwnNotes(String),
    PublicTranscript,
    ExplicitInput,
}

pub trait VisibilityPolicy: Send + Sync {
    fn context_sources(&self, identity: &str, phase: ActorPhase) -> Vec<ContextSource>;
    fn allows_delivery(&self, sender: &str, recipient: &str, active: &BTreeSet<String>) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateKeys {
    pub topology: String,
    pub dream_undo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationPlan {
    pub participants: Vec<String>,
    pub prompt: String,
    pub phase: String,
    pub max_proposals_per_part: usize,
}

pub trait MemoryPolicy: Send + Sync {
    fn identity_namespace(&self, scope: &str, mode: Mode, identity: &str) -> String;
    fn transcript_namespace(&self, scope: &str, session: &str) -> String;
    fn state_keys(&self, scope: &str, mode: Mode) -> StateKeys;
    fn consolidation_plan(&self, active_parts: &[String]) -> ConsolidationPlan;
}

#[derive(Clone)]
pub struct ModeProfile {
    pub mode: Mode,
    pub roles: Arc<dyn RolesPolicy>,
    pub peering: Arc<dyn PeeringPolicy>,
    pub flow: Arc<dyn FlowPolicy>,
    pub facing: Arc<dyn FacingPolicy>,
    pub visibility: Arc<dyn VisibilityPolicy>,
    pub memory: Arc<dyn MemoryPolicy>,
}

impl ModeProfile {
    pub fn builtin(mode: Mode) -> Self {
        let profile = Self {
            mode,
            roles: Arc::new(ReferenceRoles(mode)),
            peering: Arc::new(ReferencePeering),
            flow: Arc::new(ReferenceFlow),
            facing: Arc::new(ReferenceFacing),
            visibility: Arc::new(ReferenceVisibility),
            memory: Arc::new(ReferenceMemory),
        };
        profile
            .validate(usize::MAX)
            .expect("built-in mode profile is valid");
        profile
    }

    pub fn validate(&self, max_parts: usize) -> Result<()> {
        let seeds = self.roles.seeds();
        ensure!(
            !seeds.is_empty() && seeds.len() <= max_parts,
            "invalid mode seed count"
        );
        let ids = seeds
            .iter()
            .map(|part| part.id.as_str())
            .collect::<BTreeSet<_>>();
        ensure!(ids.len() == seeds.len(), "duplicate mode seed ID");
        let names = seeds
            .iter()
            .map(|part| part.name.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        ensure!(names.len() == seeds.len(), "duplicate mode seed name");
        ensure!(
            seeds.iter().all(|part| {
                part.active
                    && !part.id.trim().is_empty()
                    && !part.id.contains('\0')
                    && !part.name.trim().is_empty()
                    && !part.name.contains('\0')
                    && !part.role.trim().is_empty()
                    && !part.instruction.trim().is_empty()
                    && crate::canonical_peer_instruction(&part.instruction)
                        .is_ok_and(|wrapped| wrapped == part.instruction)
            }),
            "invalid mode seed identity or instruction"
        );
        let authored = self.roles.authored_order();
        ensure!(
            authored.len() == seeds.len()
                && authored.iter().collect::<BTreeSet<_>>().len() == seeds.len()
                && authored.iter().all(|id| ids.contains(id.as_str())),
            "invalid authored seed order"
        );
        let required = self.roles.required_roles();
        ensure!(
            !required.is_empty()
                && required
                    .iter()
                    .all(|role| seeds.iter().any(|part| &part.role == role)),
            "required mode role is uncovered"
        );
        for seed in &seeds {
            for phase in [
                ActorPhase::Deliberate,
                ActorPhase::Speak,
                ActorPhase::Consult,
                ActorPhase::Dream,
            ] {
                validate_context_sources(
                    &seed.id,
                    &self.visibility.context_sources(&seed.id, phase),
                )?;
            }
            validate_identity_namespace(
                "project/example",
                self.mode,
                &seed.id,
                &self
                    .memory
                    .identity_namespace("project/example", self.mode, &seed.id),
            )?;
        }
        ensure!(
            self.memory
                .transcript_namespace("project/example", "session")
                == "project/example/transcript/session",
            "invalid mode transcript namespace"
        );
        let state = self.memory.state_keys("project/example", self.mode);
        ensure!(
            state.topology == format!("project/example/{}/topology", self.mode)
                && state.dream_undo == format!("project/example/{}/dream-undo", self.mode),
            "invalid mode state keys"
        );
        let active = seeds.iter().map(|part| part.id.clone()).collect::<Vec<_>>();
        let active_set = active.iter().cloned().collect::<BTreeSet<_>>();
        validate_consolidation_plan(&self.memory.consolidation_plan(&active), &active_set)?;
        Ok(())
    }
}

pub fn validate_context_sources(identity: &str, sources: &[ContextSource]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for source in sources {
        let key = match source {
            ContextSource::OwnHistory(id) if id == identity => 0,
            ContextSource::OwnNotes(id) if id == identity => 1,
            ContextSource::PublicTranscript => 2,
            ContextSource::ExplicitInput => 3,
            _ => anyhow::bail!("mode policy requested another identity's private context"),
        };
        ensure!(seen.insert(key), "duplicate mode context source");
    }
    ensure!(seen.contains(&3), "mode policy omitted explicit input");
    Ok(())
}

pub fn validate_consolidation_plan(
    plan: &ConsolidationPlan,
    active_parts: &BTreeSet<String>,
) -> Result<()> {
    ensure!(
        plan.participants
            .iter()
            .all(|participant| active_parts.contains(participant)),
        "mode consolidation plan selected an inactive participant"
    );
    ensure!(
        plan.participants.iter().collect::<BTreeSet<_>>().len() == plan.participants.len(),
        "mode consolidation plan repeated a participant"
    );
    ensure!(
        !plan.prompt.trim().is_empty() && !plan.phase.trim().is_empty(),
        "mode consolidation plan has blank prompt or phase"
    );
    ensure!(
        plan.max_proposals_per_part <= 2,
        "mode consolidation plan exceeds proposal ceiling"
    );
    Ok(())
}

pub fn validate_identity_namespace(
    scope: &str,
    mode: Mode,
    identity: &str,
    namespace: &str,
) -> Result<()> {
    ensure!(
        !identity.is_empty() && !identity.contains('/') && !identity.contains('\0'),
        "invalid identity namespace subject"
    );
    let prefix = format!("{scope}/{mode}/identity/{identity}");
    ensure!(
        namespace == prefix || namespace.starts_with(&format!("{prefix}/")),
        "mode namespace escapes its identity scope"
    );
    ensure!(
        !namespace
            .split('/')
            .any(|piece| piece == ".." || piece == "." || piece.is_empty()),
        "invalid mode namespace component"
    );
    Ok(())
}

pub fn validate_recipients(recipients: &[String], live: &BTreeSet<String>) -> Result<()> {
    ensure!(
        recipients.iter().all(|id| live.contains(id))
            && recipients.iter().collect::<BTreeSet<_>>().len() == recipients.len(),
        "mode selected inactive or duplicate recipients"
    );
    Ok(())
}

pub fn validate_peer_edge(sender: &str, recipient: &str, live: &BTreeSet<String>) -> Result<()> {
    ensure!(
        sender != recipient && live.contains(sender) && live.contains(recipient),
        "mode selected an invalid peer edge"
    );
    Ok(())
}

pub fn validate_relationship_members(
    relation: &Relationship,
    active_parts: &BTreeSet<String>,
) -> Result<()> {
    ensure!(
        relation.members.iter().all(|id| active_parts.contains(id)),
        "mode selected an inactive relationship member"
    );
    ensure!(
        *relation == Relationship::new(relation.kind, relation.members.clone())?,
        "mode selected a noncanonical relationship"
    );
    Ok(())
}

/// Flow may omit a contribution but cannot impersonate a sender or reveal a
/// draft to a speaking identity outside its explicit relationship membership.
pub fn validate_contributions(
    speaker: &str,
    relationship: Option<&Relationship>,
    drafts: &BTreeMap<String, String>,
    returned: &[Contribution],
) -> Result<()> {
    let mut senders = BTreeSet::new();
    for contribution in returned {
        ensure!(
            senders.insert(contribution.sender.as_str()),
            "mode repeated a contribution sender"
        );
        ensure!(
            relationship.map_or(contribution.sender == speaker, |relation| relation.id
                == speaker
                && relation.members.contains(&contribution.sender)),
            "mode shared a nonmember private contribution"
        );
        ensure!(
            drafts.get(&contribution.sender) == Some(&contribution.text),
            "mode changed a contributor's private draft"
        );
    }
    Ok(())
}

pub fn validate_facing(decision: &FacingDecision, live: &BTreeSet<String>) -> Result<()> {
    ensure!(
        live.contains(&decision.speaker) && !decision.reason.is_empty(),
        "mode selected an inactive speaking identity"
    );
    Ok(())
}

struct ReferenceRoles(Mode);
impl RolesPolicy for ReferenceRoles {
    fn seeds(&self) -> Vec<Part> {
        crate::framework::builtin_seed_parts(self.0)
    }
}

struct ReferencePeering;
impl PeeringPolicy for ReferencePeering {
    fn allows_direct(&self, sender: &str, recipient: &str, active: &BTreeSet<String>) -> bool {
        sender != recipient && active.contains(sender) && active.contains(recipient)
    }
    fn relationship(
        &self,
        origin: &RelationshipOrigin,
        kind: RelationshipKind,
        members: Vec<String>,
        active_parts: &BTreeSet<String>,
    ) -> Result<Relationship> {
        ensure!(
            members.iter().all(|id| active_parts.contains(id)),
            "relationship members must be active parts"
        );
        let relationship = Relationship::new(kind, members)?;
        if let RelationshipOrigin::Peer {
            sender,
            sender_members,
        } = origin
        {
            let participates = if sender_members.is_empty() {
                active_parts.contains(sender) && relationship.members.contains(sender)
            } else {
                sender_members
                    .iter()
                    .all(|member| active_parts.contains(member))
                    && [
                        RelationshipKind::Protection,
                        RelationshipKind::Polarization,
                        RelationshipKind::Alliance,
                    ]
                    .into_iter()
                    .any(|sender_kind| {
                        Relationship::new(sender_kind, sender_members.clone()).is_ok_and(
                            |sender_relationship| {
                                sender_relationship.id == *sender
                                    && sender_relationship.members == *sender_members
                            },
                        )
                    })
                    && sender_members
                        .iter()
                        .all(|member| relationship.members.contains(member))
            };
            ensure!(
                participates,
                "a peer can only propose a relationship it participates in"
            );
        }
        Ok(relationship)
    }
}

struct ReferenceFlow;
impl FlowPolicy for ReferenceFlow {
    fn initial_recipients(&self, target: Option<&str>, active_parts: &[String]) -> Vec<String> {
        target.map_or_else(
            || {
                let mut recipients = active_parts.to_vec();
                recipients.sort();
                recipients
            },
            |id| vec![id.to_owned()],
        )
    }
    fn next_recipients(&self, pending: &BTreeSet<String>) -> Vec<String> {
        pending.iter().cloned().collect()
    }
    fn consultation_recipients(
        &self,
        recipient: &str,
        live_identities: &BTreeSet<String>,
    ) -> Vec<String> {
        if live_identities.contains(recipient) {
            vec![recipient.to_owned()]
        } else {
            vec![]
        }
    }
    fn shared_contributions(
        &self,
        speaker: &str,
        relationship: Option<&Relationship>,
        drafts: &BTreeMap<String, String>,
    ) -> Vec<Contribution> {
        let ids: Vec<&str> = relationship.map_or_else(
            || vec![speaker],
            |relation| relation.members.iter().map(String::as_str).collect(),
        );
        ids.into_iter()
            .filter_map(|id| {
                drafts.get(id).map(|text| Contribution {
                    sender: id.to_owned(),
                    text: text.clone(),
                })
            })
            .collect()
    }
}

struct ReferenceFacing;
impl FacingPolicy for ReferenceFacing {
    fn choose(&self, input: FacingInput<'_>) -> Result<FacingDecision> {
        if let Some(target) = input.target {
            ensure!(input.live.contains(target), "caller target is inactive");
            return Ok(FacingDecision {
                speaker: target.into(),
                reason: "caller-target",
            });
        }
        if let Some((focus, remaining)) = input.focus
            && remaining > 0
            && input.live.contains(focus)
        {
            return Ok(FacingDecision {
                speaker: focus.into(),
                reason: "active-focus",
            });
        }
        let maximum = input
            .drafts
            .iter()
            .map(|id| input.activation.get(id).copied().unwrap_or(0.0))
            .max_by(f64::total_cmp)
            .ok_or_else(|| anyhow::anyhow!("no successful deliberating peers"))?;
        let tied = input
            .drafts
            .iter()
            .filter(|id| {
                input
                    .activation
                    .get(*id)
                    .copied()
                    .unwrap_or(0.0)
                    .total_cmp(&maximum)
                    .is_eq()
            })
            .collect::<Vec<_>>();
        if tied.len() > 1 {
            if let Some(previous) = input.previous_completed
                && tied.iter().any(|id| id.as_str() == previous)
            {
                return Ok(FacingDecision {
                    speaker: previous.into(),
                    reason: "previous-completed-speaker",
                });
            }
            if let Some(authored) = input.authored_order.iter().find(|id| tied.contains(id)) {
                return Ok(FacingDecision {
                    speaker: authored.clone(),
                    reason: "mode-authored-order",
                });
            }
        }
        Ok(FacingDecision {
            speaker: (*tied[0]).clone(),
            reason: if tied.len() > 1 {
                "stable-id-order"
            } else {
                "maximum-activation"
            },
        })
    }
}

struct ReferenceVisibility;
impl VisibilityPolicy for ReferenceVisibility {
    fn context_sources(&self, identity: &str, _phase: ActorPhase) -> Vec<ContextSource> {
        vec![
            ContextSource::OwnHistory(identity.into()),
            ContextSource::OwnNotes(identity.into()),
            ContextSource::PublicTranscript,
            ContextSource::ExplicitInput,
        ]
    }
    fn allows_delivery(&self, sender: &str, recipient: &str, active: &BTreeSet<String>) -> bool {
        sender != recipient && active.contains(sender) && active.contains(recipient)
    }
}

struct ReferenceMemory;
impl MemoryPolicy for ReferenceMemory {
    fn identity_namespace(&self, scope: &str, mode: Mode, identity: &str) -> String {
        format!("{scope}/{mode}/identity/{identity}")
    }
    fn transcript_namespace(&self, scope: &str, session: &str) -> String {
        format!("{scope}/transcript/{session}")
    }
    fn state_keys(&self, scope: &str, mode: Mode) -> StateKeys {
        StateKeys {
            topology: format!("{scope}/{mode}/topology"),
            dream_undo: format!("{scope}/{mode}/dream-undo"),
        }
    }
    fn consolidation_plan(&self, active_parts: &[String]) -> ConsolidationPlan {
        ConsolidationPlan {
            participants: active_parts.to_vec(),
            prompt: "Review your own history. Write a concise durable memory summary of useful facts and unresolved concerns. You may suggest a new complementary member of an existing role or retire yourself if your role is redundantly covered. A suggestion is optional; do not manufacture changes. No other tools are available during dreaming.".into(),
            phase: "dream: consolidate your own memory, optionally propose membership changes".into(),
            max_proposals_per_part: 2,
        }
    }
}
