//! Template-backed and cold fixture stores against the real engine.
use super::*;
use serde_json::json;

async fn migration_receipts(store: &MemoryStore) -> Result<Vec<(i32, String)>> {
    let rows = sqlx::query("SELECT version, operation FROM kuru_migrations ORDER BY version")
        .fetch_all(store.pool.as_ref())
        .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("version")?, row.try_get("operation")?)))
        .collect()
}

async fn port(store: &MemoryStore) -> Result<i64> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT CAST(@@port AS SIGNED)")
            .fetch_one(store.pool.as_ref())
            .await?,
    )
}

#[tokio::test]
async fn template_copies_are_isolated_stores_with_their_own_servers() -> Result<()> {
    let first = MemoryStore::temporary().await?;
    let second = MemoryStore::temporary().await?;
    let (first_status, second_status) = (first.status().await?, second.status().await?);
    ensure!(
        first_status.directory != second_status.directory,
        "template copies share a directory"
    );
    ensure!(
        port(&first).await? != port(&second).await?,
        "template copies share a server"
    );
    // The documented limitation: copies share the template's identity.
    assert_eq!(first.service_instance(), second.service_instance());
    assert_eq!(first_status.revision, second_status.revision);

    first
        .put("template-isolation", &json!({"owner": "first"}))
        .await?;
    first.append("part/first", "user", "first only").await?;
    assert_eq!(second.get("template-isolation").await?, None);
    assert!(second.history("part/first", 10).await?.is_empty());
    assert_eq!(second.revision().await?, second_status.revision);
    second
        .put("template-isolation", &json!({"owner": "second"}))
        .await?;
    assert_eq!(
        first.get("template-isolation").await?,
        Some(json!({"owner": "first"}))
    );
    first.close().await?;
    assert_eq!(
        second.get("template-isolation").await?,
        Some(json!({"owner": "second"}))
    );
    second.close().await
}

#[tokio::test]
async fn cold_constructor_runs_every_migration_under_a_new_identity() -> Result<()> {
    let copied = MemoryStore::temporary().await?;
    let cold = MemoryStore::temporary_cold().await?;
    ensure!(
        copied.service_instance() != cold.service_instance(),
        "cold fixture reused the template identity"
    );
    let (copied_receipts, cold_receipts) = (
        migration_receipts(&copied).await?,
        migration_receipts(&cold).await?,
    );
    let versions = |receipts: &[(i32, String)]| -> Vec<i32> {
        receipts.iter().map(|(version, _)| *version).collect()
    };
    assert_eq!(versions(&cold_receipts), versions(&copied_receipts));
    assert_eq!(
        cold_receipts.last().map(|(version, _)| *version),
        Some(migrations::CURRENT_VERSION)
    );
    assert_eq!(
        migrations::version(&cold.pool).await?,
        migrations::CURRENT_VERSION
    );
    // Every migration operation is a fresh UUID, so no receipt is shared.
    for (version, operation) in &cold_receipts {
        ensure!(
            !copied_receipts
                .iter()
                .any(|(_, copied)| copied == operation),
            "cold migration {version} reused the template's operation"
        );
    }
    ensure!(
        copied.revision().await? != cold.revision().await?,
        "cold fixture reused the template revision"
    );
    copied.close().await?;
    cold.close().await
}
