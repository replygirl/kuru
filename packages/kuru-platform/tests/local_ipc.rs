#![cfg(unix)]

use kuru_platform::{
    fs::{Directory, NameRetention, Privacy},
    local_ipc,
};
use std::{ffi::OsStr, fs, os::unix::fs::PermissionsExt, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEADLINE: Duration = Duration::from_secs(5);

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
