use std::{
    ffi::OsStr,
    fs::File,
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use kuru_memory::{MemoryStore, PublicTranscriptEntry, PublicTurnRecord, SessionCatalogRecord};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde::Serialize;

use crate::{
    cli::SessionExportFormat,
    memory_export::{OutputTarget, StagedExport},
};

const FORMAT_VERSION: u32 = 1;
const SPOOL_NAME: &str = "session.spool";

#[derive(Serialize)]
struct Manifest<'a> {
    kind: &'static str,
    format: u32,
    snapshot: &'static str,
    session: &'a SessionCatalogRecord,
    view: &'a str,
    revision: &'a str,
    head_node_id: &'a Option<String>,
    total_rows: u64,
    excludes: [&'static str; 6],
}

#[derive(Serialize)]
struct PendingRecord<'a> {
    kind: &'static str,
    record: &'a PublicTurnRecord,
}

pub async fn export(
    memory: &MemoryStore,
    session_id: &str,
    format: SessionExportFormat,
    output: Option<&Path>,
    cwd: &Path,
) -> Result<()> {
    let catalog = memory
        .session_catalog_record(session_id)
        .await?
        .context("session is absent from this project")?;
    let target = output
        .map(|output| OutputTarget::open_replace(output, cwd))
        .transpose()?;
    let temporary_parent;
    let staging_parent = match target.as_ref() {
        Some(target) => target.directory(),
        None => {
            temporary_parent = Directory::open(
                &std::env::temp_dir(),
                Privacy::Inherited,
                NameRetention::Movable,
            )
            .context("open system session-export staging parent")?;
            &temporary_parent
        }
    };
    let mut staged = StagedExport::new(staging_parent)?;
    let mut spool = staged.create_auxiliary(OsStr::new(SPOOL_NAME))?;
    let snapshot = spool_newest_first(memory, session_id, &mut spool).await?;
    ensure!(
        snapshot.head_node_id == catalog.head_node_id
            && snapshot.pending.as_ref().map(|record| &record.node_id)
                == catalog.pending_node_id.as_ref(),
        "session catalog changed before its public transcript was captured"
    );
    let current_catalog = memory
        .session_catalog_record(session_id)
        .await?
        .context("session disappeared while its public transcript was exported")?;
    ensure!(
        current_catalog == catalog,
        "session catalog changed during public transcript export"
    );
    render(
        staged.file_mut(),
        &mut spool,
        format,
        &Manifest {
            kind: "manifest",
            format: FORMAT_VERSION,
            snapshot: "committed selected public session",
            session: &catalog,
            view: &snapshot.view,
            revision: &snapshot.revision,
            head_node_id: &snapshot.head_node_id,
            total_rows: snapshot.total_rows,
            excludes: [
                "raw actor histories",
                "relationship histories",
                "private reasoning summaries",
                "context summaries",
                "notes",
                "candidate branches and unrelated sessions",
            ],
        },
        snapshot.pending.as_ref(),
    )?;
    spool
        .sync_all()
        .context("flush private session export spool")?;
    drop(spool);
    staged.remove_auxiliary(OsStr::new(SPOOL_NAME))?;
    staged.complete()?;
    match target {
        Some(target) => staged.publish(target),
        None => staged.copy_stdout(),
    }
}

struct Snapshot {
    view: String,
    revision: String,
    head_node_id: Option<String>,
    pending: Option<PublicTurnRecord>,
    total_rows: u64,
}

async fn spool_newest_first(
    memory: &MemoryStore,
    session_id: &str,
    spool: &mut File,
) -> Result<Snapshot> {
    let mut cursor = None;
    let mut snapshot: Option<Snapshot> = None;
    loop {
        let page = memory
            .public_transcript_page(session_id, cursor.as_ref(), 1024)
            .await?;
        if let Some(expected) = &snapshot {
            ensure!(
                page.view == expected.view
                    && page.revision == expected.revision
                    && page.head_node_id == expected.head_node_id
                    && page.pending == expected.pending
                    && page.total_rows == expected.total_rows,
                "session export snapshot changed during traversal"
            );
        } else {
            snapshot = Some(Snapshot {
                view: page.view.clone(),
                revision: page.revision.clone(),
                head_node_id: page.head_node_id.clone(),
                pending: page.pending.clone(),
                total_rows: page.total_rows,
            });
        }
        for record in page.records {
            let bytes = serde_json::to_vec(&record)?;
            spool.write_all(&bytes)?;
            spool.write_all(
                &u64::try_from(bytes.len())
                    .context("session export record length overflowed")?
                    .to_be_bytes(),
            )?;
        }
        cursor = page.next;
        if cursor.is_none() {
            return snapshot.context("session export did not capture a snapshot");
        }
    }
}

fn render(
    output: &mut impl Write,
    spool: &mut File,
    format: SessionExportFormat,
    manifest: &Manifest<'_>,
    pending: Option<&PublicTurnRecord>,
) -> Result<()> {
    let mut writer = BufWriter::new(output);
    match format {
        SessionExportFormat::Jsonl => write_json_line(&mut writer, manifest)?,
        SessionExportFormat::Markdown => {
            writeln!(
                writer,
                "# Kuru public session export\n\n## Snapshot\n\n```json"
            )?;
            serde_json::to_writer(&mut writer, manifest)?;
            writeln!(writer, "\n```\n\n## Public transcript")?;
        }
    }
    let mut end = spool.seek(SeekFrom::End(0))?;
    while end > 0 {
        ensure!(end >= 8, "session export spool frame is truncated");
        spool.seek(SeekFrom::Start(end - 8))?;
        let mut length = [0_u8; 8];
        spool.read_exact(&mut length)?;
        let length = u64::from_be_bytes(length);
        ensure!(
            length <= end - 8,
            "session export spool frame length is invalid"
        );
        let start = end - 8 - length;
        spool.seek(SeekFrom::Start(start))?;
        let mut bytes = vec![0; usize::try_from(length)?];
        spool.read_exact(&mut bytes)?;
        let entry: PublicTranscriptEntry = serde_json::from_slice(&bytes)?;
        match format {
            SessionExportFormat::Jsonl => write_json_line(&mut writer, &entry)?,
            SessionExportFormat::Markdown => write_markdown_record(&mut writer, &entry)?,
        }
        end = start;
    }
    if let Some(record) = pending {
        let pending = PendingRecord {
            kind: "pending",
            record,
        };
        match format {
            SessionExportFormat::Jsonl => write_json_line(&mut writer, &pending)?,
            SessionExportFormat::Markdown => write_markdown_record(&mut writer, &pending)?,
        }
    }
    writer.flush().context("flush completed session export")
}

fn write_json_line(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn write_markdown_record(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    writeln!(writer, "\n```json")?;
    serde_json::to_writer(&mut *writer, value)?;
    writeln!(writer, "\n```")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{self, BufRead, BufReader};

    use kuru_core::{Message, Mode};
    use kuru_memory::{
        PUBLIC_TURN_RECORD_FORMAT, PublicTurnKind, PublicTurnSettlement,
        SESSION_CATALOG_RECORD_FORMAT, SessionLifecycleState, public_turn_node_id,
    };

    use super::*;

    struct CapacityWriter<W> {
        inner: W,
        remaining: usize,
    }

    impl<W: Write> Write for CapacityWriter<W> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::other("staging capacity exhausted"));
            }
            let count = bytes.len().min(self.remaining);
            let written = self.inner.write(&bytes[..count])?;
            self.remaining -= written;
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    #[test]
    fn staging_capacity_failure_keeps_selected_output_and_cleans_spool() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let destination = temporary.path().join("selected-session.jsonl");
        std::fs::write(&destination, b"previous completed export")?;
        let target = OutputTarget::open_replace(&destination, temporary.path())?;
        let mut staged = StagedExport::new(target.directory())?;
        let stage_path = staged.stage_path().to_path_buf();
        let mut spool = staged.create_auxiliary(OsStr::new(SPOOL_NAME))?;
        let catalog = SessionCatalogRecord {
            session_id: "capacity-session".into(),
            mode: Mode::Ifs,
            label: "capacity".into(),
            created_order: 1,
            updated_order: 1,
            lifecycle_generation: 0,
            lifecycle_state: SessionLifecycleState::Active,
            head_node_id: None,
            pending_node_id: None,
            legacy_prefix: None,
            fork_provenance: None,
            record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
        };
        {
            let mut bounded = CapacityWriter {
                inner: staged.file_mut(),
                remaining: 8,
            };
            let error = render(
                &mut bounded,
                &mut spool,
                SessionExportFormat::Jsonl,
                &Manifest {
                    kind: "manifest",
                    format: FORMAT_VERSION,
                    snapshot: "committed selected public session",
                    session: &catalog,
                    view: "main",
                    revision: "0123456789abcdef0123456789abcdef",
                    head_node_id: &catalog.head_node_id,
                    total_rows: 0,
                    excludes: [
                        "raw actor histories",
                        "relationship histories",
                        "private reasoning summaries",
                        "context summaries",
                        "notes",
                        "candidate branches and unrelated sessions",
                    ],
                },
                None,
            )
            .unwrap_err();
            assert!(
                format!("{error:#}").contains("staging capacity exhausted"),
                "{error:#}"
            );
        }
        drop(spool);
        drop(staged);
        assert!(!stage_path.exists(), "failed export left an orphan stage");
        assert_eq!(std::fs::read(destination)?, b"previous completed export");
        Ok(())
    }

    #[test]
    fn both_formats_reverse_more_than_one_page_and_32_mib_without_omission() -> Result<()> {
        const ROWS: usize = 1025;
        let session_id = "long-export-session";
        let nodes = (0..ROWS)
            .map(|index| public_turn_node_id(session_id, &format!("turn-{index:04}")))
            .collect::<Result<Vec<_>>>()?;
        let catalog = SessionCatalogRecord {
            session_id: session_id.into(),
            mode: Mode::Ifs,
            label: "long export".into(),
            created_order: 1,
            updated_order: 1,
            lifecycle_generation: 0,
            lifecycle_state: SessionLifecycleState::Active,
            head_node_id: nodes.last().cloned(),
            pending_node_id: None,
            legacy_prefix: None,
            fork_provenance: None,
            record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
        };
        let payload = "x".repeat(32 * 1024);
        let mut spool = tempfile::tempfile()?;
        let mut serialized_bytes = 0_u64;
        for index in (0..ROWS).rev() {
            let entry = PublicTranscriptEntry::Turn {
                record: PublicTurnRecord {
                    node_id: nodes[index].clone(),
                    origin_session_id: session_id.into(),
                    turn_id: format!("turn-{index:04}"),
                    kind: PublicTurnKind::Primary,
                    continuation_of_node_id: None,
                    predecessor_node_id: index.checked_sub(1).map(|prior| nodes[prior].clone()),
                    settlement: PublicTurnSettlement::Completed,
                    user_entry: Some(Message::text("user", format!("question-{index:04}"))),
                    speaker_id: Some("speaker-a".into()),
                    terminal_entries: vec![Message::text(
                        "assistant",
                        format!("answer-{index:04}:{payload}"),
                    )],
                    record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
                },
            };
            let bytes = serde_json::to_vec(&entry)?;
            serialized_bytes += u64::try_from(bytes.len())?;
            spool.write_all(&bytes)?;
            spool.write_all(&u64::try_from(bytes.len())?.to_be_bytes())?;
        }
        assert!(serialized_bytes > 32 * 1024 * 1024);

        let manifest = Manifest {
            kind: "manifest",
            format: FORMAT_VERSION,
            snapshot: "committed selected public session",
            session: &catalog,
            view: "main",
            revision: "0123456789abcdef0123456789abcdef",
            head_node_id: &catalog.head_node_id,
            total_rows: ROWS as u64,
            excludes: [
                "raw actor histories",
                "relationship histories",
                "private reasoning summaries",
                "context summaries",
                "notes",
                "candidate branches and unrelated sessions",
            ],
        };
        let mut output = tempfile::tempfile()?;
        render(
            &mut output,
            &mut spool,
            SessionExportFormat::Jsonl,
            &manifest,
            None,
        )?;
        output.seek(SeekFrom::Start(0))?;
        let mut lines = BufReader::new(output).lines();
        let parsed_manifest: serde_json::Value =
            serde_json::from_str(&lines.next().context("long export manifest")??)?;
        assert_eq!(parsed_manifest["total_rows"], ROWS);
        for index in 0..ROWS {
            let entry: PublicTranscriptEntry =
                serde_json::from_str(&lines.next().context("long export omitted a turn")??)?;
            assert!(matches!(
                entry,
                PublicTranscriptEntry::Turn { record }
                    if record.turn_id == format!("turn-{index:04}")
            ));
        }
        assert!(lines.next().is_none());

        let mut markdown = tempfile::tempfile()?;
        render(
            &mut markdown,
            &mut spool,
            SessionExportFormat::Markdown,
            &manifest,
            None,
        )?;
        markdown.seek(SeekFrom::Start(0))?;
        let mut seen = 0;
        for line in BufReader::new(markdown).lines() {
            let line = line?;
            if !line.starts_with('{') {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(&line)?;
            if value["kind"] == "turn" {
                assert_eq!(value["record"]["turn_id"], format!("turn-{seen:04}"));
                seen += 1;
            }
        }
        assert_eq!(seen, ROWS);
        Ok(())
    }
}
