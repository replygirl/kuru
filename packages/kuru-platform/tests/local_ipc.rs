#![cfg(unix)]

use kuru_platform::{
    fs::{Directory, NameRetention, Privacy},
    local_ipc,
};
use std::{ffi::OsStr, fs, os::unix::fs::PermissionsExt, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEADLINE: Duration = Duration::from_secs(5);

#[test]
fn private_socket_locators_refuse_malformed_routing_before_creating_a_directory() {
    for locator in [
        "",
        "../outside",
        "0123456789abcdef0123456",
        "0123456789abcdef012345678",
        "0123456789abcdef0123456G",
        "ABCDEF0123456789abcdef01",
    ] {
        let error = local_ipc::prepare_short_directory(locator).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            local_ipc::open_short_directory(locator).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}

#[tokio::test]
async fn pending_accept_refuses_a_replaced_directory_and_drop_preserves_both_names() {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };

    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("private");
    let moved = root.path().join("displaced");
    let listener = local_ipc::PrivateServiceListener::bind_at(
        Directory::ensure_private(&original).unwrap(),
        OsStr::new("generation.sock"),
    )
    .unwrap();
    let error = {
        let mut accepting = std::pin::pin!(listener.accept());
        // Poll exactly once: the checked directory was admitted and the native
        // listener has registered its wait before the namespace changes.
        poll_fn(|cx| {
            assert!(accepting.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        fs::write(original.join("generation.sock"), b"replacement sentinel").unwrap();
        let _client = local_ipc::connect(
            &Directory::ensure_private(&moved).unwrap(),
            OsStr::new("generation.sock"),
        )
        .await
        .unwrap();
        tokio::time::timeout(DEADLINE, accepting)
            .await
            .unwrap()
            .unwrap_err()
    };
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    drop(listener);
    assert_eq!(
        fs::read(original.join("generation.sock")).unwrap(),
        b"replacement sentinel"
    );
    assert!(moved.join("generation.sock").exists());
}

#[tokio::test]
async fn stale_parent_authority_refuses_bind_connect_and_accept_before_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("private");
    let moved = root.path().join("displaced");
    let listener = local_ipc::PrivateServiceListener::bind_at(
        Directory::ensure_private(&original).unwrap(),
        OsStr::new("generation.sock"),
    )
    .unwrap();
    let connector = Directory::ensure_private(&original).unwrap();
    let binder = Directory::ensure_private(&original).unwrap();
    fs::rename(&original, &moved).unwrap();
    Directory::ensure_private(&original).unwrap();
    fs::write(original.join("generation.sock"), b"replacement sentinel").unwrap();

    let rejected_bind = local_ipc::PrivateServiceListener::bind_at(binder, OsStr::new("new.sock"))
        .err()
        .unwrap();
    let rejected_connect = local_ipc::connect(&connector, OsStr::new("generation.sock"))
        .await
        .unwrap_err();
    let rejected_accept = tokio::time::timeout(DEADLINE, listener.accept())
        .await
        .expect("stale listener authority reached a native accept wait")
        .unwrap_err();
    for error in [rejected_bind, rejected_connect, rejected_accept] {
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }
    drop(listener);
    assert_eq!(
        fs::read(original.join("generation.sock")).unwrap(),
        b"replacement sentinel"
    );
    assert!(!original.join("new.sock").exists());
    assert!(!moved.join("new.sock").exists());
    assert!(moved.join("generation.sock").exists());
}

#[tokio::test]
async fn private_socket_accepts_successive_clients_and_cleans_only_its_name() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let directory = Directory::ensure_private(&private).unwrap();
    let name = OsStr::new("service-generation.sock");
    let listener = local_ipc::PrivateServiceListener::bind_at(directory, name).unwrap();
    let path = listener.path();
    assert_eq!(
        fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        local_ipc::PrivateServiceListener::bind_at(
            Directory::ensure_private(&private).unwrap(),
            name
        )
        .is_err()
    );

    for (index, payload) in [b"one".as_slice(), b"two".as_slice()]
        .into_iter()
        .enumerate()
    {
        let mut client = tokio::time::timeout(
            DEADLINE,
            local_ipc::connect(&Directory::ensure_private(&private).unwrap(), name),
        )
        .await
        .unwrap_or_else(|_| panic!("private IPC client {index} connect exceeded 5 seconds"))
        .unwrap();
        let mut server = tokio::time::timeout(DEADLINE, listener.accept())
            .await
            .unwrap_or_else(|_| panic!("private IPC client {index} accept exceeded 5 seconds"))
            .unwrap();
        tokio::time::timeout(DEADLINE, client.write_all(payload))
            .await
            .unwrap_or_else(|_| panic!("private IPC client {index} write exceeded 5 seconds"))
            .unwrap();
        let mut received = vec![0; payload.len()];
        tokio::time::timeout(DEADLINE, server.read_exact(&mut received))
            .await
            .unwrap_or_else(|_| panic!("private IPC client {index} read exceeded 5 seconds"))
            .unwrap();
        assert_eq!(received, payload);
    }

    drop(listener);
    assert!(!path.exists());
}

#[tokio::test]
async fn pending_accepts_retain_independent_waiters_and_receive_distinct_clients() {
    use std::{
        future::{Future, poll_fn},
        sync::Arc,
        task::Poll,
    };

    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let name = OsStr::new("generation.sock");
    let listener = Arc::new(
        local_ipc::PrivateServiceListener::bind_at(
            Directory::ensure_private(&private).unwrap(),
            name,
        )
        .unwrap(),
    );
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..2 {
        let listener = listener.clone();
        let (registered, pending) = tokio::sync::oneshot::channel();
        workers.spawn(async move {
            let mut accepting = std::pin::pin!(listener.accept());
            // Register this task's real waker before admitting either client.
            poll_fn(|cx| {
                assert!(accepting.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            registered.send(()).unwrap();
            let mut stream = accepting.await.unwrap();
            let mut payload = [0; 3];
            stream.read_exact(&mut payload).await.unwrap();
            payload
        });
        tokio::time::timeout(DEADLINE, pending)
            .await
            .expect("accept did not register its waiter")
            .unwrap();
    }
    for payload in [b"one", b"two"] {
        let directory = Directory::ensure_private(&private).unwrap();
        let mut client = tokio::time::timeout(DEADLINE, local_ipc::connect(&directory, name))
            .await
            .expect("pending accepts prevented client admission")
            .unwrap();
        client.write_all(payload).await.unwrap();
    }
    let received = tokio::time::timeout(DEADLINE, async {
        let mut received = Vec::new();
        while let Some(result) = workers.join_next().await {
            received.push(result.unwrap());
        }
        received.sort();
        received
    })
    .await
    .expect("one accept lost its independent readiness waiter");
    assert_eq!(received, [*b"one", *b"two"]);
    drop(listener);
    assert!(!private.join(name).exists());
}

#[tokio::test]
async fn private_socket_refuses_non_socket_and_world_readable_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let directory = Directory::ensure_private(&private).unwrap();
    let name = OsStr::new("endpoint.sock");
    let path = private.join(name);
    fs::write(&path, b"not a socket").unwrap();
    assert!(local_ipc::connect(&directory, name).await.is_err());
    fs::remove_file(&path).unwrap();

    let listener = local_ipc::PrivateServiceListener::bind_at(directory, name).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    let directory = Directory::ensure_private(&private).unwrap();
    assert!(local_ipc::connect(&directory, name).await.is_err());
    drop(listener);
}

#[tokio::test]
async fn private_socket_rejects_a_public_parent_even_if_passed_as_a_directory() {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let inherited =
        Directory::open(root.path(), Privacy::Inherited, NameRetention::Pinned).unwrap();
    assert!(
        local_ipc::PrivateServiceListener::bind_at(inherited, OsStr::new("public.sock")).is_err()
    );
}

#[tokio::test]
async fn listener_drop_preserves_a_replaced_socket_name() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let directory = Directory::ensure_private(&private).unwrap();
    let listener =
        local_ipc::PrivateServiceListener::bind_at(directory, OsStr::new("generation.sock"))
            .unwrap();
    let original = listener.path();
    let moved = private.join("moved.sock");
    fs::rename(&original, &moved).unwrap();
    fs::write(&original, b"replacement").unwrap();
    drop(listener);
    assert_eq!(fs::read(&original).unwrap(), b"replacement");
    fs::remove_file(&moved).unwrap();
}
