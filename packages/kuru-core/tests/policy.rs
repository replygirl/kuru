use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use kuru_core::{
    ActorPhase, ConsolidationPlan, ContextSource, Contribution, FacingDecision, FacingInput,
    FacingPolicy, FlowPolicy, Framework, MemoryPolicy, Mode, ModeProfile, Part, PeeringPolicy,
    Relationship, RelationshipKind, RolesPolicy, StateKeys, validate_context_sources,
    validate_contributions, validate_facing, validate_identity_namespace, validate_peer_edge,
    validate_recipients, validate_relationship_members,
};
use sha2::{Digest, Sha256};

#[test]
fn reference_profiles_keep_exact_seed_bytes_and_serialized_modes() {
    let expected = [
        (
            Mode::Ifs,
            "783ce3bba630620e1060764e6fdd13683ba3c4bd83e04dabb807dc299f6582df",
        ),
        (
            Mode::Polyvagal,
            "8c308c92fa88c7993d28062f626fb3633aaa4037129ffb8124c0e26d40c6d2e9",
        ),
        (
            Mode::Freudian,
            "f85620ea0dd1fdcfedc51db22435898141a07c29397ce15cc2fb74bae137ad82",
        ),
        (
            Mode::Jungian,
            "368596914fa17d5d63ca1be53556cf40c693486a0b676ccde01b0a09dbcf78bf",
        ),
    ];
    for (mode, digest) in expected {
        let profile = ModeProfile::builtin(mode);
        profile.validate(32).unwrap();
        let seeds = profile.roles.seeds();
        assert_eq!(seeds, Framework::builtin(mode).parts);
        assert_eq!(
            profile.roles.authored_order(),
            seeds.iter().map(|part| part.id.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            profile.roles.required_roles(),
            seeds.iter().map(|part| part.role.clone()).collect()
        );
        assert_eq!(serde_json::to_string(&mode).unwrap(), format!("\"{mode}\""));
        let actual = Sha256::digest(serde_json::to_vec(&seeds).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(actual, digest, "persisted {mode} seed bytes changed");
    }
}

#[test]
fn reference_decisions_cover_peering_flow_facing_visibility_and_memory() {
    for mode in Mode::ALL {
        let profile = ModeProfile::builtin(mode);
        let seeds = profile.roles.seeds();
        let ids = seeds.iter().map(|part| part.id.clone()).collect::<Vec<_>>();
        let live = ids.iter().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            profile.flow.initial_recipients(None, &ids),
            live.iter().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            profile.flow.initial_recipients(Some(&ids[1]), &ids),
            vec![ids[1].clone()]
        );
        assert_eq!(
            profile.flow.next_recipients(&live),
            live.iter().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            profile.flow.consultation_recipients(&ids[1], &live),
            vec![ids[1].clone()]
        );
        assert!(profile.peering.allows_direct(&ids[0], &ids[1], &live));
        assert!(!profile.peering.allows_direct(&ids[0], &ids[0], &live));
        assert!(!profile.peering.allows_direct(&ids[0], "inactive", &live));
        assert!(profile.visibility.allows_delivery(&ids[0], &ids[1], &live));
        let relation = profile
            .peering
            .relationship(
                &ids[0],
                None,
                RelationshipKind::Alliance,
                ids[..2].to_vec(),
                &live,
            )
            .unwrap();
        let live_with_relationship = live
            .iter()
            .cloned()
            .chain([relation.id.clone()])
            .collect::<BTreeSet<_>>();
        assert_eq!(
            profile
                .flow
                .consultation_recipients(&relation.id, &live_with_relationship),
            vec![relation.id.clone()]
        );
        validate_relationship_members(&relation, &live).unwrap();
        assert!(
            profile
                .peering
                .relationship(
                    &ids[2],
                    None,
                    RelationshipKind::Alliance,
                    ids[..2].to_vec(),
                    &live
                )
                .is_err()
        );
        let drafts = BTreeMap::from([
            (ids[0].clone(), "zero".into()),
            (ids[1].clone(), "one".into()),
        ]);
        let shared = profile
            .flow
            .shared_contributions(&relation.id, Some(&relation), &drafts);
        validate_contributions(&relation.id, Some(&relation), &drafts, &shared).unwrap();
        assert_eq!(
            shared
                .iter()
                .map(|item| item.sender.as_str())
                .collect::<Vec<_>>(),
            relation
                .members
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        let activation = BTreeMap::new();
        let draft_ids = drafts.keys().cloned().collect();
        let authored = profile.roles.authored_order();
        let choose = |target, focus, previous| {
            profile
                .facing
                .choose(FacingInput {
                    target,
                    focus,
                    live: &live,
                    drafts: &draft_ids,
                    activation: &activation,
                    previous_completed: previous,
                    authored_order: &authored,
                })
                .unwrap()
        };
        assert_eq!(choose(Some(&ids[1]), None, None).reason, "caller-target");
        assert_eq!(
            choose(None, Some((&ids[1], 2)), None).reason,
            "active-focus"
        );
        assert_eq!(
            choose(None, None, Some(&ids[1])).reason,
            "previous-completed-speaker"
        );
        assert_eq!(
            choose(None, None, None),
            FacingDecision {
                speaker: ids[0].clone(),
                reason: "mode-authored-order"
            }
        );
        let sources = profile
            .visibility
            .context_sources(&ids[0], ActorPhase::Speak);
        validate_context_sources(&ids[0], &sources).unwrap();
        assert_eq!(
            profile
                .memory
                .identity_namespace("project/p", mode, &ids[0]),
            format!("project/p/{mode}/identity/{}", ids[0])
        );
        assert_eq!(
            profile.memory.transcript_namespace("project/p", "session"),
            "project/p/transcript/session"
        );
        let plan = profile.memory.consolidation_plan(&ids);
        assert_eq!(plan.participants, ids);
        assert_eq!(plan.max_proposals_per_part, 2);
    }
}

#[test]
fn facing_parity_covers_activation_focus_relationship_and_dream_only_tie() {
    let profile = ModeProfile::builtin(Mode::Ifs);
    let authored = profile.roles.authored_order();
    let relation = Relationship::new(RelationshipKind::Alliance, authored[..2].to_vec()).unwrap();
    let live = authored
        .iter()
        .cloned()
        .chain([relation.id.clone()])
        .collect::<BTreeSet<_>>();
    let drafts = authored[..2].iter().cloned().collect::<BTreeSet<_>>();
    let mut activation = BTreeMap::from([(authored[0].clone(), 0.4), (authored[1].clone(), 0.9)]);
    let choose =
        |target, focus, previous, drafts: &BTreeSet<String>, activation: &BTreeMap<String, f64>| {
            profile
                .facing
                .choose(FacingInput {
                    target,
                    focus,
                    live: &live,
                    drafts,
                    activation,
                    previous_completed: previous,
                    authored_order: &authored,
                })
                .unwrap()
        };
    assert_eq!(
        choose(None, None, Some(&authored[0]), &drafts, &activation),
        FacingDecision {
            speaker: authored[1].clone(),
            reason: "maximum-activation"
        }
    );
    activation.insert(authored[0].clone(), 0.9);
    assert_eq!(
        choose(None, None, Some(&authored[1]), &drafts, &activation).reason,
        "previous-completed-speaker"
    );
    assert_eq!(
        choose(None, Some((&relation.id, 1)), None, &drafts, &activation).speaker,
        relation.id
    );
    let dream_ids = ["dream-a".to_string(), "dream-z".to_string()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let dream_live = live.union(&dream_ids).cloned().collect::<BTreeSet<_>>();
    let dream = profile
        .facing
        .choose(FacingInput {
            target: None,
            focus: None,
            live: &dream_live,
            drafts: &dream_ids,
            activation: &BTreeMap::new(),
            previous_completed: None,
            authored_order: &authored,
        })
        .unwrap();
    assert_eq!(
        dream,
        FacingDecision {
            speaker: "dream-a".into(),
            reason: "stable-id-order"
        }
    );
}

struct BadRoles;
impl RolesPolicy for BadRoles {
    fn seeds(&self) -> Vec<Part> {
        let mut seeds = Framework::builtin(Mode::Ifs).parts;
        seeds[1].id = seeds[0].id.clone();
        seeds
    }
}

struct DuplicateNames;
impl RolesPolicy for DuplicateNames {
    fn seeds(&self) -> Vec<Part> {
        let mut seeds = Framework::builtin(Mode::Ifs).parts;
        seeds[1].name = seeds[0].name.to_ascii_lowercase();
        seeds
    }
}

struct OtherRoles;
impl RolesPolicy for OtherRoles {
    fn seeds(&self) -> Vec<Part> {
        Framework::builtin(Mode::Ifs).parts
    }
    fn authored_order(&self) -> Vec<String> {
        let mut order = self
            .seeds()
            .into_iter()
            .map(|part| part.id)
            .collect::<Vec<_>>();
        order.reverse();
        order
    }
}

struct OtherFacing;
impl FacingPolicy for OtherFacing {
    fn choose(&self, input: FacingInput<'_>) -> anyhow::Result<FacingDecision> {
        Ok(FacingDecision {
            speaker: input.drafts.iter().next_back().unwrap().clone(),
            reason: "test-alternative",
        })
    }
}

struct OtherFlow;
impl FlowPolicy for OtherFlow {
    fn initial_recipients(&self, _: Option<&str>, active: &[String]) -> Vec<String> {
        active.last().cloned().into_iter().collect()
    }
    fn next_recipients(&self, pending: &BTreeSet<String>) -> Vec<String> {
        pending.iter().next_back().cloned().into_iter().collect()
    }
    fn consultation_recipients(&self, _: &str, _: &BTreeSet<String>) -> Vec<String> {
        vec![]
    }
    fn shared_contributions(
        &self,
        _: &str,
        _: Option<&Relationship>,
        _: &BTreeMap<String, String>,
    ) -> Vec<Contribution> {
        vec![]
    }
}

struct OtherPeering;
impl PeeringPolicy for OtherPeering {
    fn allows_direct(&self, _: &str, _: &str, _: &BTreeSet<String>) -> bool {
        false
    }
    fn relationship(
        &self,
        _: &str,
        _: Option<&[String]>,
        _: RelationshipKind,
        _: Vec<String>,
        _: &BTreeSet<String>,
    ) -> anyhow::Result<Relationship> {
        anyhow::bail!("test relationship denied")
    }
}

struct OtherVisibility;
impl kuru_core::VisibilityPolicy for OtherVisibility {
    fn context_sources(&self, identity: &str, _: ActorPhase) -> Vec<ContextSource> {
        vec![ContextSource::OwnHistory(identity.into())]
    }
    fn allows_delivery(&self, _: &str, _: &str, _: &BTreeSet<String>) -> bool {
        false
    }
}

struct OtherMemory;
impl MemoryPolicy for OtherMemory {
    fn identity_namespace(&self, scope: &str, mode: Mode, identity: &str) -> String {
        format!("{scope}/{mode}/identity/{identity}/other")
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
    fn consolidation_plan(&self, active: &[String]) -> ConsolidationPlan {
        ModeProfile::builtin(Mode::Ifs)
            .memory
            .consolidation_plan(active)
    }
}

#[test]
fn components_can_be_replaced_independently_and_invalid_results_refuse() {
    let original = ModeProfile::builtin(Mode::Ifs);
    let ids = original.roles.authored_order();
    let live = ids.iter().cloned().collect::<BTreeSet<_>>();
    let drafts = live.clone();
    let mut changed = original.clone();
    changed.roles = Arc::new(OtherRoles);
    assert_eq!(changed.roles.authored_order().first(), ids.last());
    changed.validate(32).unwrap();
    changed = original.clone();
    changed.flow = Arc::new(OtherFlow);
    assert_eq!(
        changed.flow.initial_recipients(None, &ids),
        vec![ids.last().unwrap().clone()]
    );
    assert_eq!(changed.roles.seeds(), original.roles.seeds());
    changed = original.clone();
    changed.peering = Arc::new(OtherPeering);
    assert!(!changed.peering.allows_direct(&ids[0], &ids[1], &live));
    changed = original.clone();
    changed.facing = Arc::new(OtherFacing);
    assert_eq!(
        changed
            .facing
            .choose(FacingInput {
                target: None,
                focus: None,
                live: &live,
                drafts: &drafts,
                activation: &BTreeMap::new(),
                previous_completed: None,
                authored_order: &ids
            })
            .unwrap()
            .speaker,
        ids.iter().max().unwrap().as_str()
    );
    changed = original.clone();
    changed.visibility = Arc::new(OtherVisibility);
    assert_eq!(
        changed
            .visibility
            .context_sources(&ids[0], ActorPhase::Speak),
        vec![ContextSource::OwnHistory(ids[0].clone())]
    );
    changed.validate(32).unwrap();
    changed = original.clone();
    changed.memory = Arc::new(OtherMemory);
    assert!(
        changed
            .memory
            .identity_namespace("project/p", Mode::Ifs, &ids[0])
            .ends_with("/other")
    );
    changed.validate(32).unwrap();
    changed = original.clone();
    changed.roles = Arc::new(BadRoles);
    assert!(changed.validate(32).is_err());
    changed.roles = Arc::new(DuplicateNames);
    assert!(changed.validate(32).is_err());
    assert!(
        validate_context_sources(&ids[0], &[ContextSource::OwnHistory(ids[1].clone())]).is_err()
    );
    assert!(
        validate_identity_namespace(
            "project/p",
            Mode::Ifs,
            &ids[0],
            "project/p/ifs/identity/other"
        )
        .is_err()
    );
    assert!(validate_recipients(&["inactive".into()], &live).is_err());
    assert!(validate_peer_edge(&ids[0], &ids[0], &live).is_err());
    assert!(validate_peer_edge(&ids[0], "inactive", &live).is_err());
    assert!(
        validate_facing(
            &FacingDecision {
                speaker: "inactive".into(),
                reason: "test"
            },
            &live
        )
        .is_err()
    );
    let drafts = BTreeMap::from([
        (ids[0].clone(), "private".into()),
        (ids[1].clone(), "other".into()),
    ]);
    assert!(
        validate_contributions(
            &ids[0],
            None,
            &drafts,
            &[Contribution {
                sender: ids[1].clone(),
                text: "other".into()
            },]
        )
        .is_err()
    );
    assert!(
        validate_contributions(
            &ids[0],
            None,
            &drafts,
            &[Contribution {
                sender: ids[0].clone(),
                text: "forged".into()
            },]
        )
        .is_err()
    );
}
