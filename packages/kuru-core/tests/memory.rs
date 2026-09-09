use std::{
    collections::HashSet,
    fs,
    sync::{Arc, Barrier},
    thread,
};

use kuru_core::{MemoryStore, Message};
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn namespace_isolation_and_recent_history_order_are_preserved() {
    let store = MemoryStore::in_memory().unwrap();
    for i in 0..6 {
        store
            .append("part/one", "user", &format!("one-{i}"))
            .unwrap();
        store
            .append("part/two", "assistant", &format!("two-{i}"))
            .unwrap();
    }
    let expected = (3..6)
        .map(|i| Message {
            role: "user".into(),
            content: format!("one-{i}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(store.history("part/one", 3).unwrap(), expected);
    assert_eq!(store.history("part/two", 100).unwrap().len(), 6);
    assert!(store.history("missing", 100).unwrap().is_empty());
    assert!(store.history("part/one", 0).unwrap().is_empty());
    let other = MemoryStore::in_memory().unwrap();
    assert!(other.history("part/one", 100).unwrap().is_empty());
}

#[test]
fn unicode_nul_content_and_tool_roles_roundtrip_without_normalization() {
    let store = MemoryStore::in_memory().unwrap();
    let content = "Привет 東京 🫧 e\u{301} é\0\n";
    store.append("部品/記憶", "tool", content).unwrap();
    store
        .append("部品/記憶", "future_protocol_role", "")
        .unwrap();
    assert_eq!(
        store.history("部品/記憶", 2).unwrap(),
        [
            Message {
                role: "tool".into(),
                content: content.into()
            },
            Message {
                role: "future_protocol_role".into(),
                content: "".into()
            },
        ]
    );
    store
        .put("日本語", &json!({"text":content,"nested":[1,true,null]}))
        .unwrap();
    assert_eq!(
        store.get("日本語").unwrap(),
        Some(json!({"text":content,"nested":[1,true,null]}))
    );
}

#[test]
fn sql_looking_identifiers_are_bound_as_values() {
    let store = MemoryStore::in_memory().unwrap();
    let injection = "'; DELETE FROM messages; --";
    store.append("safe", "user", "keep").unwrap();
    store.append(injection, "assistant", "literal").unwrap();
    store.put(injection, &json!("literal-key")).unwrap();
    assert_eq!(store.history(injection, 10).unwrap()[0].content, "literal");
    assert_eq!(store.get(injection).unwrap(), Some(json!("literal-key")));
    store.clear(injection).unwrap();
    assert_eq!(store.history("safe", 10).unwrap()[0].content, "keep");
}

#[test]
fn clearing_only_one_namespace_does_not_erase_state_or_other_histories() {
    let store = MemoryStore::in_memory().unwrap();
    for namespace in ["part/a", "part/b", "group/a-b", "session/a"] {
        store.append(namespace, "user", namespace).unwrap();
    }
    store.put("part/a", &json!({"archived":true})).unwrap();
    store.clear("part/a").unwrap();
    store.clear("missing").unwrap();
    assert!(store.history("part/a", 10).unwrap().is_empty());
    for namespace in ["part/b", "group/a-b", "session/a"] {
        assert_eq!(store.history(namespace, 10).unwrap()[0].content, namespace);
    }
    assert_eq!(store.get("part/a").unwrap(), Some(json!({"archived":true})));
    store.append("part/a", "assistant", "fresh").unwrap();
    assert_eq!(store.history("part/a", 100).unwrap().len(), 1);
}

#[test]
fn state_upserts_and_json_null_are_distinct_from_missing_keys() {
    let store = MemoryStore::in_memory().unwrap();
    assert_eq!(store.get("key").unwrap(), None);
    store.put("key", &json!([1, 2, 3])).unwrap();
    assert_eq!(store.get("key").unwrap(), Some(json!([1, 2, 3])));
    store.put("key", &Value::Null).unwrap();
    assert_eq!(store.get("key").unwrap(), Some(Value::Null));
    assert_eq!(store.get("other").unwrap(), None);
}

#[test]
fn related_state_updates_commit_or_rollback_together() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("atomic.sqlite3");
    let store = MemoryStore::open(&path).unwrap();
    store
        .put_many(&[("one".into(), json!(1)), ("two".into(), json!(2))])
        .unwrap();
    assert_eq!(store.get("one").unwrap(), Some(json!(1)));
    assert_eq!(store.get("two").unwrap(), Some(json!(2)));
    assert!(
        store
            .put_many(&[("one".into(), json!(10)), ("".into(), Value::Null)])
            .is_err()
    );
    assert!(
        store
            .put_many(&[("one".into(), json!(10)), ("one".into(), json!(20))])
            .is_err()
    );
    assert_eq!(store.get("one").unwrap(), Some(json!(1)));
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_bad BEFORE INSERT ON state WHEN NEW.key = 'bad' BEGIN SELECT RAISE(ABORT, 'simulated storage failure'); END;").unwrap();
    assert!(
        store
            .put_many(&[("one".into(), json!(10)), ("bad".into(), Value::Null)])
            .is_err()
    );
    assert_eq!(
        store.get("one").unwrap(),
        Some(json!(1)),
        "the first write must roll back when a later write fails"
    );
    assert!(store.get("bad").unwrap().is_none());
    store.put_many(&[]).unwrap();
}

#[cfg(unix)]
#[test]
fn an_existing_database_symlink_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("target.sqlite3");
    fs::write(&target, "leave untouched").unwrap();
    let alias = dir.path().join("memory.sqlite3");
    symlink(&target, &alias).unwrap();
    assert!(format!("{:#}", MemoryStore::open(&alias).unwrap_err()).contains("symbolic link"));
    assert_eq!(fs::read_to_string(&target).unwrap(), "leave untouched");
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn reserved_sqlite_filename_is_still_durable_when_opened_as_a_path() {
    const CHILD: &str = "KURU_CORE_TEST_RELATIVE_MEMORY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let path = std::path::Path::new(":memory:");
        let store = MemoryStore::open(path).unwrap();
        store.append("durable", "user", "persist this").unwrap();
        drop(store);
        assert_eq!(
            MemoryStore::open(path)
                .unwrap()
                .history("durable", 1)
                .unwrap()[0]
                .content,
            "persist this"
        );
        return;
    }
    // Isolate the working directory in a child process; process-wide chdir would
    // race unrelated tests running concurrently in this test executable.
    let dir = TempDir::new().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "reserved_sqlite_filename_is_still_durable_when_opened_as_a_path",
        ])
        .current_dir(dir.path())
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(dir.path().join(":memory:").metadata().unwrap().len() > 0);
}

#[test]
fn file_memory_and_state_survive_handles_being_dropped_and_reopened() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested/private/memory.sqlite3");
    {
        let store = MemoryStore::open(&path).unwrap();
        store.append("part/a", "assistant", "durable").unwrap();
        store
            .put("topology", &json!({"members":["a","b"]}))
            .unwrap();
    }
    let reopened = MemoryStore::open(&path).unwrap();
    assert_eq!(reopened.history("part/a", 1).unwrap()[0].content, "durable");
    assert_eq!(
        reopened.get("topology").unwrap(),
        Some(json!({"members":["a","b"]}))
    );
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "application_id", |row| row.get(0))
            .unwrap(),
        0x4b55_5255
    );
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .pragma_query_value::<String, _>(None, "journal_mode", |row| row.get(0))
            .unwrap(),
        "wal"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn concurrent_clones_preserve_all_writes_and_each_writers_order() {
    let store = MemoryStore::in_memory().unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|writer| {
            let store = store.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                for i in 0..30 {
                    store
                        .append("shared", &format!("writer-{writer}"), &i.to_string())
                        .unwrap();
                    store.put(&format!("writer-{writer}"), &json!(i)).unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let history = store.history("shared", 300).unwrap();
    assert_eq!(history.len(), 240);
    for writer in 0..8 {
        let role = format!("writer-{writer}");
        let entries: Vec<_> = history
            .iter()
            .filter(|m| m.role == role)
            .map(|m| m.content.parse::<usize>().unwrap())
            .collect();
        assert_eq!(entries, (0..30).collect::<Vec<_>>());
        assert_eq!(store.get(&role).unwrap(), Some(json!(29)));
    }
}

#[test]
fn independent_handles_coordinate_durable_writes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("memory.sqlite3");
    let first = MemoryStore::open(&path).unwrap();
    let second = MemoryStore::open(&path).unwrap();
    let worker = thread::spawn(move || {
        for i in 0..40 {
            second.append("peer", "second", &i.to_string()).unwrap();
        }
    });
    for i in 0..40 {
        first.append("peer", "first", &i.to_string()).unwrap();
    }
    worker.join().unwrap();
    let history = first.history("peer", 100).unwrap();
    assert_eq!(history.len(), 80);
    assert_eq!(
        history
            .iter()
            .map(|message| (&message.role, &message.content))
            .collect::<HashSet<_>>()
            .len(),
        80
    );
}

#[test]
fn concurrent_first_opens_initialize_one_shared_schema() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("fresh.sqlite3");
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let path = path.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                let store = MemoryStore::open(&path).unwrap();
                store.append("shared", "writer", &i.to_string()).unwrap();
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let store = MemoryStore::open(&path).unwrap();
    assert_eq!(store.history("shared", 100).unwrap().len(), 8);
}

#[test]
fn invalid_identifiers_and_unrepresentable_limits_are_rejected() {
    let store = MemoryStore::in_memory().unwrap();
    for invalid in [
        "".to_owned(),
        " ".into(),
        "nul\0id".into(),
        "x".repeat(1025),
    ] {
        assert!(store.append(&invalid, "user", "body").is_err());
        assert!(store.history(&invalid, 1).is_err());
        assert!(store.clear(&invalid).is_err());
        assert!(store.put(&invalid, &Value::Null).is_err());
        assert!(store.get(&invalid).is_err());
    }
    for invalid in ["".to_owned(), " ".into(), "tool\0".into(), "x".repeat(129)] {
        assert!(store.append("valid", &invalid, "body").is_err());
    }
    if usize::BITS > 63 {
        assert!(store.history("valid", usize::MAX).is_err());
    }
}

#[test]
fn corrupt_files_unidentified_schemas_and_future_versions_fail_without_reinitialization() {
    let dir = TempDir::new().unwrap();
    let corrupt = dir.path().join("corrupt.sqlite3");
    fs::write(&corrupt, b"this is not sqlite").unwrap();
    assert!(MemoryStore::open(&corrupt).is_err());
    assert_eq!(fs::read(&corrupt).unwrap(), b"this is not sqlite");
    let foreign = dir.path().join("foreign.sqlite3");
    let connection = Connection::open(&foreign).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE important (value TEXT); INSERT INTO important VALUES ('keep');",
        )
        .unwrap();
    assert!(
        format!("{:#}", MemoryStore::open(&foreign).unwrap_err()).contains("refusing to adopt")
    );
    assert_eq!(
        connection
            .query_row::<String, _, _>("SELECT value FROM important", [], |row| row.get(0))
            .unwrap(),
        "keep"
    );
    let future = dir.path().join("future.sqlite3");
    let store = MemoryStore::open(&future).unwrap();
    store.append("saved", "user", "keep").unwrap();
    drop(store);
    let connection = Connection::open(&future).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    assert!(format!("{:#}", MemoryStore::open(&future).unwrap_err()).contains("version 99"));
    assert_eq!(
        connection
            .query_row::<String, _, _>("SELECT content FROM messages", [], |row| row.get(0))
            .unwrap(),
        "keep"
    );
    connection
        .pragma_update(None, "application_id", 123)
        .unwrap();
    assert!(format!("{:#}", MemoryStore::open(&future).unwrap_err()).contains("does not identify"));
}

#[test]
fn malformed_schema_and_corrupt_json_have_specific_errors() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("memory.sqlite3");
    let store = MemoryStore::open(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute("INSERT INTO state (key,value) VALUES ('bad','{broken')", [])
        .unwrap();
    assert!(
        store
            .get("bad")
            .unwrap_err()
            .to_string()
            .contains("invalid JSON")
    );
    connection.execute_batch("DROP TABLE messages;").unwrap();
    assert!(MemoryStore::open(&path).is_err());
    assert!(store.append("a", "user", "error").is_err());
    assert!(store.history("a", 1).is_err());
    assert!(store.clear("a").is_err());
}

#[test]
fn invalid_parent_or_database_paths_return_filesystem_errors() {
    let dir = TempDir::new().unwrap();
    let parent = dir.path().join("file");
    fs::write(&parent, "not a directory").unwrap();
    assert!(MemoryStore::open(&parent.join("memory.sqlite3")).is_err());
    assert!(MemoryStore::open(dir.path()).is_err());
    assert!(MemoryStore::open(&dir.path().join("bad\0name")).is_err());
}
