use super::*;
use kuru_connectors::DemoProvider;

fn config() -> Config {
    Config {
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}

#[tokio::test]
async fn complete_topology_load_preserves_large_extra_inventory_and_warm_member_reports()
-> Result<()> {
    kuru_memory::test_support::closing(async {
        let memory = MemoryStore::temporary().await?;
        let scope = "project/complete-topology";
        let profile = ModeProfile::builtin(Mode::Ifs);
        let keys = checked_state_keys(scope, &profile)?;
        let mut topology = Topology {
            parts: profile.roles.seeds(),
            relationships: vec![],
            states: BTreeMap::new(),
            focus: None,
        };
        let mut archived = topology.parts[0].clone();
        archived.id = "archived-part".into();
        archived.active = false;
        topology.parts.push(archived);
        memory
            .put(
                &keys.membership,
                &serde_json::to_value(MembershipRecord::from_topology(&topology))?,
            )
            .await?;
        let mut identities = (0..300)
            .map(|index| format!("extra/{index}"))
            .collect::<Vec<_>>();
        identities.extend([
            "archived-part".into(),
            "legacy/".to_owned() + &"é".repeat(1200),
        ]);
        let report = StateReport {
            activation: 0.25,
            note: "retained".into(),
        };
        for chunk in identities.chunks(100) {
            let rows = chunk
                .iter()
                .map(|id| {
                    Ok((
                        keys.state_report(id),
                        topology_state::report_value(id, &report)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            memory.put_many(&rows).await?;
        }
        let first = topology_state::load(&memory, &keys, &profile, None, None).await?;
        assert_eq!(first.topology.states.len(), identities.len());
        assert_eq!(first.topology.parts.len(), topology.parts.len());
        for identity in &identities {
            assert_eq!(first.topology.states[identity].note, "retained");
        }
        let warm =
            topology_state::load(&memory, &keys, &profile, None, Some(&first.inventory)).await?;
        assert_eq!(
            serde_json::to_value(&first.topology)?,
            serde_json::to_value(&warm.topology)?
        );
        assert_eq!(first.expectation, warm.expectation);

        let small_scope = "project/small-topology";
        let small_keys = checked_state_keys(small_scope, &profile)?;
        memory
            .put(
                &small_keys.membership,
                &serde_json::to_value(MembershipRecord::from_topology(&topology))?,
            )
            .await?;
        let before = topology_state::load(&memory, &small_keys, &profile, None, None).await?;
        let new_identity = &topology.parts[0].id;
        memory
            .put(
                &small_keys.state_report(new_identity),
                &topology_state::report_value(new_identity, &report)?,
            )
            .await?;
        let fast = topology_state::load(
            &memory,
            &small_keys,
            &profile,
            None,
            Some(&before.inventory),
        )
        .await?;
        let cut = topology_state::load(&memory, &small_keys, &profile, None, None).await?;
        assert_eq!(fast.topology.states[new_identity].note, "retained");
        assert_eq!(
            serde_json::to_value(&fast.topology)?,
            serde_json::to_value(&cut.topology)?
        );
        // A small identity inventory can still exceed the fast getter's byte
        // bound. Full inspection must fall back to the cut without truncation.
        let large = StateReport {
            activation: 0.5,
            note: "x".repeat(16 * 1024 * 1024 + 1024),
        };
        memory
            .put(
                &small_keys.state_report(new_identity),
                &topology_state::report_value(new_identity, &large)?,
            )
            .await?;
        let byte_fallback =
            topology_state::load(&memory, &small_keys, &profile, None, Some(&fast.inventory))
                .await?;
        assert_eq!(byte_fallback.topology.states[new_identity].note, large.note);
        assert_eq!(byte_fallback.expectation, fast.expectation);
        memory.close().await
    })
    .await
}

#[tokio::test]
async fn session_report_and_stale_membership_publications_preserve_other_owned_rows() -> Result<()>
{
    kuru_memory::test_support::closing(async {
        let project = tempfile::tempdir()?;
        let memory = MemoryStore::temporary().await?;
        let mut first = Harness::new(
            config(),
            project.path(),
            memory.clone(),
            Arc::new(DemoProvider),
            None,
        )
        .await?;
        let mut second = Harness::new(
            config(),
            project.path(),
            memory.clone(),
            Arc::new(DemoProvider),
            None,
        )
        .await?;
        let keys = checked_state_keys(&first.scope, &first.profile)?;
        let legacy = json!({"frozen_legacy_bytes": "never rewritten"});
        memory.put(&keys.topology, &legacy).await?;
        let initial_membership = memory.get_versioned(&keys.membership).await?.unwrap();
        let first_id = first.topology.parts[0].id.clone();
        let second_id = first.topology.parts[1].id.clone();
        first.focus(Some(&first_id)).await?;
        second.focus(Some(&second_id)).await?;
        let second_session_key = format!("{}/session/{}", second.scope, second.session.id);
        let second_session = memory.get_versioned(&second_session_key).await?.unwrap();
        let report = StateReport {
            activation: 0.9,
            note: "first report".into(),
        };
        let mut projected = first.topology.clone();
        projected.states.insert(first_id.clone(), report.clone());
        first.persist_report(&first_id, projected, &report).await?;
        first
            .set_model("changed-model", Some("high".into()))
            .await?;
        assert_eq!(
            memory.get_versioned(&second_session_key).await?,
            Some(second_session.clone())
        );
        assert_eq!(
            memory.get_versioned(&keys.membership).await?,
            Some(initial_membership.clone())
        );
        assert_eq!(memory.get(&keys.topology).await?, Some(legacy.clone()));
        assert_eq!(first.topology.focus.as_ref().unwrap().id, first_id);

        let later = StateReport {
            activation: 0.5,
            note: "same identity last report".into(),
        };
        let mut projected = second.topology.clone();
        projected.states.insert(first_id.clone(), later.clone());
        second.persist_report(&first_id, projected, &later).await?;
        let other = StateReport {
            activation: 0.1,
            note: "other identity".into(),
        };
        let mut projected = second.topology.clone();
        projected.states.insert(second_id.clone(), other.clone());
        second.persist_report(&second_id, projected, &other).await?;
        let relation = second
            .relate(
                RelationshipKind::Alliance,
                vec![first_id.clone(), second_id.clone()],
            )
            .await?;
        let winner = memory.get_versioned(&keys.membership).await?.unwrap();
        let before = memory.revision().await?;
        let refusal = first
            .persist_membership(
                first.topology.clone(),
                StateExpectation::Version(initial_membership.version),
                vec![],
                true,
                None,
            )
            .await
            .unwrap_err();
        assert!(refusal.is::<StateStale>());
        assert!(first.pending_publication.is_none());
        assert_eq!(memory.revision().await?, before);
        assert_eq!(memory.get_versioned(&keys.membership).await?, Some(winner));
        let loaded = topology_state::load(
            &memory,
            &keys,
            &first.profile,
            Some(&format!("{}/session/{}", first.scope, first.session.id)),
            None,
        )
        .await?;
        assert!(
            loaded
                .topology
                .relationships
                .iter()
                .any(|item| item.id == relation.id)
        );
        assert_eq!(
            loaded.topology.states[&first_id].note,
            "same identity last report"
        );
        assert_eq!(loaded.topology.states[&second_id].note, "other identity");
        assert_eq!(loaded.topology.focus.as_ref().unwrap().id, first_id);
        assert_eq!(memory.get(&keys.topology).await?, Some(legacy));
        first.shutdown(false).await?;
        second.shutdown(false).await?;
        memory.close().await
    })
    .await
}

#[test]
fn report_codec_preserves_exact_identities_and_refuses_mismatched_envelopes() -> Result<()> {
    let keys = ModeProfile::builtin(Mode::Ifs)
        .memory
        .state_keys("project/codec", Mode::Ifs);
    let report = StateReport {
        activation: 0.5,
        note: "exact".into(),
    };
    for identity in ["K", "k", "e", "é", "extra/identity", &"long".repeat(1200)] {
        let key = keys.state_report(identity);
        assert_eq!(key.len(), keys.state_prefix.len() + 64);
        let (actual, _) = topology_state::decode_report(
            &keys,
            &key,
            topology_state::report_value(identity, &report)?,
        )?;
        assert_eq!(actual, identity);
    }
    assert_ne!(keys.state_report("K"), keys.state_report("k"));
    assert!(
        topology_state::decode_report(
            &keys,
            &keys.state_report("K"),
            topology_state::report_value("k", &report)?
        )
        .is_err()
    );
    assert!(
        MembershipRecord::decode(
            json!({"record_format":"future","parts":[],"relationships":[]}),
            true
        )
        .is_err()
    );
    assert!(
        MembershipRecord::decode(
            json!({"parts":[],"relationships":[],"states":{},"focus":null}),
            true
        )
        .is_ok()
    );
    Ok(())
}

#[test]
fn retired_focus_cannot_resolve_through_an_active_name_alias() {
    let profile = ModeProfile::builtin(Mode::Ifs);
    let mut parts = profile.roles.seeds();
    let retired_id = parts[0].id.clone();
    parts[0].active = false;
    parts[1].name = retired_id.clone();
    let mut topology = Topology {
        parts,
        relationships: vec![],
        states: BTreeMap::new(),
        focus: Some(Focus {
            id: retired_id,
            remaining: 3,
        }),
    };
    topology_state::clear_inactive_focus(&mut topology);
    assert!(topology.focus.is_none());
}

#[tokio::test]
async fn admitted_relationship_delta_and_dropped_turn_refresh_preserve_ownership() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let project = tempfile::tempdir()?;
        let memory = MemoryStore::temporary().await?;
        let mut admitted = Harness::new(
            config(),
            project.path(),
            memory.clone(),
            Arc::new(DemoProvider),
            None,
        )
        .await?;
        let mut other = Harness::new(
            config(),
            project.path(),
            memory.clone(),
            Arc::new(DemoProvider),
            None,
        )
        .await?;
        let members = admitted
            .topology
            .parts
            .iter()
            .take(2)
            .map(|part| part.id.clone())
            .collect::<Vec<_>>();
        admitted.active_turn.store(true, Ordering::Release);
        let unrelated = other
            .relate(RelationshipKind::Polarization, members.clone())
            .await?;
        let owned = other
            .relate(RelationshipKind::Alliance, members.clone())
            .await?;
        let membership_key = checked_state_keys(&admitted.scope, &admitted.profile)?.membership;
        let before = memory.get_versioned(&membership_key).await?;
        // The relation already exists durably. Adopting its own delta must not
        // rewrite membership or admit the unrelated relation into this turn.
        admitted.relate(RelationshipKind::Alliance, members).await?;
        assert_eq!(memory.get_versioned(&membership_key).await?, before);
        assert_eq!(admitted.topology.focus.as_ref().unwrap().id, owned.id);
        assert!(
            admitted
                .topology
                .relationships
                .iter()
                .any(|relation| relation.id == owned.id)
        );
        assert!(
            !admitted
                .topology
                .relationships
                .iter()
                .any(|relation| relation.id == unrelated.id)
        );

        let guard = TurnDropGuard {
            slot: admitted.aborted_turn.clone(),
            active: admitted.active_turn.clone(),
            refresh: admitted.deferred_topology_refresh.clone(),
            turn: None,
        };
        drop(guard);
        assert!(!admitted.active_turn.load(Ordering::Acquire));
        assert!(admitted.deferred_topology_refresh.load(Ordering::Acquire));
        admitted.set_model("after-cancel", None).await?;
        assert!(
            admitted
                .topology
                .relationships
                .iter()
                .any(|relation| relation.id == unrelated.id)
        );
        assert!(!admitted.deferred_topology_refresh.load(Ordering::Acquire));
        admitted.shutdown(false).await?;
        other.shutdown(false).await?;
        memory.close().await
    })
    .await
}

#[tokio::test]
async fn dream_and_legacy_undo_preserve_current_reports_and_session_focus() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let project = tempfile::tempdir()?;
        let memory = MemoryStore::temporary().await?;
        let mut harness = Harness::new(
            config(),
            project.path(),
            memory.clone(),
            Arc::new(DemoProvider),
            None,
        )
        .await?;
        let keys = checked_state_keys(&harness.scope, &harness.profile)?;
        let identity = harness.topology.parts[0].id.clone();
        harness.focus(Some(&identity)).await?;
        let mut historical = harness.topology.clone();
        historical.states.insert(
            identity.clone(),
            StateReport {
                activation: 0.1,
                note: "obsolete undo report".into(),
            },
        );
        historical.focus = None;
        let report = StateReport {
            activation: 0.8,
            note: "current live report".into(),
        };
        let mut projected = harness.topology.clone();
        projected.states.insert(identity.clone(), report.clone());
        harness
            .persist_report(&identity, projected, &report)
            .await?;
        let session_key = format!("{}/session/{}", harness.scope, harness.session.id);
        let session_before = memory.get_versioned(&session_key).await?;
        let report_before = memory.get_versioned(&keys.state_report(&identity)).await?;
        let role = harness.topology.parts[0].role.clone();
        harness
            .apply_dream(vec![crate::DreamProposal::Add {
                name: "Split-state observer".into(),
                role,
                instruction: "Notice overlooked details.".into(),
            }])
            .await?;
        let added = harness.resolve("Split-state observer")?;
        assert_eq!(memory.get_versioned(&session_key).await?, session_before);
        assert_eq!(
            memory.get_versioned(&keys.state_report(&identity)).await?,
            report_before
        );
        let undo = memory.get(&keys.dream_undo).await?.unwrap();
        assert!(undo.get("states").is_none());
        assert!(undo.get("focus").is_none());
        // Previously released undo payloads contained the complete topology.
        // Their stale report/focus fields must not replace today's owned rows.
        memory
            .put(&keys.dream_undo, &serde_json::to_value(historical)?)
            .await?;
        harness.undo_dream().await?;
        assert_eq!(memory.get_versioned(&session_key).await?, session_before);
        assert_eq!(
            memory.get_versioned(&keys.state_report(&identity)).await?,
            report_before
        );
        assert_eq!(
            harness.topology.states[&identity].note,
            "current live report"
        );
        assert_eq!(harness.topology.focus.as_ref().unwrap().id, identity);
        assert!(
            !harness
                .topology
                .parts
                .iter()
                .find(|part| part.id == added)
                .unwrap()
                .active
        );
        harness.shutdown(false).await?;
        memory.close().await
    })
    .await
}

#[tokio::test]
async fn retirement_clears_own_projection_then_own_checkpoint_without_writing_other_focus()
-> Result<()> {
    kuru_memory::test_support::closing(async {
        let project = tempfile::tempdir()?;
        let memory = MemoryStore::temporary().await?;
        let mut first = Harness::new(
            config(),
            project.path(),
            memory.clone(),
            Arc::new(DemoProvider),
            None,
        )
        .await?;
        let role = first.topology.parts[0].role.clone();
        first
            .apply_dream(vec![crate::DreamProposal::Add {
                name: "Retirable observer".into(),
                role,
                instruction: "Keep a distinct perspective.".into(),
            }])
            .await?;
        let retired = first.resolve("Retirable observer")?;
        first.focus(Some(&retired)).await?;
        let mut second = Harness::new(
            config(),
            project.path(),
            memory.clone(),
            Arc::new(DemoProvider),
            None,
        )
        .await?;
        let second_id = second.topology.parts[0].id.clone();
        second.focus(Some(&second_id)).await?;
        let own_key = format!("{}/session/{}", first.scope, first.session.id);
        let other_key = format!("{}/session/{}", second.scope, second.session.id);
        let own_before = memory.get_versioned(&own_key).await?;
        let other_before = memory.get_versioned(&other_key).await?;
        second
            .apply_dream(vec![crate::DreamProposal::Retire {
                id: retired.clone(),
            }])
            .await?;
        let before_refusal = memory.revision().await?;
        // First has an idle stale projection. Its focus command must observe
        // the shared retirement before resolving/staging its own session row.
        assert!(first.focus(Some(&retired)).await.is_err());
        assert_eq!(memory.revision().await?, before_refusal);
        assert!(first.topology.focus.is_none());
        assert_eq!(memory.get_versioned(&own_key).await?, own_before);
        assert_eq!(memory.get_versioned(&other_key).await?, other_before);
        first.set_model("after-retirement", None).await?;
        let saved: Session = serde_json::from_value(memory.get(&own_key).await?.unwrap())?;
        assert!(saved.focus.is_none());
        assert_eq!(memory.get_versioned(&other_key).await?, other_before);
        first.shutdown(false).await?;
        second.shutdown(false).await?;
        memory.close().await
    })
    .await
}
