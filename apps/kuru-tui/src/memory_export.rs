use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use kuru_memory::{ActiveExportSnapshot, ExportProvenance, StorageRecord};
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication, PublicationPhase};
use uuid::Uuid;

use crate::cli::ExportFormat;

const FORMAT_VERSION: u32 = 4;
const STAGED_PAYLOAD: &str = "export.json";

pub async fn export(
    memory: &kuru_memory::MemoryStore,
    format: ExportFormat,
    output: Option<&Path>,
    cwd: &Path,
) -> Result<()> {
    let target = output
        .map(|output| OutputTarget::open(output, cwd))
        .transpose()?;
    let snapshot = memory.begin_active_export().await?;
    let temporary_parent;
    let staging_parent = match target.as_ref() {
        Some(target) => &target.directory,
        None => {
            temporary_parent = Directory::open(
                &std::env::temp_dir(),
                Privacy::Inherited,
                NameRetention::Movable,
            )
            .context("open system export staging parent")?;
            &temporary_parent
        }
    };
    let mut staged = StagedExport::new(staging_parent)?;
    let counts = render(&snapshot, format, staged.file_mut()).await?;
    snapshot.verify_counts(
        counts.messages,
        counts.state,
        counts.context_summaries,
        counts.context_cursors,
        counts.session_catalog,
        counts.public_turns,
    )?;
    staged.complete()?;
    match target {
        Some(target) => staged.publish(target),
        None => staged.copy_stdout(),
    }
}

pub(crate) struct OutputTarget {
    directory: Directory,
    name: OsString,
    expected: Option<File>,
}

impl OutputTarget {
    pub(crate) fn open(output: &Path, cwd: &Path) -> Result<Self> {
        Self::open_with_replacement(output, cwd, false)
    }

    pub(crate) fn open_replace(output: &Path, cwd: &Path) -> Result<Self> {
        Self::open_with_replacement(output, cwd, true)
    }

    fn open_with_replacement(output: &Path, cwd: &Path, replace: bool) -> Result<Self> {
        let output = if output.is_absolute() {
            output.to_path_buf()
        } else {
            cwd.join(output)
        };
        let parent = output
            .parent()
            .context("export output has no parent directory")?;
        let parent = parent
            .canonicalize()
            .context("export output parent directory does not exist")?;
        ensure!(parent.is_dir(), "export output parent is not a directory");
        let name = output
            .file_name()
            .context("export output has no file name")?;
        ensure!(name != OsStr::new("."), "export output must name a file");
        let directory = Directory::open(&parent, Privacy::Inherited, NameRetention::Pinned)
            .context("open export output directory")?;
        let expected = if replace {
            // Windows replacement requires a delete-sharing handle, while the
            // pinned parent still protects the selected directory identity.
            let movable = Directory::open(&parent, Privacy::Inherited, NameRetention::Movable)
                .context("open movable export output directory")?;
            ensure!(
                movable.identity() == directory.identity(),
                "export output directory changed while it was selected"
            );
            match movable.read(name) {
                Ok(file) => Some(file),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error).context("inspect selected export destination"),
            }
        } else {
            None
        };
        Ok(Self {
            directory,
            name: name.to_owned(),
            expected,
        })
    }

    pub(crate) fn directory(&self) -> &Directory {
        &self.directory
    }

    fn publication(&self) -> Result<Publication> {
        match &self.expected {
            Some(expected) => {
                self.directory
                    .verify(&self.name, expected)
                    .context("selected export destination changed before publication")?;
                Ok(Publication::ReplaceRegular)
            }
            None => Ok(Publication::New),
        }
    }
}

#[derive(Default)]
struct Counts {
    messages: u64,
    state: u64,
    context_summaries: u64,
    context_cursors: u64,
    session_catalog: u64,
    public_turns: u64,
}

async fn render(
    snapshot: &ActiveExportSnapshot,
    format: ExportFormat,
    writer: &mut File,
) -> Result<Counts> {
    let mut writer = BufWriter::new(writer);
    match format {
        ExportFormat::Json => {
            write!(writer, "{{\"manifest\":")?;
            serde_json::to_writer(&mut writer, &manifest(snapshot.provenance()))?;
            write!(writer, ",\"records\":[")?;
        }
        ExportFormat::Markdown => {
            writeln!(writer, "# Kuru committed memory export\n")?;
            writeln!(writer, "## Snapshot\n\n```json")?;
            serde_json::to_writer(&mut writer, &manifest(snapshot.provenance()))?;
            writeln!(writer, "\n```\n\n## Records")?;
        }
    }
    let mut cursor = None;
    let mut first = true;
    let mut counts = Counts::default();
    loop {
        let page = snapshot.page(cursor).await?;
        for record in page.records {
            match &record {
                StorageRecord::Message { .. } => counts.messages += 1,
                StorageRecord::State { .. } => counts.state += 1,
                StorageRecord::ContextSummary { .. } => counts.context_summaries += 1,
                StorageRecord::ContextCursor { .. } => counts.context_cursors += 1,
                StorageRecord::SessionCatalog { .. } => counts.session_catalog += 1,
                StorageRecord::PublicTurn { .. } => counts.public_turns += 1,
            }
            // JSON strings escape line breaks and fence-looking content, so a stored
            // record cannot terminate this Markdown record fence.
            match format {
                ExportFormat::Json => {
                    if !first {
                        write!(writer, ",")?;
                    }
                    serde_json::to_writer(&mut writer, &record)?;
                }
                ExportFormat::Markdown => {
                    render_markdown_record(&mut writer, &record)?;
                }
            }
            first = false;
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    if format == ExportFormat::Json {
        writeln!(writer, "]}}")?;
    }
    writer.flush()?;
    Ok(counts)
}

/// Markdown deliberately retains typed blocks as structured JSON. It must not
/// flatten a tool receipt, call identity, or future block into prose.
fn render_markdown_record(writer: &mut impl Write, record: &StorageRecord) -> Result<()> {
    match record {
        StorageRecord::Message {
            sequence,
            namespace,
            session_id,
            role,
            content_format,
            content,
        } => {
            writeln!(writer, "\n### Message {sequence}\n")?;
            writeln!(writer, "- Content format: `{content_format}`\n")?;
            writeln!(writer, "```json")?;
            let content = if content_format == "typed-v1" {
                serde_json::from_str::<serde_json::Value>(content)
                    .context("typed export record has invalid JSON")?
            } else {
                serde_json::Value::String(content.clone())
            };
            serde_json::to_writer(
                &mut *writer,
                &serde_json::json!({
                    "kind": "message",
                    "sequence": sequence,
                    "namespace": namespace,
                    "session_id": session_id,
                    "role": role,
                    "content_format": content_format,
                    "content": content,
                }),
            )?;
            writeln!(writer, "\n```")?;
        }
        StorageRecord::State { .. }
        | StorageRecord::ContextSummary { .. }
        | StorageRecord::ContextCursor { .. }
        | StorageRecord::SessionCatalog { .. }
        | StorageRecord::PublicTurn { .. } => {
            writeln!(writer, "\n```json")?;
            serde_json::to_writer(&mut *writer, record)?;
            writeln!(writer, "\n```")?;
        }
    }
    Ok(())
}

fn manifest(provenance: &ExportProvenance) -> serde_json::Value {
    serde_json::json!({
        "format": FORMAT_VERSION,
        "snapshot": "committed active main",
        "provenance": provenance,
        "excludes": ["previous revisions", "candidate branches", "uncommitted working rows", "operations and schema tables"],
    })
}

pub(crate) struct StagedExport {
    // Field order closes the payload and checked directory before Drop removes it.
    file: Option<File>,
    directory: Option<Directory>,
    temp: PrivateStage,
    auxiliaries: Vec<(OsString, File)>,
    retain_on_drop: bool,
}

impl StagedExport {
    pub(crate) fn new(parent: &Directory) -> Result<Self> {
        let (temp, directory) = PrivateStage::new(parent)?;
        let file = directory
            .create_new(OsStr::new(STAGED_PAYLOAD))
            .context("create export staging payload")?;
        Ok(Self {
            file: Some(file),
            directory: Some(directory),
            temp,
            auxiliaries: Vec::new(),
            retain_on_drop: false,
        })
    }

    pub(crate) fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("open export staging payload")
    }

    #[cfg(test)]
    pub(crate) fn stage_path(&self) -> &Path {
        self.temp
            .path
            .as_deref()
            .expect("private export stage path")
    }

    pub(crate) fn create_auxiliary(&mut self, name: &OsStr) -> Result<File> {
        let file = self
            .directory
            .as_ref()
            .expect("open export staging directory")
            .create_new(name)
            .context("create private export auxiliary file")?;
        let retained = file
            .try_clone()
            .context("retain owned export auxiliary identity")?;
        self.auxiliaries.push((name.to_os_string(), retained));
        Ok(file)
    }

    pub(crate) fn remove_auxiliary(&mut self, name: &OsStr) -> Result<()> {
        let index = self
            .auxiliaries
            .iter()
            .position(|(retained, _)| retained == name)
            .context("export auxiliary identity was not retained")?;
        let (name, file) = self.auxiliaries.remove(index);
        let removed = self
            .directory
            .as_ref()
            .expect("open export staging directory")
            .remove_file(&name, file)
            .map_err(anyhow::Error::from)
            .context("remove checked export auxiliary file");
        if let Err(error) = removed {
            self.retain_on_drop = true;
            let retained = self.temp.keep();
            return Err(error.context(format!(
                "private export staging directory retained at {}",
                retained.display()
            )));
        }
        Ok(())
    }

    pub(crate) fn complete(&mut self) -> Result<()> {
        self.file_mut()
            .sync_all()
            .context("flush completed export staging file")
    }

    pub(crate) fn copy_stdout(mut self) -> Result<()> {
        let mut source = self
            .file
            .as_ref()
            .expect("open export staging payload")
            .try_clone()
            .context("retain completed export staging handle")?;
        source
            .seek(SeekFrom::Start(0))
            .context("rewind completed export staging handle")?;
        let output = std::io::copy(&mut source, &mut std::io::stdout())
            .map(|_| ())
            .context("write completed memory export to stdout");
        drop(source);
        let cleanup = self.cleanup();
        match (output, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(error)) => Err(error.context("clean completed export staging payload")),
            (Err(error), _) => Err(error),
        }
    }

    pub(crate) fn publish(mut self, target: OutputTarget) -> Result<()> {
        let policy = match target.publication() {
            Ok(policy) => policy,
            Err(primary) => {
                return match self.cleanup() {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(primary
                        .context(format!("private staging cleanup also failed: {cleanup:#}"))),
                };
            }
        };
        let result = target.directory.publish_file(
            self.directory
                .as_ref()
                .expect("open export staging directory"),
            OsStr::new(STAGED_PAYLOAD),
            self.file.as_ref().expect("open export staging payload"),
            &target.name,
            policy,
        );
        match result {
            Ok(()) => self.remove_empty_stage(),
            Err(error) if error.phase == PublicationPhase::Uncertain => {
                drop(self.file.take());
                drop(self.directory.take());
                let retained = self.temp.keep();
                Err(anyhow::Error::from(error).context(format!(
                    "export publication outcome is uncertain; staging directory retained at {} and the payload may already be published",
                    retained.display()
                )))
            }
            Err(error) => {
                let primary = anyhow::Error::from(error).context("export publication was rejected");
                match self.cleanup() {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(primary
                        .context(format!("private staging cleanup also failed: {cleanup:#}"))),
                }
            }
        }
    }

    fn cleanup(&mut self) -> Result<()> {
        let directory = self
            .directory
            .as_ref()
            .expect("open export staging directory");
        while let Some((name, file)) = self.auxiliaries.pop() {
            directory
                .remove_file(&name, file)
                .map_err(anyhow::Error::from)
                .context("remove checked export auxiliary file")?;
        }
        let file = self.file.take().expect("open export staging payload");
        directory
            .remove_file(OsStr::new(STAGED_PAYLOAD), file)
            .map_err(anyhow::Error::from)
            .context("remove checked export staging payload")?;
        self.remove_empty_stage()
    }

    fn remove_empty_stage(&mut self) -> Result<()> {
        drop(self.file.take());
        drop(self.directory.take());
        let path = self.temp.take_path();
        std::fs::remove_dir(&path).with_context(|| {
            format!(
                "remove empty private export staging directory {}",
                path.display()
            )
        })
    }
}

impl Drop for StagedExport {
    fn drop(&mut self) {
        if !self.retain_on_drop && self.file.is_some() && self.directory.is_some() {
            let _ = self.cleanup();
        }
    }
}

struct PrivateStage {
    path: Option<PathBuf>,
}

impl PrivateStage {
    fn new(parent: &Directory) -> Result<(Self, Directory)> {
        let name = Uuid::new_v4().to_string();
        let directory = parent
            .create_private_directory(OsStr::new(&name))
            .context("create private export staging directory")?;
        let path = directory.path().to_owned();
        Ok((Self { path: Some(path) }, directory))
    }

    fn keep(&mut self) -> PathBuf {
        self.path.take().expect("private export stage path")
    }

    fn take_path(&mut self) -> PathBuf {
        self.path.take().expect("private export stage path")
    }
}

impl Drop for PrivateStage {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_dir(path);
        }
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_session_spool_cleans_its_owned_stage_and_keeps_destination() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("session.jsonl");
        std::fs::write(&destination, b"existing completed export").unwrap();
        let target = OutputTarget::open_replace(&destination, temporary.path()).unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let mut staged = StagedExport::new(target.directory()).unwrap();
            let mut spool = staged
                .create_auxiliary(OsStr::new("session.spool"))
                .unwrap();
            spool.write_all(b"partial chronological source").unwrap();
            let stage_path = staged.temp.path.as_ref().unwrap().clone();
            ready_tx.send(stage_path).unwrap();
            let _ = release_rx.await;
            spool.sync_all().unwrap();
            staged.complete().unwrap();
        });
        let stage_path = ready_rx.await.unwrap();
        assert!(stage_path.join("session.spool").exists());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!stage_path.exists(), "cancelled spool left an orphan stage");
        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"existing completed export"
        );
    }

    #[test]
    fn selected_replacement_rejects_a_changed_destination_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("session.md");
        let displaced = temporary.path().join("selected-session.md");
        std::fs::write(&destination, b"selected destination").unwrap();
        let target = OutputTarget::open_replace(&destination, temporary.path()).unwrap();
        std::fs::rename(&destination, &displaced).unwrap();
        std::fs::write(&destination, b"unrelated replacement").unwrap();

        let mut staged = StagedExport::new(target.directory()).unwrap();
        staged.file_mut().write_all(b"completed export").unwrap();
        staged.complete().unwrap();
        let error = staged.publish(target).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("selected export destination changed before publication"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"unrelated replacement"
        );
        assert_eq!(std::fs::read(displaced).unwrap(), b"selected destination");
    }

    #[test]
    fn markdown_keeps_typed_content_structured_and_bumps_the_outer_format() {
        let record = StorageRecord::Message {
            sequence: 7,
            namespace: "project/transcript/session".into(),
            session_id: Some("session".into()),
            role: "assistant".into(),
            content_format: "typed-v1".into(),
            content: r#"{"blocks":[{"type":"tool_result","call_id":"c1","output":{"ok":true},"is_error":false}]}"#.into(),
        };
        let mut rendered = Vec::new();
        render_markdown_record(&mut rendered, &record).unwrap();
        let rendered = String::from_utf8(rendered).unwrap();
        assert!(rendered.contains("Content format: `typed-v1`"));
        assert!(rendered.contains("\"call_id\":\"c1\""));
        assert!(rendered.contains("\"output\":{\"ok\":true}"));
        assert_eq!(
            manifest(&ExportProvenance {
                project_scope: "scope".into(),
                branch: "main".into(),
                revision: "revision".into(),
                schema_version: 3,
                message_count: 1,
                state_count: 0,
                context_summary_count: 0,
                context_cursor_count: 0,
                session_catalog_count: 0,
                public_turn_count: 0,
            })["format"],
            4
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::io::Write;

    use kuru_platform::fs::PublicationError;

    use super::*;

    #[test]
    fn held_source_payload_retains_the_private_stage_after_uncertain_new_publication() {
        let temporary = tempfile::tempdir().unwrap();
        let parent = Directory::ensure_private(&temporary.path().join("exports")).unwrap();
        let destination = parent.path().join("committed.json");
        let target = OutputTarget {
            directory: Directory::open(parent.path(), Privacy::OwnerOnly, NameRetention::Pinned)
                .unwrap(),
            name: "committed.json".into(),
            expected: None,
        };
        let mut staged = StagedExport::new(&parent).unwrap();
        staged.file_mut().write_all(b"committed export").unwrap();
        staged.complete().unwrap();
        let stage_path = staged.temp.path.as_ref().unwrap().clone();
        let source = Directory::open(
            staged.directory.as_ref().unwrap().path(),
            Privacy::OwnerOnly,
            NameRetention::Pinned,
        )
        .unwrap();
        let held = source.read(OsStr::new(STAGED_PAYLOAD)).unwrap();

        let error = staged.publish(target).unwrap_err();
        let publication = error.downcast_ref::<PublicationError>().unwrap();
        assert_eq!(publication.phase, PublicationPhase::Uncertain);
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read(stage_path.join(STAGED_PAYLOAD)).unwrap(),
            b"committed export"
        );
        assert!(error.to_string().contains("staging directory retained at"));
        assert!(
            error
                .to_string()
                .contains(stage_path.to_string_lossy().as_ref())
        );

        drop(held);
        drop(source);
    }
}
