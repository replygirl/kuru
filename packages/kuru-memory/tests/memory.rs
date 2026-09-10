use kuru_core::Message;
use kuru_memory::MemoryStore;
use serde_json::{Value, json};

#[tokio::test]
async fn private_namespaces_order_and_literal_values_survive() {
    let store = MemoryStore::temporary().await.unwrap();
    for index in 0..6 {
        store
            .append("part/one", "user", &format!("one-{index}"))
            .await
            .unwrap();
        store
            .append("part/two", "assistant", &format!("two-{index}"))
            .await
            .unwrap();
    }
    let expected = (3..6)
        .map(|index| Message {
            role: "user".into(),
            content: format!("one-{index}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(store.history("part/one", 3).await.unwrap(), expected);
    assert_eq!(store.history("part/two", 100).await.unwrap().len(), 6);
    assert!(store.history("missing", 100).await.unwrap().is_empty());
    assert!(store.history("part/one", 0).await.unwrap().is_empty());
    let content = "Привет 東京 🫧 e\u{301} é\0\n";
    store.append("部品/記憶", "tool", content).await.unwrap();
    store
        .append("部品/記憶", "future_protocol_role", "")
        .await
        .unwrap();
    assert_eq!(
        store.history("部品/記憶", 2).await.unwrap(),
        [
            Message {
                role: "tool".into(),
                content: content.into()
            },
            Message {
                role: "future_protocol_role".into(),
                content: "".into()
            }
        ]
    );
    let injection = "'; DELETE FROM messages; --";
    store
        .append(injection, "assistant", "literal")
        .await
        .unwrap();
    store
        .put(injection, &json!({"text":content,"nested":[1,true,null]}))
        .await
        .unwrap();
    assert_eq!(
        store.get(injection).await.unwrap(),
        Some(json!({"text":content,"nested":[1,true,null]}))
    );
    assert_eq!(
        store.history(injection, 10).await.unwrap()[0].content,
        "literal"
    );
    store.clear(injection).await.unwrap();
    assert_eq!(store.history("part/one", 10).await.unwrap().len(), 6);
    store.put("Case", &json!(1)).await.unwrap();
    store.put("case", &json!(2)).await.unwrap();
    assert_eq!(store.get("Case").await.unwrap(), Some(json!(1)));
    assert_eq!(store.get("case").await.unwrap(), Some(json!(2)));
    store.close().await.unwrap();
}

#[tokio::test]
async fn clear_state_upserts_and_validation_preserve_unrelated_data() {
    let store = MemoryStore::temporary().await.unwrap();
    for namespace in ["part/a", "part/b", "group/a-b", "session/a"] {
        store.append(namespace, "user", namespace).await.unwrap();
    }
    store
        .put("part/a", &json!({"archived":true}))
        .await
        .unwrap();
    store.clear("part/a").await.unwrap();
    store.clear("missing").await.unwrap();
    assert!(store.history("part/a", 10).await.unwrap().is_empty());
    for namespace in ["part/b", "group/a-b", "session/a"] {
        assert_eq!(
            store.history(namespace, 10).await.unwrap()[0].content,
            namespace
        );
    }
    assert_eq!(
        store.get("part/a").await.unwrap(),
        Some(json!({"archived":true}))
    );
    store.put("key", &json!([1, 2, 3])).await.unwrap();
    store.put("key", &Value::Null).await.unwrap();
    assert_eq!(store.get("key").await.unwrap(), Some(Value::Null));
    assert_eq!(store.get("other").await.unwrap(), None);
    for invalid in ["".into(), " ".into(), "nul\0id".into(), "x".repeat(1025)] {
        assert!(store.append(&invalid, "user", "body").await.is_err());
        assert!(store.history(&invalid, 1).await.is_err());
        assert!(store.clear(&invalid).await.is_err());
        assert!(store.put(&invalid, &Value::Null).await.is_err());
        assert!(store.get(&invalid).await.is_err());
    }
    for invalid in ["".into(), " ".into(), "tool\0".into(), "x".repeat(129)] {
        assert!(store.append("valid", &invalid, "body").await.is_err());
    }
    assert!(store.history("valid", usize::MAX).await.is_err());
    assert!(store.revisions(usize::MAX).await.is_err());
    store
        .put_many(&[("one".into(), json!(1)), ("two".into(), json!(2))])
        .await
        .unwrap();
    assert!(
        store
            .put_many(&[("one".into(), json!(10)), ("".into(), Value::Null)])
            .await
            .is_err()
    );
    assert!(
        store
            .put_many(&[("one".into(), json!(10)), ("one".into(), json!(20))])
            .await
            .is_err()
    );
    assert_eq!(store.get("one").await.unwrap(), Some(json!(1)));
    assert_eq!(store.get("two").await.unwrap(), Some(json!(2)));
    store.put_many(&[]).await.unwrap();
    store.close().await.unwrap();
    assert!(store.put("closed", &json!(1)).await.is_err());
}

#[tokio::test]
async fn candidates_remain_private_and_stale_promotion_preserves_both_histories() {
    let store = MemoryStore::temporary().await.unwrap();
    store.put("topology", &json!("before")).await.unwrap();
    let base = store.revision().await.unwrap();
    let candidate = store.begin_candidate("dream").await.unwrap();
    assert_eq!(candidate.base(), base);
    let view = candidate.view();
    view.append("private", "assistant", "candidate note")
        .await
        .unwrap();
    view.put("topology", &json!("after")).await.unwrap();
    assert!(store.history("private", 10).await.unwrap().is_empty());
    assert_eq!(store.get("topology").await.unwrap(), Some(json!("before")));
    let promoted = candidate.promote().await.unwrap();
    assert_ne!(promoted, base);
    assert_eq!(store.get("topology").await.unwrap(), Some(json!("after")));
    assert_eq!(
        store.history("private", 10).await.unwrap()[0].content,
        "candidate note"
    );
    assert_eq!(candidate.promote().await.unwrap(), promoted);
    let stale = store.begin_candidate("stale").await.unwrap();
    stale.view().put("topology", &json!("stale")).await.unwrap();
    store
        .append("chat", "user", "new conversation")
        .await
        .unwrap();
    assert!(
        stale
            .promote()
            .await
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert_eq!(store.get("topology").await.unwrap(), Some(json!("after")));
    assert_eq!(
        stale.view().get("topology").await.unwrap(),
        Some(json!("stale"))
    );
    assert!(!store.status().await.unwrap().read_only);
    let revisions = store.revisions(20).await.unwrap();
    assert!(
        revisions
            .iter()
            .any(|revision| revision.message.starts_with("message ["))
    );
    assert!(revisions.iter().all(|revision| !revision.hash.is_empty()));
    store.reconcile().await.unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_peers_preserve_each_peers_order() {
    let store = MemoryStore::temporary().await.unwrap();
    let mut tasks = Vec::new();
    for peer in 0..4 {
        let memory = store.clone();
        tasks.push(tokio::spawn(async move {
            for index in 0..8 {
                memory
                    .append("shared", &format!("peer-{peer}"), &index.to_string())
                    .await
                    .unwrap();
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let history = store.history("shared", 100).await.unwrap();
    assert_eq!(history.len(), 32);
    for peer in 0..4 {
        let actual: Vec<_> = history
            .iter()
            .filter(|message| message.role == format!("peer-{peer}"))
            .map(|message| message.content.parse::<usize>().unwrap())
            .collect();
        assert_eq!(actual, (0..8).collect::<Vec<_>>());
    }
    store.close().await.unwrap();
}
