//! Compatibility decoding belongs to memory: importing a runtime crate here
//! would invert the storage boundary. Original topology JSON is never rewritten.

use super::*;
use kuru_core::{Part, Relationship};
use serde_json::{Value, json};

#[derive(Deserialize)]
struct LegacyTopology {
    parts: Vec<Value>,
    relationships: Vec<Value>,
    states: BTreeMap<String, Value>,
    focus: Option<LegacyFocus>,
}

#[derive(Deserialize)]
struct LegacyFocus {
    id: String,
    remaining: usize,
}

#[derive(Deserialize)]
struct LegacyReport {
    activation: f64,
    note: String,
}

pub(super) async fn materialize(connection: &mut MySqlConnection) -> Result<()> {
    let mut after = Vec::new();
    loop {
        // Scan bounded key pages, releasing each query before writing through
        // this same short migration transaction. There is no graph-size cap.
        let keys: Vec<Vec<u8>> = bounded_query(
            sqlx::query_scalar("SELECT `key` FROM state WHERE RIGHT(`key`, 9) = ? AND `key` > ? ORDER BY `key` LIMIT 256")
                .bind(b"/topology".as_slice())
                .bind(&after)
                .fetch_all(&mut *connection),
        )
        .await?;
        if keys.is_empty() {
            break;
        }
        after = keys.last().expect("nonempty key page").clone();
        for key in keys {
            let key = String::from_utf8(key).context("legacy topology key is not UTF-8")?;
            let base = key.strip_suffix("/topology").expect("selected suffix");
            let Some((scope, mode)) = base.rsplit_once('/') else {
                continue;
            };
            if scope.is_empty() || !Mode::ALL.iter().any(|known| known.to_string() == mode) {
                continue;
            }
            let row = bounded_query(
                sqlx::query("SELECT OCTET_LENGTH(value) AS value_bytes, IF(OCTET_LENGTH(value) <= ?, value, NULL) AS value FROM state WHERE `key` = ?")
                    .bind(crate::service::rpc::OPERATION_FRAME_LIMIT as u64)
                    .bind(key.as_bytes())
                    .fetch_one(&mut *connection),
            )
            .await?;
            ensure!(
                usize::try_from(row.try_get::<i64, _>("value_bytes")?)?
                    <= crate::service::rpc::OPERATION_FRAME_LIMIT,
                "legacy topology exceeds the service envelope"
            );
            let raw: String = row.try_get("value")?;
            let topology: LegacyTopology = serde_json::from_str(&raw)
                .with_context(|| format!("invalid legacy mode topology at {key}"))?;
            validate(&topology)?;
            let membership = json!({
                "record_format": "membership.v1",
                "parts": topology.parts,
                "relationships": topology.relationships,
            });
            insert_equal_or_absent(connection, &format!("{base}/membership"), &membership).await?;
            for (identity, report) in topology.states {
                let suffix = Sha256::digest(identity.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let value = json!({"record_format":"state_report.v1", "identity":identity, "report":report});
                insert_equal_or_absent(connection, &format!("{base}/state/{suffix}"), &value)
                    .await?;
            }
        }
    }
    Ok(())
}

fn validate(topology: &LegacyTopology) -> Result<()> {
    let parts = topology
        .parts
        .iter()
        .map(Part::deserialize)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("invalid legacy topology part")?;
    let ids = parts.iter().map(|part| &part.id).collect::<BTreeSet<_>>();
    ensure!(
        ids.len() == parts.len(),
        "legacy topology has duplicate part IDs"
    );
    for relation in &topology.relationships {
        let relation =
            Relationship::deserialize(relation).context("invalid legacy topology relationship")?;
        let canonical = Relationship::new(relation.kind, relation.members.clone())?;
        ensure!(
            canonical.id == relation.id && relation.members.iter().all(|id| ids.contains(id)),
            "invalid legacy topology relationship membership"
        );
    }
    for value in topology.states.values() {
        let report =
            LegacyReport::deserialize(value).context("invalid legacy topology state report")?;
        ensure!(
            report.activation.is_finite(),
            "legacy state report activation is not finite"
        );
        let _ = report.note;
    }
    // Decode the former focus shape, but do not infer which session owns it.
    if let Some(focus) = &topology.focus {
        let _ = (&focus.id, focus.remaining);
    }
    Ok(())
}

async fn insert_equal_or_absent(
    connection: &mut MySqlConnection,
    key: &str,
    value: &Value,
) -> Result<()> {
    super::super::identifier("materialized state key", key, 1024)?;
    let existing = bounded_query(
        sqlx::query("SELECT OCTET_LENGTH(value) AS value_bytes, IF(OCTET_LENGTH(value) <= ?, value, NULL) AS value, version FROM state WHERE `key` = ?")
            .bind(crate::service::rpc::OPERATION_FRAME_LIMIT as u64)
            .bind(key.as_bytes())
            .fetch_optional(&mut *connection),
    ).await?;
    if let Some(existing) = existing {
        ensure!(
            usize::try_from(existing.try_get::<i64, _>("value_bytes")?)?
                <= crate::service::rpc::OPERATION_FRAME_LIMIT,
            "topology destination exceeds the service envelope"
        );
        ensure!(
            existing.try_get::<i64, _>("version")? >= 0,
            "topology destination has a negative version"
        );
        let raw: String = existing.try_get("value")?;
        let found: Value =
            serde_json::from_str(&raw).context("invalid topology destination JSON")?;
        ensure!(
            found == *value,
            "conflicting topology migration destination at {key}"
        );
        // An equal pre-existing row keeps its token; migration never resets ABA.
        return Ok(());
    }
    bounded_query(
        sqlx::query("INSERT INTO state (`key`, value, version) VALUES (?, ?, 0)")
            .bind(key.as_bytes())
            .bind(serde_json::to_string(value)?)
            .execute(&mut *connection),
    )
    .await?;
    Ok(())
}
