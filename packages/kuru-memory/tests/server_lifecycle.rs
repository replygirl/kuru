use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    task::Poll,
    time::Duration,
};
#[cfg(unix)]
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    sync::mpsc,
};

use anyhow::{Context, Result, ensure};
use kuru_memory::{
    server::{Server, ServerOptions},
    test_support,
};
use sqlx::Row;
use tokio::{
    sync::Semaphore,
    time::{Instant, sleep},
};

static SERVERS: Semaphore = Semaphore::const_new(1);

async fn engine() -> Result<PathBuf> {
    test_support::warm_runtime_cache().await
}

fn options(root: &Path, binary: PathBuf) -> ServerOptions {
    ServerOptions {
        binary,
        directory: root.join("project café"),
        project_scope: "project/server-fixture".into(),
        supervisor: PathBuf::from(env!("CARGO_BIN_EXE_kuru-memory")),
        timeout: Duration::from_secs(20),
        read_only: false,
        retained: None,
        lifecycle_root: if cfg!(windows) {
            Some(root.join("lifecycles"))
        } else {
            None
        },
    }
}

async fn open(options: ServerOptions) -> Result<Server> {
    let log = options.directory.join("server.log");
    Server::open(options).await.with_context(|| {
        format!(
            "server fixture diagnostics: {}",
            fs::read_to_string(log).unwrap_or_default()
        )
    })
}

async fn wait_removed(path: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while path.try_exists()? {
        ensure!(
            Instant::now() < deadline,
            "owned memory fixture was not cleaned: {}",
            path.display()
        );
        sleep(Duration::from_millis(25)).await;
    }
    Ok(())
}

#[tokio::test]
async fn configured_startup_budget_is_not_preempted_by_a_shorter_query_timer() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    let writer = open(options(root.path(), engine().await?)).await?;
    let pool = writer.pool("main").await?;
    let auto_gc_enabled: i64 = sqlx::query_scalar("SELECT @@GLOBAL.dolt_auto_gc_enabled")
        .fetch_one(pool.as_ref())
        .await?;
    ensure!(
        auto_gc_enabled == 1,
        "pinned Dolt server did not enable automatic GC from generated configuration"
    );
    // Dolt uses the listener read timeout while executing a result iterator,
    // including bootstrap DDL. A valid query inside our 20-second budget must
    // not inherit an unrelated five-second server cancellation timer.
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        sqlx::query_scalar::<_, i64>("SELECT SLEEP(6)").fetch_one(pool.as_ref()),
    )
    .await;
    let elapsed = started.elapsed();
    pool.close().await;
    let stopped = writer.close().await;
    stopped?;
    assert_eq!(
        result
            .context("controlled query exceeded its outer ten-second bound")?
            .with_context(|| format!("query failed after {elapsed:?}"))?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn authenticated_readers_branch_pools_and_reopen_share_only_committed_state() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    let opts = options(root.path(), engine().await?);
    let writer = open(opts.clone()).await?;
    let main = writer.pool("main").await?;
    sqlx::query("CREATE TABLE fixture (id INT PRIMARY KEY, value TEXT NOT NULL)")
        .execute(main.as_ref())
        .await?;
    sqlx::query("INSERT INTO fixture VALUES (1, 'baseline')")
        .execute(main.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'server baseline', '--author', 'Kuru tests <tests@example.invalid>')").execute(main.as_ref()).await?;
    sqlx::query("CALL DOLT_BRANCH('candidate_test')")
        .execute(main.as_ref())
        .await?;
    let candidate = writer.pool("candidate_test").await?;
    sqlx::query("INSERT INTO fixture VALUES (2, 'candidate')")
        .execute(candidate.as_ref())
        .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fixture")
            .fetch_one(main.as_ref())
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fixture")
            .fetch_one(candidate.as_ref())
            .await?,
        2
    );
    let mut read_options = opts.clone();
    read_options.read_only = true;
    let reader = Server::open(read_options.clone()).await?;
    let read = reader.pool("main").await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fixture")
            .fetch_one(read.as_ref())
            .await?,
        1
    );
    assert!(
        sqlx::query("INSERT INTO fixture VALUES (3, 'forbidden')")
            .execute(read.as_ref())
            .await
            .is_err()
    );
    assert!(
        sqlx::query("CREATE TABLE forbidden (id INT)")
            .execute(read.as_ref())
            .await
            .is_err()
    );
    assert!(
        sqlx::query("CALL DOLT_BRANCH('reader_must_not_create')")
            .execute(read.as_ref())
            .await
            .is_err(),
        "read-only account created a branch"
    );
    assert!(
        sqlx::query("CALL DOLT_COMMIT('--allow-empty', '-m', 'forbidden', '--author', 'Kuru tests <tests@example.invalid>')")
            .execute(read.as_ref())
            .await
            .is_err(),
        "read-only account created a revision"
    );
    assert!(writer.pool("main/other").await.is_err());
    let mut wrong = opts.clone();
    wrong.project_scope = "project/foreign".into();
    assert!(
        Server::open(wrong)
            .await
            .unwrap_err()
            .to_string()
            .contains("identity")
    );
    reader.close().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fixture")
            .fetch_one(main.as_ref())
            .await?,
        1
    );
    assert!(!format!("{writer:?}").contains("password"));
    writer.close().await?;
    writer.close().await?;
    assert!(writer.pool("main").await.is_err());
    assert!(!opts.directory.join("endpoint.json").exists());
    let log = std::fs::read_to_string(opts.directory.join("server.log"))?;
    assert!(
        log.ends_with("\nKuru engine shutdown: Graceful\n"),
        "real Dolt did not complete the graceful shutdown branch: {log}"
    );
    let reopened_reader = open(read_options).await?;
    let pool = reopened_reader.pool("main").await?;
    assert_eq!(
        sqlx::query("SELECT value FROM fixture WHERE id=1")
            .fetch_one(pool.as_ref())
            .await?
            .try_get::<String, _>(0)?,
        "baseline"
    );
    assert!(
        sqlx::query("DELETE FROM fixture")
            .execute(pool.as_ref())
            .await
            .is_err()
    );
    reopened_reader.close().await?;
    let reopened = open(opts).await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fixture")
            .fetch_one(reopened.pool("candidate_test").await?.as_ref())
            .await?,
        2
    );
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn retained_fixture_is_deleted_only_after_drop_reaps_the_supervisor() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    let path = root.path().to_path_buf();
    let server = open(options(root.path(), engine().await?)).await?;
    server.retain_directory(root).await?;
    let pool = server.pool("main").await?;
    drop(pool);
    drop(server);
    wait_removed(&path).await
}

#[tokio::test]
async fn writable_open_waits_for_a_cold_reader_then_owns_its_lifetime() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    let opts = options(root.path(), engine().await?);
    let initialized = open(opts.clone()).await?;
    initialized.close().await?;
    let reader = open(ServerOptions {
        read_only: true,
        ..opts.clone()
    })
    .await?;
    let mut pending = Box::pin(Server::open(opts.clone()));
    let early = tokio::time::timeout(Duration::from_millis(150), pending.as_mut()).await;
    let writer = match early {
        Ok(result) => {
            // Close the cold reader before any assertion, even on the old
            // attach path.
            reader.close().await?;
            let diagnostic = format!("{result:?}");
            if let Ok(writer) = result {
                writer.close().await?;
            }
            anyhow::bail!(
                "writable open completed while a cold reader still owned the server: {diagnostic}"
            );
        }
        Err(_) => {
            // Keep polling the already-started writer while the reader's
            // supervisor reaps. Its parent-side readiness deadline is live
            // throughout predecessor cleanup and must observe the successor's
            // response instead of being left dormant until after the deadline.
            let (closed, writer) = tokio::join!(reader.close(), pending);
            closed?;
            writer?
        }
    };
    let pool = writer.pool("main").await?;
    sqlx::query("CREATE TABLE writer_lifetime (id INT PRIMARY KEY)")
        .execute(pool.as_ref())
        .await?;
    sqlx::query("INSERT INTO writer_lifetime VALUES (42)")
        .execute(pool.as_ref())
        .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT id FROM writer_lifetime")
            .fetch_one(pool.as_ref())
            .await?,
        42
    );
    writer.close().await?;
    assert!(!opts.directory.join("endpoint.json").exists());
    Ok(())
}

#[tokio::test]
async fn second_writable_open_times_out_without_disrupting_the_owner() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    let opts = options(root.path(), engine().await?);
    let owner = open(opts.clone()).await?;
    let mut second_options = opts;
    second_options.timeout = Duration::from_millis(100);
    let result = Server::open(second_options).await;
    let error = match result {
        Ok(second) => {
            second.close().await?;
            None
        }
        Err(error) => Some(error),
    };
    let pool = owner.pool("main").await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kuru_instance")
        .fetch_one(pool.as_ref())
        .await?;
    owner.close().await?;
    assert_eq!(count, 1, "second opener damaged the active owner");
    let error =
        error.context("second writable open incorrectly attached to another owner's lifetime")?;
    assert!(
        format!("{error:#}").contains("lifecycle lock is held"),
        "{error:#}"
    );
    Ok(())
}

#[tokio::test]
async fn short_lived_branch_pools_do_not_exhaust_the_server_connection_limit() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    let server = open(options(root.path(), engine().await?)).await?;
    let main = server.pool("main").await?;
    assert!(Arc::ptr_eq(&main, &server.pool("main").await?));
    sqlx::query("CALL DOLT_COMMIT('-Am', 'pool fixture baseline', '--author', 'Kuru tests <tests@example.invalid>')")
        .execute(main.as_ref()).await?;
    for index in 0..40 {
        let branch = format!("short_lived_{index}");
        sqlx::query("CALL DOLT_BRANCH(?)")
            .bind(&branch)
            .execute(main.as_ref())
            .await?;
        let pool = server.pool(&branch).await?;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kuru_instance")
                .fetch_one(pool.as_ref())
                .await?,
            1
        );
        drop(pool);
    }
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM information_schema.processlist")
        .fetch_one(main.as_ref())
        .await?;
    assert!(
        sessions < 8,
        "discarded pools retained {sessions} SQL sessions"
    );
    server.close().await?;
    Ok(())
}

#[tokio::test]
async fn wrong_credentials_directory_and_sql_identity_fail_without_server_takeover() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    let opts = options(root.path(), engine().await?);
    let server = open(opts.clone()).await?;
    let pool = server.pool("main").await?;
    let identity_path = opts.directory.join("identity.json");
    let endpoint_path = opts.directory.join("endpoint.json");
    let identity = fs::read(&identity_path)?;
    let endpoint = fs::read(&endpoint_path)?;
    let mut invalid: serde_json::Value = serde_json::from_slice(&identity)?;
    invalid["password"] = "0".repeat(64).into();
    fs::write(&identity_path, serde_json::to_vec(&invalid)?)?;
    let credentials_result = Server::open(opts.clone()).await;
    fs::write(&identity_path, &identity)?;
    assert!(
        credentials_result.is_err(),
        "wrong credentials were accepted"
    );

    let cloned = root.path().join("cloned identity");
    kuru_platform::fs::Directory::ensure_private(&cloned)?;
    fs::copy(&identity_path, cloned.join("identity.json"))?;
    fs::copy(&endpoint_path, cloned.join("endpoint.json"))?;
    let mut wrong_directory = opts.clone();
    wrong_directory.directory = cloned.clone();
    let directory_result = Server::open(wrong_directory).await;
    assert!(directory_result.is_err());
    assert!(
        !cloned.join("server.yaml").exists(),
        "foreign endpoint was adopted"
    );

    let mut connection = pool.acquire().await?;
    sqlx::query("UPDATE kuru_instance SET project_scope='foreign-sql-project'")
        .execute(&mut *connection)
        .await?;
    let sql_identity_result = Server::open(opts.clone()).await;
    sqlx::query("UPDATE kuru_instance SET project_scope=?")
        .bind(&opts.project_scope)
        .execute(&mut *connection)
        .await?;
    assert!(
        sql_identity_result.is_err(),
        "wrong SQL identity was accepted"
    );
    assert_eq!(fs::read(&endpoint_path)?, endpoint);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kuru_instance")
            .fetch_one(&mut *connection)
            .await?,
        1
    );
    drop(connection);
    server.close().await?;

    // Observe diagnostic assertions only after restoring the authentic identity
    // and closing the real server, including during the expected RED run.
    let recorded: serde_json::Value = serde_json::from_slice(&identity)?;
    let wrong_password = "0".repeat(64);
    let secrets = [
        recorded["password"].as_str().context("fixture password")?,
        recorded["reader_password"]
            .as_str()
            .context("fixture reader password")?,
        wrong_password.as_str(),
    ];
    let credentials_error = credentials_result.unwrap_err();
    let directory_error = directory_result.unwrap_err();
    let sql_identity_error = sql_identity_result.unwrap_err();
    for error in [&credentials_error, &directory_error, &sql_identity_error] {
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.len() < 4096,
            "connection diagnostic is unbounded"
        );
        for secret in secrets {
            assert!(
                !diagnostic.contains(secret),
                "connection diagnostic exposed a fixture credential"
            );
        }
        assert!(
            !diagnostic.contains("foreign-sql-project"),
            "connection diagnostic exposed the SQL identity payload"
        );
    }
    for (error, phase, cause) in [
        (
            directory_error,
            "checked data directory comparison",
            "checked filesystem validation failed",
        ),
        (
            sql_identity_error,
            "SQL project/instance comparison",
            "memory SQL project/instance identity mismatch",
        ),
    ] {
        assert!(
            matches!(
                error.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::PoolTimedOut)
            ),
            "original SQLx pool timeout was lost: {error:#}"
        );
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains(phase) && diagnostic.contains(cause),
            "callback rejection lost its checked phase or cause: {diagnostic}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn cancelled_start_retains_its_directory_until_supervisor_cleanup() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = Arc::new(tempfile::tempdir()?);
    let path = root.path().to_path_buf();
    let mut opts = options(root.path(), engine().await?);
    opts.retained = Some(root.clone());
    let mut opening = Box::pin(Server::open(opts));
    // A fresh store has no preliminary connection await. Its first suspension
    // occurs after spawning the supervisor and taking ownership of its pipe.
    // Stop at that suspension instead of racing a wall-clock startup delay.
    std::future::poll_fn(|context| {
        assert!(opening.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(root);
    drop(opening);
    wait_removed(&path).await
}

#[tokio::test]
async fn failed_engine_start_reaps_before_retrying_the_same_private_store() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    // A native failure process must not emit LLVM profiles into the private
    // store when its intentionally isolated environment omits LLVM_PROFILE_FILE.
    #[cfg(unix)]
    let failure = PathBuf::from("/usr/bin/false");
    #[cfg(windows)]
    let failure = kuru_platform::windows::process::system_directory()?.join("where.exe");
    let mut opts = options(root.path(), failure);
    let error = Server::open(opts.clone()).await.unwrap_err();
    assert!(
        format!("{error:#}").contains("before readiness"),
        "{error:#}"
    );
    assert!(!opts.directory.join("endpoint.json").exists());
    opts.binary = engine().await?;
    let server = open(opts).await?;
    server.close().await?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn server_crash_parent_helper() -> Result<()> {
    let Some(root) = std::env::var_os("KURU_SERVER_FIXTURE_ROOT") else {
        return Ok(());
    };
    let binary =
        std::env::var_os("KURU_SERVER_FIXTURE_BINARY").context("fixture engine missing")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let _server = open(options(Path::new(&root), binary.into())).await?;
        println!("KURU_SERVER_PARENT_READY");
        std::io::stdout().flush()?;
        sleep(Duration::from_secs(60)).await;
        Ok(())
    })
}

#[cfg(unix)]
#[tokio::test]
async fn parent_sigkill_closes_lifetime_pipe_and_allows_a_new_owner() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let root = tempfile::tempdir()?;
    let binary = engine().await?;
    let log = root.path().join("parent.stderr");
    let mut parent = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "server_crash_parent_helper",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("KURU_SERVER_FIXTURE_ROOT", root.path())
        .env("KURU_SERVER_FIXTURE_BINARY", &binary)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(fs::File::create(&log)?)
        .spawn()?;
    let output = parent
        .stdout
        .take()
        .context("fixture parent stdout missing")?;
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };
            if line.ends_with("KURU_SERVER_PARENT_READY") {
                let _ = send.send(());
                break;
            }
        }
    });
    let ready = receive.recv_timeout(Duration::from_secs(25));
    // Kill/reap our fixture child before any assertion can unwind its tempdir.
    parent.kill()?;
    parent.wait()?;
    let _ = reader.join();
    ready.with_context(|| {
        format!(
            "fixture parent did not become ready: {}",
            fs::read_to_string(log).unwrap_or_default()
        )
    })?;
    let opts = options(root.path(), binary);
    wait_removed(&opts.directory.join("endpoint.json")).await?;
    let reopened = Server::open(opts).await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kuru_instance")
            .fetch_one(reopened.pool("main").await?.as_ref())
            .await?,
        1
    );
    reopened.close().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn supervisor_sigterm_reaps_and_exits_while_the_parent_pipe_is_open() -> Result<()> {
    let _permit = SERVERS.acquire().await?;
    let binary = engine().await?;
    let root = tempfile::tempdir()?;
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))?;
    let directory = fs::canonicalize(root.path())?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_kuru-memory"))
        .arg("--internal-dolt-supervisor")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().context("supervisor stdin missing")?;
    let mut output = child.stdout.take().context("supervisor stdout missing")?;
    let request = serde_json::to_vec(&serde_json::json!({
        "binary":binary, "directory":directory,
        "project_scope":"signal-fixture", "timeout_millis":20000, "read_only":false, "lifecycle_root":null
    }))?;
    input.write_all(&u32::try_from(request.len())?.to_be_bytes())?;
    input.write_all(&request)?;
    input.flush()?;
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let response = (|| -> Result<serde_json::Value> {
            let mut size = [0; 4];
            output.read_exact(&mut size)?;
            let size = u32::from_be_bytes(size) as usize;
            ensure!(size <= 65536, "unbounded supervisor response");
            let mut body = vec![0; size];
            output.read_exact(&mut body)?;
            Ok(serde_json::from_slice(&body)?)
        })();
        let _ = send.send(response);
    });
    let response = receive.recv_timeout(Duration::from_secs(25));
    let ready = matches!(&response, Ok(Ok(response)) if response.get("Ready").is_some());
    if ready {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child.id().try_into()?),
            nix::sys::signal::Signal::SIGTERM,
        )?;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if !ready || Instant::now() >= deadline {
            break None;
        }
        sleep(Duration::from_millis(20)).await;
    };
    // Cleanup owns the child even on assertion failure. Keep stdin open until
    // after checking its status: closing it would hide a blocked stdin reader.
    drop(input);
    if status.is_none() {
        let cleanup = Instant::now() + Duration::from_secs(13);
        while child.try_wait()?.is_none() && Instant::now() < cleanup {
            sleep(Duration::from_millis(20)).await;
        }
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        child.wait()?;
    }
    let _ = reader.join();
    assert!(ready, "supervisor readiness failed: {response:?}");
    assert!(
        status.is_some_and(|status| status.success()),
        "supervisor did not exit after SIGTERM while parent pipe remained open: {status:?}"
    );
    assert!(!directory.join("endpoint.json").exists());
    Ok(())
}
