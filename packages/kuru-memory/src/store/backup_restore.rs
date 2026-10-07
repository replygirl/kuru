//! Preparation of an already validated private native restore. Original image
//! checking precedes this job; ordinary writable validation is not relaxed.

use super::*;
use crate::backup::{BackupManifest, BackupRefKind, BackupWorkingSet};

pub(crate) struct Preparation {
    pub manifest: BackupManifest,
    pub manifest_sha256: String,
    pub project_path: Vec<u8>,
    pub project_scope: String,
}

/// Exact native snapshot coordinates, retained within the restore provenance.
/// Only main and the existing usage branch can be prepared for writable open.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreparedRoots {
    pub branch: String,
    pub original_head: String,
    pub original_working: String,
    pub original_staged: String,
    pub staged_commit: Option<String>,
    pub working_commit: String,
}

pub(crate) async fn prepare(server: &Server, stage: &Path, input: Preparation) -> Result<()> {
    // Observe both original branch tuples before appending any snapshot or
    // running released migrations. The image has no independent writers.
    for reference in input.manifest.refs.iter().filter(|reference| {
        reference.kind == BackupRefKind::Branch
            && ["main", usage_ledger::BRANCH].contains(&reference.name.as_str())
    }) {
        let pool = server.pool(&reference.name).await?;
        let captured = reference
            .working
            .as_ref()
            .context("restored working roots are missing")?;
        ensure!(
            revision(&pool).await? == reference.head
                && root(&pool, "STAGED").await? == captured.staged_root
                && root(&pool, "WORKING").await? == captured.working_root,
            "restored original branch tuple changed before preparation"
        );
    }
    let mut prepared = Vec::new();
    for branch in ["main", usage_ledger::BRANCH] {
        let Some(reference) =
            input.manifest.refs.iter().find(|reference| {
                reference.kind == BackupRefKind::Branch && reference.name == branch
            })
        else {
            ensure!(branch != "main", "restored main ref is missing");
            continue;
        };
        let pool = server.pool(branch).await?;
        let roots = reference
            .working
            .as_ref()
            .context("restored branch working roots are missing")?;
        if let Some(snapshot) = snapshot(&pool, branch, &reference.head, roots).await? {
            prepared.push(snapshot);
        }
        if branch == "main" {
            migrations::upgrade(server, &pool).await?;
            migrations::validate_active(&pool).await?;
        } else {
            // Existing content validation precedes a usage migration and its
            // unchanged clean/schema checks. No imported hash grants writes.
            usage_ledger::validate_restored_content(&pool).await?;
            migrations::upgrade_usage(server, &pool).await?;
            migrations::validate_usage(&pool).await?;
            usage_ledger::validate_restored_content(&pool).await?;
        }
    }
    let main = server.pool("main").await?;
    let initial_revision = revision(&main).await?;
    let activation = Activation {
        format: 2,
        project_scope: input.project_scope.clone(),
        history_scope: Some(input.manifest.history_scope.clone()),
        initial_revision,
        migration: None,
        restore: Some(RestoreReceipt {
            source_project_path: input.manifest.source_project_path,
            target_project_path: input.project_path,
            source_storage_scope: input.manifest.source_storage_scope,
            history_scope: input.manifest.history_scope,
            source_store_instance: input.manifest.source_store_instance,
            sql_origin_instance: input.manifest.sql_origin_instance,
            sql_origin_scope: input.manifest.sql_origin_scope,
            dataset_root: input.manifest.dataset_root,
            backup_manifest_sha256: input.manifest_sha256,
            prepared_roots: prepared,
        }),
    };
    validate_activation(&activation, &input.project_scope)?;
    write_json(&stage.join("ready.json"), &activation)?;
    ensure!(
        read_activation(stage, &input.project_scope)?.initial_revision
            == activation.initial_revision,
        "restored ready marker changed before owned shutdown"
    );
    Ok(())
}

pub(crate) fn ready(stage: &Path, scope: &str) -> Result<(i32, String)> {
    let activation = read_activation(stage, scope)?;
    ensure!(
        activation.restore.is_some(),
        "restore stage has no checked restore provenance"
    );
    Ok((migrations::CURRENT_VERSION, activation.initial_revision))
}

async fn root(pool: &MemoryPool, selected: &str) -> Result<String> {
    crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT DOLT_HASHOF_DB(?)")
            .bind(selected)
            .fetch_one(pool),
    )
    .await
    .context("restored root read deadline exceeded")?
    .map_err(Into::into)
}

async fn snapshot(
    pool: &MemoryPool,
    branch: &str,
    head: &str,
    captured: &BackupWorkingSet,
) -> Result<Option<PreparedRoots>> {
    ensure!(
        revision(pool).await? == head,
        "restored branch head differs from original validation"
    );
    let original_head_root = root(pool, "HEAD").await?;
    ensure!(
        root(pool, "STAGED").await? == captured.staged_root
            && root(pool, "WORKING").await? == captured.working_root,
        "restored branch working roots differ from original validation"
    );
    if original_head_root == captured.staged_root && original_head_root == captured.working_root {
        return Ok(None);
    }
    let staged_commit = if captured.staged_root != original_head_root {
        commit_snapshot(pool, false).await?;
        ensure!(
            root(pool, "HEAD").await? == captured.staged_root
                && root(pool, "WORKING").await? == captured.working_root,
            "native staged snapshot differs from captured roots"
        );
        Some(revision(pool).await?)
    } else {
        None
    };
    if root(pool, "HEAD").await? != captured.working_root {
        commit_snapshot(pool, true).await?;
    }
    ensure!(
        root(pool, "HEAD").await? == captured.working_root
            && root(pool, "STAGED").await? == captured.working_root
            && root(pool, "WORKING").await? == captured.working_root,
        "native working snapshot differs from original WORKING"
    );
    Ok(Some(PreparedRoots {
        branch: branch.to_owned(),
        original_head: head.to_owned(),
        original_staged: captured.staged_root.clone(),
        original_working: captured.working_root.clone(),
        staged_commit,
        working_commit: revision(pool).await?,
    }))
}

async fn commit_snapshot(pool: &MemoryPool, working: bool) -> Result<()> {
    let (flags, message) = if working {
        ("-Am", "Preserve restored working snapshot")
    } else {
        ("--message", "Preserve restored staged snapshot")
    };
    crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_COMMIT(?, ?, '--author', ?)")
            .bind(flags)
            .bind(message)
            .bind(AUTHOR)
            .fetch_all(pool),
    )
    .await
    .context("native restore snapshot commit deadline exceeded")??;
    Ok(())
}
