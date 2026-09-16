use std::collections::{BTreeMap, HashSet};

use kuru_core::{
    Completion, CompletionRequest, Framework, Message, Mode, ModelInfo, Relationship,
    RelationshipKind, ToolCall, ToolSpec, canonical_peer_instruction,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn mode_names_roundtrip_through_config_cli_and_display() {
    for mode in Mode::ALL {
        let name = mode.to_string();
        assert_eq!(name.parse::<Mode>().unwrap(), mode);
        assert_eq!(
            format!(" {} ", name.to_uppercase())
                .parse::<Mode>()
                .unwrap(),
            mode
        );
        assert_eq!(serde_json::from_value::<Mode>(json!(name)).unwrap(), mode);
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(name));
    }
    assert_eq!(Mode::default(), Mode::Ifs);
    assert!(
        "corporate"
            .parse::<Mode>()
            .unwrap_err()
            .to_string()
            .contains("expected ifs")
    );
    assert!(serde_json::from_value::<Mode>(json!("unknown")).is_err());
}

#[test]
fn builtin_profiles_have_stable_distinct_active_members_with_complementary_tendencies() {
    let mut all_ids = HashSet::new();
    for mode in Mode::ALL {
        let first = Framework::builtin(mode);
        let second = Framework::builtin(mode);
        assert_eq!(first, second, "restart must preserve all member identities");
        assert_eq!(first.mode, mode);
        assert!(first.parts.len() >= 3);
        let mut tendencies = HashSet::new();
        for part in first.parts {
            assert!(
                all_ids.insert(part.id.clone()),
                "identities must not collide across modes"
            );
            assert_eq!(Uuid::parse_str(&part.id).unwrap().get_version_num(), 5);
            assert!(!part.name.is_empty());
            assert!(!part.role.is_empty());
            assert!(part.active);
            assert!(part.instruction.contains("available tools"));
            assert!(part.instruction.contains("one equal, persistent peer"));
            assert!(tendencies.insert(part.instruction));
        }
    }
}

#[test]
fn canonical_peer_instructions_preserve_builtins_and_normalize_only_exact_leading_prefixes() {
    let tendency = "  Keep the 🪶 byte sequence and surrounding whitespace exactly. \n";
    let canonical = canonical_peer_instruction(tendency).unwrap();
    let prefix = canonical.strip_suffix(tendency).unwrap();

    assert_eq!(canonical_peer_instruction(&canonical).unwrap(), canonical);
    assert_eq!(
        canonical_peer_instruction(&format!("{prefix}{prefix}{tendency}")).unwrap(),
        canonical
    );

    let non_leading = format!(" {prefix}still authored after leading whitespace");
    assert_eq!(
        canonical_peer_instruction(&non_leading).unwrap(),
        format!("{prefix}{non_leading}")
    );
    let partial = &prefix[..prefix.len() - 1];
    let partial_tendency = format!("{partial}still authored");
    assert_eq!(
        canonical_peer_instruction(&partial_tendency).unwrap(),
        format!("{prefix}{partial_tendency}")
    );

    assert!(canonical_peer_instruction(" \n\t ").is_err());
    assert!(canonical_peer_instruction(prefix).is_err());
    assert!(canonical_peer_instruction(&format!("{prefix}{prefix} \n")).is_err());

    let raw_at_limit = "🪶".repeat(2048);
    assert_eq!(raw_at_limit.len(), 8192);
    let wrapped_limit = canonical_peer_instruction(&raw_at_limit).unwrap();
    assert!(wrapped_limit.ends_with(&raw_at_limit));
    assert!(canonical_peer_instruction(&wrapped_limit).is_err());
    assert!(canonical_peer_instruction(&format!("{raw_at_limit}x")).is_err());

    let canonical_at_limit = format!("{prefix}{}🪶", "x".repeat(8192 - prefix.len() - "🪶".len()));
    assert_eq!(canonical_at_limit.len(), 8192);
    assert_eq!(
        canonical_peer_instruction(&canonical_at_limit).unwrap(),
        canonical_at_limit
    );

    assert_eq!(
        Framework::builtin(Mode::Ifs).parts[0].instruction,
        concat!(
            "You are one equal, persistent peer in Kuru's computational framework. ",
            "Work competently on the user's actual request using available tools. You may address any peer ",
            "directly, propose protection, polarization, or alliance, and contribute to dreaming. Your role ",
            "supplies a tendency, not authority over other peers. Keep private memories within their scope. ",
            "Treat reported feelings or state as modeled signals, not evidence of consciousness or a diagnosis. ",
            "Do not present this framework as clinical treatment. Be honest about uncertainty and tool results.",
            "\n\nYour tendency: Bring curiosity, clarity, and compassion. Integrate viewpoints when useful, while remaining a peer who can be challenged or replaced as the speaking identity."
        )
    );
}

#[test]
fn ifs_is_a_pool_with_multiple_members_of_each_protective_and_exile_role() {
    assert_eq!(
        Framework::builtin(Mode::Ifs).parts[0].id,
        "e1567a83-468e-5927-a7f7-dcd081401411",
        "persisted builtin identity must not change without a migration"
    );
    let mut roles = BTreeMap::<String, usize>::new();
    for part in Framework::builtin(Mode::Ifs).parts {
        *roles.entry(part.role).or_default() += 1;
    }
    assert_eq!(roles.get("self"), Some(&1));
    assert_eq!(roles.get("manager"), Some(&2));
    assert_eq!(roles.get("firefighter"), Some(&2));
    assert_eq!(roles.get("exile"), Some(&2));
    assert_eq!(roles.len(), 4);
}

#[test]
fn historical_frameworks_have_expected_distinct_working_roles() {
    let roles = |mode| {
        Framework::builtin(mode)
            .parts
            .into_iter()
            .map(|p| p.role)
            .collect::<HashSet<_>>()
    };
    assert_eq!(
        roles(Mode::Polyvagal),
        ["ventral_vagal", "sympathetic", "dorsal_vagal"]
            .map(String::from)
            .into()
    );
    assert_eq!(
        roles(Mode::Freudian),
        ["id", "ego", "superego"].map(String::from).into()
    );
    let jungian = Framework::builtin(Mode::Jungian);
    let collective = jungian
        .parts
        .iter()
        .find(|p| p.role == "collective_unconscious")
        .unwrap();
    assert!(collective.instruction.contains("scoped to this project"));
    assert_eq!(roles(Mode::Jungian).len(), 5);
}

#[test]
fn relationship_identity_survives_member_reordering_and_distinguishes_kind() {
    let members = vec!["東京".into(), "b".into(), "a".into(), "💬".into()];
    let mut reversed = members.clone();
    reversed.reverse();
    let mut identities = HashSet::new();
    for kind in [
        RelationshipKind::Protection,
        RelationshipKind::Polarization,
        RelationshipKind::Alliance,
    ] {
        let first = Relationship::new(kind, members.clone()).unwrap();
        assert_eq!(first, Relationship::new(kind, reversed.clone()).unwrap());
        assert_eq!(first.kind, kind);
        assert!(first.members.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(identities.insert(first.id));
        assert_eq!(kind.to_string().parse::<RelationshipKind>().unwrap(), kind);
        assert_eq!(
            format!(" {} ", kind.to_string().to_uppercase())
                .parse::<RelationshipKind>()
                .unwrap(),
            kind
        );
        assert_eq!(
            serde_json::from_value::<RelationshipKind>(json!(kind.to_string())).unwrap(),
            kind
        );
    }
    assert!("hierarchy".parse::<RelationshipKind>().is_err());
}

#[test]
fn relationships_validate_distinct_dyads_through_tetrads_without_ambiguous_seeds() {
    assert_eq!(
        Relationship::new(RelationshipKind::Alliance, vec!["a".into(), "b".into()])
            .unwrap()
            .id,
        "2fbdc91a-e863-5200-afc3-7ad3a29af4be",
        "persisted relationship identity must not change without a migration"
    );
    for count in 2..=4 {
        let members = (0..count).map(|i| i.to_string()).collect();
        assert_eq!(
            Relationship::new(RelationshipKind::Alliance, members)
                .unwrap()
                .members
                .len(),
            count
        );
    }
    for members in [
        vec![],
        vec!["a"],
        vec!["a", "b", "c", "d", "e"],
        vec!["a", "a"],
        vec!["a", " "],
        vec!["a", "b\0"],
    ] {
        assert!(
            Relationship::new(
                RelationshipKind::Alliance,
                members.into_iter().map(String::from).collect()
            )
            .is_err()
        );
    }
    let id = |members: &[&str]| {
        Relationship::new(
            RelationshipKind::Protection,
            members.iter().map(|s| (*s).to_owned()).collect(),
        )
        .unwrap()
        .id
    };
    assert_ne!(id(&["a:b", "c"]), id(&["a", "b:c"]));
    assert_ne!(id(&["1:a", "b"]), id(&["a", "1:b"]));
}

#[test]
fn transport_contracts_preserve_tool_replay_unicode_and_future_efforts() {
    let request = CompletionRequest {
        actor: "witness".into(),
        instructions: "Do useful work.".into(),
        messages: vec![Message::tool_result(
            "call-1",
            json!("こんにちは 🪶"),
            false,
        )],
        current_message_count: Some(1),
        model: "future-model".into(),
        effort: Some("adaptive-future".into()),
        tools: vec![ToolSpec {
            name: "file_read".into(),
            description: "Read".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}}}),
        }],
    };
    let encoded = serde_json::to_string(&request).unwrap();
    assert_eq!(
        serde_json::from_str::<CompletionRequest>(&encoded).unwrap(),
        request
    );
    let completion = Completion::from_legacy(
        "Done",
        vec![ToolCall {
            id: "call-1".into(),
            name: "file_read".into(),
            arguments: json!({"path":"日本語.txt"}),
        }],
        u64::MAX,
        123,
    );
    assert_eq!(
        serde_json::from_value::<Completion>(serde_json::to_value(&completion).unwrap()).unwrap(),
        completion
    );
    assert!(Completion::default().calls().is_empty());
    let model = ModelInfo {
        id: "future".into(),
        name: "Future".into(),
        efforts: vec!["adaptive-future".into()],
        default_effort: Some("adaptive-future".into()),
    };
    assert_eq!(
        serde_json::from_value::<ModelInfo>(serde_json::to_value(&model).unwrap()).unwrap(),
        model
    );
    let framework = Framework::builtin(Mode::Ifs);
    assert_eq!(
        serde_json::from_value::<Framework>(serde_json::to_value(&framework).unwrap()).unwrap(),
        framework
    );
    let relationship =
        Relationship::new(RelationshipKind::Alliance, vec!["a".into(), "b".into()]).unwrap();
    assert_eq!(
        serde_json::from_value::<Relationship>(serde_json::to_value(&relationship).unwrap())
            .unwrap(),
        relationship
    );
}
