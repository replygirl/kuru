//! One owner-served native dataset copy. The exact SQL connection and query
//! stay on an independent reactor until a checked outcome or honest unknown
//! result. Unknown stages stay named; no Drop removes their native image.

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use tokio::sync::oneshot;

use super::{BackupCancellation, SQL_TIMEOUT, native::WorkerGuard};
use crate::server::BackupConnection;

pub(super) async fn copy(
    connection: BackupConnection,
    image: PathBuf,
    cancellation: &BackupCancellation,
) -> Result<()> {
    let url = url::Url::from_file_path(&image)
        .map_err(|_| anyhow::anyhow!("backup image has no native file URL"))?;
    let (reply, result) = oneshot::channel();
    let requested = cancellation.clone();
    let thread = std::thread::Builder::new()
        .name("kuru-native-backup-copy".into())
        .spawn(move || {
            let outcome = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(async {
                    copy_owned(&connection, url, &requested).await?;
                    #[cfg(test)]
                    connection.hold_completed_cut(&requested).await?;
                    Ok(())
                })
            })();
            let _ = reply.send(outcome);
        })
        .context("start native backup SQL worker")?;
    let mut worker = WorkerGuard::new(thread, cancellation.clone());
    let outcome = result
        .await
        .context("native backup SQL worker stopped without an outcome");
    worker.join();
    outcome?
}

async fn copy_owned(
    description: &BackupConnection,
    url: url::Url,
    cancellation: &BackupCancellation,
) -> Result<()> {
    ensure!(
        !cancellation.is_cancelled(),
        "native backup cancelled before SQL launch"
    );
    let deadline = tokio::time::Instant::now() + SQL_TIMEOUT;
    let mut connection = description.connect().await?;
    let id: u64 = crate::pool::within_until(
        deadline,
        sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&mut connection),
    )
    .await
    .context("native backup SQL identity deadline exceeded")??;
    ensure!(
        u32::try_from(id).is_ok(),
        "native backup SQL connection identity is invalid"
    );
    // This second independent connection never acquires an ordinary writer
    // guard. Retaining the first connection prevents numeric ID reuse.
    let mut control = description.connect().await?;
    ensure!(
        !cancellation.is_cancelled(),
        "native backup cancelled before SQL launch"
    );
    {
        let query = sqlx::query("CALL DOLT_BACKUP(?, ?)")
            .bind("sync-url")
            .bind(url.as_str())
            .execute(&mut connection);
        tokio::pin!(query);
        let interrupt = tokio::select! {
            biased;
            result = &mut query => {
                #[cfg(test)]
                if description.discard_reply {
                    // Native CALL ran, but this caller deliberately retains
                    // neither its result nor the positive finished-command
                    // proof. The unpublished image must remain unconfirmed.
                    bail!("fixture discarded native backup CALL reply before completion proof");
                }
                observe_finished(&mut control, id).await?;
                result.context("native Dolt backup copy failed")?;
                return Ok(());
            }
            _ = cancellation.cancelled() => "native backup caller cancelled",
            _ = tokio::time::sleep_until(deadline) => "native backup SQL deadline exceeded",
        };
        // KILL CONNECTION deletes process-list evidence before the underlying
        // handler has necessarily unwound. Query-only cancellation keeps its
        // exact connection available for the positive finished-command check.
        let statement = sqlx::AssertSqlSafe(format!("KILL QUERY {id}"));
        let killed = crate::pool::within(
            crate::store::QUERY_TIMEOUT,
            sqlx::query(statement).execute(&mut control),
        )
        .await;
        let terminal = crate::pool::within_until(deadline, &mut query).await;
        match terminal {
            Ok(terminal) => {
                observe_finished(&mut control, id).await?;
                if !matches!(killed, Ok(Ok(_))) {
                    bail!(
                        "{interrupt}; cancellation request failed; copy settled without publication"
                    );
                }
                // A response and idle command prove the procedure has ended,
                // including its deferred destination Close. Its bytes remain
                // unpublished regardless of whether it completed or refused.
                let _ = terminal;
                bail!("{interrupt}; SQL copy settled; private image remains unpublished");
            }
            Err(_) => Err(anyhow::anyhow!(
                "{interrupt}; SQL copy outcome remains uncertain; preserve the private image without hashing, deleting or publishing it"
            )),
        }
    }
}

async fn observe_finished(control: &mut sqlx::MySqlConnection, id: u64) -> Result<()> {
    crate::pool::within(crate::store::QUERY_TIMEOUT, async {
        loop {
            let command: Option<String> = sqlx::query_scalar(
                "SELECT COMMAND FROM information_schema.processlist WHERE ID = ?",
            ).bind(id).fetch_optional(&mut *control).await?;
            match command {
                Some(command) if command.eq_ignore_ascii_case("Sleep") => return Ok(()),
                Some(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                None => bail!("native backup connection disappeared; copy cleanup is uncertain; preserve unpublished stage"),
            }
        }
    }).await.context("native backup finished-command observation remains uncertain; preserve unpublished stage")?
}
