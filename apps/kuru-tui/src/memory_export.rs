use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use kuru_memory::{ActiveExportSnapshot, ExportProvenance, StorageRecord};
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication, PublicationPhase};
use uuid::Uuid;

use crate::cli::ExportFormat;

const FORMAT_VERSION: u32 = 2;
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
    snapshot.verify_counts(counts.messages, counts.state)?;
    staged.complete()?;
    match target {
        Some(target) => staged.publish(target),
        None => staged.copy_stdout(),
    }
}

struct OutputTarget {
    directory: Directory,
    name: OsString,
}

impl OutputTarget {
    fn open(output: &Path, cwd: &Path) -> Result<Self> {
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
        Ok(Self {
            directory: Directory::open(&parent, Privacy::Inherited, NameRetention::Pinned)
                .context("open export output directory")?,
            name: name.to_owned(),
        })
    }
}

#[derive(Default)]
struct Counts {
    messages: u64,
    state: u64,
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
                    "role": role,
                    "content_format": content_format,
                    "content": content,
                }),
            )?;
            writeln!(writer, "\n```")?;
        }
        StorageRecord::State { .. } => {
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

struct StagedExport {
    // Field order closes the payload and checked directory before Drop removes it.
    file: Option<File>,
    directory: Option<Directory>,
    temp: PrivateStage,
}

impl StagedExport {
    fn new(parent: &Directory) -> Result<Self> {
        let (temp, directory) = PrivateStage::new(parent)?;
        let file = directory
            .create_new(OsStr::new(STAGED_PAYLOAD))
            .context("create export staging payload")?;
        Ok(Self {
            file: Some(file),
            directory: Some(directory),
            temp,
        })
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("open export staging payload")
    }

    fn complete(&mut self) -> Result<()> {
        self.file_mut()
            .sync_all()
            .context("flush completed export staging file")
    }

    fn copy_stdout(mut self) -> Result<()> {
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

    fn publish(mut self, target: OutputTarget) -> Result<()> {
        let result = target.directory.publish_file(
            self.directory
                .as_ref()
                .expect("open export staging directory"),
            OsStr::new(STAGED_PAYLOAD),
            self.file.as_ref().expect("open export staging payload"),
            &target.name,
            Publication::New,
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
        if self.file.is_some() && self.directory.is_some() {
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

    #[test]
    fn markdown_keeps_typed_content_structured_and_bumps_the_outer_format() {
        let record = StorageRecord::Message {
            sequence: 7,
            namespace: "project/transcript/session".into(),
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
            })["format"],
            2
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
