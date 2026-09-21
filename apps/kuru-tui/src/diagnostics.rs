use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::File,
    io::{Seek, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::Directory;
use serde_json::{Map, Value, json};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context as LayerContext, prelude::*, registry::LookupSpan};

const FILE_COUNT: usize = 4;
const FILE_BYTES: u64 = 64 * 1024;

#[derive(Clone)]
pub(crate) struct DiagnosticsGuard {
    ring: Arc<Ring>,
    directory: PathBuf,
}

impl DiagnosticsGuard {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn finish(self) -> Result<()> {
        self.ring.finish()
    }
}

pub(crate) fn install(
    data: &std::path::Path,
    scope: &str,
    debug: bool,
) -> Result<Option<DiagnosticsGuard>> {
    if cfg!(test) {
        return Ok(None);
    }
    let hash = scope
        .strip_prefix("project/")
        .context("invalid project diagnostic scope")?;
    ensure!(
        hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid project diagnostic scope"
    );
    let directory = Directory::ensure_private(&data.join("diagnostics").join(hash))
        .context("cannot create the private project diagnostics directory")?;
    let directory_path = directory.path().to_owned();
    let ring = Arc::new(Ring::open(directory)?);
    let layer = JsonLayer {
        ring: ring.clone(),
        debug,
        next_span: AtomicU64::new(1),
    };
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
        .context("cannot install project diagnostics for this process")?;
    Ok(Some(DiagnosticsGuard {
        ring,
        directory: directory_path,
    }))
}

/// Remove the bounded ring for one already-confirmed project purge. The CLI
/// holds that project's writer lease; this does not initialize diagnostics or
/// create a directory when no ring was ever written.
pub(crate) fn purge(data: &std::path::Path, scope: &str) -> Result<()> {
    let hash = scope
        .strip_prefix("project/")
        .context("invalid project diagnostic scope")?;
    ensure!(
        hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid project diagnostic scope"
    );
    let path = data.join("diagnostics").join(hash);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => Directory::open(
            &path,
            kuru_platform::fs::Privacy::OwnerOnly,
            kuru_platform::fs::NameRetention::Movable,
        )?
        .remove_tree()
        .map_err(anyhow::Error::from),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

struct Ring {
    state: Mutex<State>,
    failure: Mutex<Option<String>>,
}

struct State {
    directory: Option<Directory>,
    index: usize,
    file: Option<File>,
    bytes: u64,
}

impl Ring {
    fn open(directory: Directory) -> Result<Self> {
        let index = startup_slot(&directory)?;
        let (file, bytes) = open_slot(&directory, index)?;
        Ok(Self {
            state: Mutex::new(State {
                directory: Some(directory),
                index,
                file: Some(file),
                bytes,
            }),
            failure: Mutex::new(None),
        })
    }

    fn write(&self, value: Value) {
        let result = (|| -> Result<()> {
            let bytes = serde_json::to_vec(&value)?;
            ensure!(
                (bytes.len() as u64) < FILE_BYTES,
                "diagnostic record exceeds fixed file budget"
            );
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("diagnostic writer lock poisoned"))?;
            if state.directory.is_none() {
                return Ok(());
            }
            if state.bytes.saturating_add(bytes.len() as u64 + 1) > FILE_BYTES {
                let next = (state.index + 1) % FILE_COUNT;
                state.index = next;
                let directory = state.directory.as_ref().expect("checked above");
                let (file, bytes) = open_slot(directory, state.index)?;
                state.file = Some(file);
                state.bytes = bytes;
            }
            let index = state.index;
            let directory = state.directory.as_ref().expect("checked above");
            let file = state
                .file
                .as_ref()
                .context("project diagnostics are closed")?;
            let name = OsString::from(format!("trace-{index}.jsonl"));
            directory.verify(&name, file)?;
            let file = state.file.as_mut().expect("checked above");
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            state.bytes = state.bytes.saturating_add(bytes.len() as u64 + 1);
            Ok(())
        })();
        if let Err(error) = result {
            if let Ok(mut state) = self.state.lock() {
                state.file.take();
                state.directory.take();
            }
            if let Ok(mut failure) = self.failure.lock() {
                *failure = Some(format!("diagnostic write failed: {error}"));
            }
        }
    }

    fn finish(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("diagnostic writer lock poisoned"))?;
        let file = state.file.take();
        state.directory.take();
        if let Some(mut file) = file {
            file.flush().context("flush project diagnostics")?;
        }
        if let Some(error) = self
            .failure
            .lock()
            .map_err(|_| anyhow::anyhow!("diagnostic failure lock poisoned"))?
            .take()
        {
            anyhow::bail!("{error}");
        }
        Ok(())
    }
}

fn startup_slot(directory: &Directory) -> Result<usize> {
    let mut oldest = None;
    for index in 0..FILE_COUNT {
        let name = OsString::from(format!("trace-{index}.jsonl"));
        let file = match directory.read_write(&name) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(index),
            Err(error) => return Err(error.into()),
        };
        directory.verify(&name, &file)?;
        let modified = file.metadata()?.modified()?;
        if oldest.is_none_or(|(candidate, _)| modified < candidate) {
            oldest = Some((modified, index));
        }
    }
    oldest
        .map(|(_, index)| index)
        .context("diagnostic ring has no slots")
}

fn open_slot(directory: &Directory, index: usize) -> Result<(File, u64)> {
    let name = OsString::from(format!("trace-{index}.jsonl"));
    let mut file = match directory.create_new(&name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            directory.read_write(&name)?
        }
        Err(error) => return Err(error.into()),
    };
    directory.verify(&name, &file)?;
    file.set_len(0)?;
    file.rewind()?;
    Ok((file, 0))
}

struct JsonLayer {
    ring: Arc<Ring>,
    debug: bool,
    next_span: AtomicU64,
}

#[derive(Clone, Default)]
struct SafeFields(BTreeMap<String, Value>);

impl Visit for SafeFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert_string(field, value);
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert_value(field, json!(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert_value(field, json!(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert_value(field, json!(value));
    }
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}
impl SafeFields {
    fn allowed(field: &Field) -> bool {
        matches!(
            field.name(),
            "session"
                | "turn"
                | "actor"
                | "tool"
                | "operation"
                | "status"
                | "attempt"
                | "elapsed_ms"
                | "input_tokens"
                | "output_tokens"
                | "bytes"
                | "delay_ms"
                | "span"
                | "stage"
                | "digest"
                | "published"
                | "first_cause"
                | "os_error"
                | "attempts"
                | "receipt_error"
                | "collected"
                | "remaining"
                // Stream reconciliation shape: identity, kind, position and
                // text length only. Never model text or tool arguments.
                | "item_index"
                | "item_id"
                | "item_type"
                | "text_len"
        )
    }

    fn insert_string(&mut self, field: &Field, value: &str) {
        if !Self::allowed(field) {
            return;
        }
        self.0.insert(
            field.name().into(),
            Value::String(value.chars().take(256).collect()),
        );
    }

    fn insert_value(&mut self, field: &Field, value: Value) {
        if Self::allowed(field) {
            self.0.insert(field.name().into(), value);
        }
    }
}

impl<S> Layer<S> for JsonLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        ctx: LayerContext<'_, S>,
    ) {
        if !admitted_target(attrs.metadata()) {
            return;
        }
        let mut fields = SafeFields::default();
        attrs.record(&mut fields);
        fields.0.insert(
            "span".into(),
            json!(self.next_span.fetch_add(1, Ordering::Relaxed)),
        );
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(fields.clone());
        }
        if self.debug {
            self.record("span_open", attrs.metadata(), fields);
        }
    }
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _: LayerContext<'_, S>) -> bool {
        admitted_target(metadata)
    }

    fn on_event(&self, event: &Event<'_>, ctx: LayerContext<'_, S>) {
        if !admitted_target(event.metadata()) {
            return;
        }
        // Ordinary runs keep the ring to the informational record. Verbose
        // diagnostics, such as stream reconciliation shape, need `--debug`.
        if !self.debug && *event.metadata().level() > tracing::Level::INFO {
            return;
        }
        let mut fields = SafeFields::default();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(parent) = span.extensions().get::<SafeFields>() {
                    fields.0.extend(parent.0.clone());
                }
            }
        }
        event.record(&mut fields);
        self.record("event", event.metadata(), fields);
    }
    fn on_close(&self, id: tracing::Id, ctx: LayerContext<'_, S>) {
        if !self.debug {
            return;
        }
        if let Some(span) = ctx.span(&id)
            && admitted_target(span.metadata())
        {
            self.record(
                "span_close",
                span.metadata(),
                span.extensions()
                    .get::<SafeFields>()
                    .cloned()
                    .unwrap_or_default(),
            );
        }
    }
}
impl JsonLayer {
    fn record(
        &self,
        kind: &str,
        metadata: &'static tracing::Metadata<'static>,
        fields: SafeFields,
    ) {
        if !admitted_target(metadata) {
            return;
        }
        let mut row = Map::new();
        row.insert("kind".into(), json!(kind));
        row.insert("target".into(), json!(metadata.target()));
        row.insert("name".into(), json!(metadata.name()));
        if self.debug {
            row.insert("level".into(), json!(metadata.level().as_str()));
        }
        row.extend(fields.0);
        self.ring.write(Value::Object(row));
    }
}

fn admitted_target(metadata: &tracing::Metadata<'_>) -> bool {
    matches!(
        metadata.target(),
        "kuru.runtime" | "kuru.actor" | "kuru.tool" | "kuru.provider" | "kuru.memory"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_rotates_and_finish_disarms_retained_callbacks() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Ring::open(directory).unwrap();
        for index in 0..(FILE_COUNT + 2) {
            ring.write(json!({"kind":"event","target":"kuru.runtime","name":"turn","status":"ok","detail": "x".repeat(FILE_BYTES as usize / 2)}));
            let path = root.join(format!("trace-{}.jsonl", index % FILE_COUNT));
            assert!(path.exists());
        }
        let before = (0..FILE_COUNT)
            .map(|index| {
                std::fs::read(root.join(format!("trace-{index}.jsonl"))).unwrap_or_default()
            })
            .collect::<Vec<_>>();
        ring.finish().unwrap();
        ring.write(json!({"kind":"event","target":"kuru.runtime","name":"late"}));
        for (index, before) in before.iter().enumerate() {
            let path = root.join(format!("trace-{index}.jsonl"));
            if path.exists() {
                assert!(std::fs::metadata(&path).unwrap().len() <= FILE_BYTES);
            }
            assert_eq!(std::fs::read(path).unwrap_or_default(), before.as_slice());
        }
    }

    #[cfg(unix)]
    #[test]
    fn checked_ring_disarms_when_the_current_file_name_is_replaced() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Ring::open(directory).unwrap();
        ring.write(json!({"kind":"event","target":"kuru.runtime","status":"ok"}));
        let replacement = root.join("replacement");
        std::fs::write(&replacement, b"replacement").unwrap();
        std::fs::remove_file(root.join("trace-0.jsonl")).unwrap();
        std::fs::hard_link(&replacement, root.join("trace-0.jsonl")).unwrap();

        ring.write(json!({"kind":"event","target":"kuru.runtime","status":"ok"}));
        assert!(ring.finish().is_err());
        assert_eq!(std::fs::read(replacement).unwrap(), b"replacement");
    }

    #[test]
    fn restart_replaces_the_oldest_slot_and_retains_newest_records() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Ring::open(directory).unwrap();
        for index in 0..(FILE_COUNT + 2) {
            ring.write(json!({"kind":"event","target":"kuru.runtime","status":"ok","detail": "x".repeat(FILE_BYTES as usize / 2)}));
            std::thread::sleep(std::time::Duration::from_millis(2));
            assert!(
                root.join(format!("trace-{}", index % FILE_COUNT))
                    .with_extension("jsonl")
                    .exists()
            );
        }
        let newest_zero = std::fs::read(root.join("trace-0.jsonl")).unwrap();
        let newest_one = std::fs::read(root.join("trace-1.jsonl")).unwrap();
        ring.finish().unwrap();

        let reopened = Ring::open(Directory::ensure_private(&root).unwrap()).unwrap();
        reopened.write(json!({"kind":"event","target":"kuru.runtime","status":"ok"}));
        reopened.finish().unwrap();

        assert_eq!(
            std::fs::read(root.join("trace-0.jsonl")).unwrap(),
            newest_zero
        );
        assert_eq!(
            std::fs::read(root.join("trace-1.jsonl")).unwrap(),
            newest_one
        );
    }

    #[test]
    fn layer_rejects_foreign_and_unadmitted_secret_fields_across_rotation() {
        const SECRET: &str = "sk-proj-diagnostics-layer-secret-0123456789";
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Arc::new(Ring::open(directory).unwrap());
        let subscriber = tracing_subscriber::registry().with(JsonLayer {
            ring: ring.clone(),
            debug: true,
            next_span: AtomicU64::new(1),
        });
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "foreign.library", detail = %SECRET, "foreign payload");
            for _ in 0..3000 {
                tracing::info!(target: "kuru.provider", operation = "responses-completion", status = 503_u64, detail = %SECRET, "retry scheduled");
            }
        });
        ring.finish().unwrap();

        for index in 0..FILE_COUNT {
            let text = std::fs::read_to_string(root.join(format!("trace-{index}.jsonl"))).unwrap();
            assert!(!text.is_empty());
            assert!(!text.contains(SECRET));
            assert!(text.contains("responses-completion"));
        }
    }

    #[test]
    fn layer_inherits_safe_turn_fields_and_numeric_status() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Arc::new(Ring::open(directory).unwrap());
        let subscriber = tracing_subscriber::registry().with(JsonLayer {
            ring: ring.clone(),
            debug: true,
            next_span: AtomicU64::new(1),
        });
        tracing::subscriber::with_default(subscriber, || {
            let turn = tracing::info_span!(target: "kuru.runtime", "turn", session = "session-1", turn = "digest-1", operation = "turn");
            let _entered = turn.enter();
            tracing::info!(target: "kuru.provider", operation = "responses-completion", status = 503_u64, attempt = 2_u64, "retry scheduled");
        });
        ring.finish().unwrap();
        let records = (0..FILE_COUNT)
            .flat_map(|index| {
                std::fs::read_to_string(root.join(format!("trace-{index}.jsonl")))
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .map(|line| serde_json::from_str::<Value>(&line).unwrap())
            .collect::<Vec<_>>();
        let retry = records
            .iter()
            .find(|record| record["target"] == "kuru.provider")
            .unwrap();
        assert_eq!(retry["session"], "session-1");
        assert_eq!(retry["turn"], "digest-1");
        assert_eq!(retry["status"], 503);
        assert!(retry["span"].is_u64());
        assert!(
            records
                .iter()
                .all(|record| !record.to_string().contains("secret"))
        );
    }

    #[test]
    fn layer_admits_the_retained_install_stage_target_and_its_fixed_fields() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Arc::new(Ring::open(directory).unwrap());
        let subscriber = tracing_subscriber::registry().with(JsonLayer {
            ring: ring.clone(),
            debug: false,
            next_span: AtomicU64::new(1),
        });
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                target: "kuru.memory",
                stage = "/private/cache/versions/1.2.3/.install-Ab12Cd",
                digest = "deadbeef",
                published = true,
                first_cause = "Uncertain removal of a private install stage (os error 145)",
                os_error = 145_i64,
                attempts = 88_u64,
                elapsed_ms = 2003_u64,
                receipt_error = "write leftover receipt: permission denied",
                "retained private install stage after published engine"
            );
            tracing::warn!(
                target: "kuru.memory",
                collected = 0_u64,
                remaining = 8_u64,
                "retained private install stages reached their reporting cap"
            );
        });
        ring.finish().unwrap();
        let records = (0..FILE_COUNT)
            .flat_map(|index| {
                std::fs::read_to_string(root.join(format!("trace-{index}.jsonl")))
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .map(|line| serde_json::from_str::<Value>(&line).unwrap())
            .collect::<Vec<_>>();
        let retained = records
            .iter()
            .find(|record| record["target"] == "kuru.memory")
            .expect("the kuru.memory target must be admitted into the diagnostics ring");
        assert_eq!(
            retained["stage"],
            "/private/cache/versions/1.2.3/.install-Ab12Cd"
        );
        assert_eq!(retained["digest"], "deadbeef");
        assert_eq!(retained["published"], true);
        assert_eq!(
            retained["first_cause"], "Uncertain removal of a private install stage (os error 145)",
            "the ruling's required first-cause-with-OS-error detail must reach the ring"
        );
        assert_eq!(retained["os_error"], 145);
        assert_eq!(retained["attempts"], 88);
        assert_eq!(retained["elapsed_ms"], 2003);
        assert_eq!(
            retained["receipt_error"], "write leftover receipt: permission denied",
            "an unwritable receipt must be distinguishable in the ring"
        );
        let capped = records
            .iter()
            .find(|record| record.get("remaining").is_some())
            .expect("the leftover-stage cap record must be admitted too");
        assert_eq!(capped["collected"], 0);
        assert_eq!(capped["remaining"], 8);
        // No conversation, model, provider-request or memory-write field is
        // present: this record is a diagnostics-only row.
        assert!(retained.get("session").is_none());
        assert!(retained.get("turn").is_none());
    }

    #[test]
    fn normal_layer_keeps_parent_correlation_without_span_rows() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Arc::new(Ring::open(directory).unwrap());
        let subscriber = tracing_subscriber::registry().with(JsonLayer {
            ring: ring.clone(),
            debug: false,
            next_span: AtomicU64::new(1),
        });
        tracing::subscriber::with_default(subscriber, || {
            let turn = tracing::info_span!(target: "kuru.runtime", "turn", session = "session-2", turn = "digest-2");
            let _entered = turn.enter();
            tracing::info!(target: "kuru.provider", operation = "chatgpt-refresh", status = "not-dispatched", "refresh retry scheduled");
        });
        ring.finish().unwrap();
        let records = std::fs::read_to_string(root.join("trace-0.jsonl")).unwrap();
        assert!(records.contains("session-2") && records.contains("digest-2"));
        assert!(!records.contains("span_open") && !records.contains("span_close"));
    }

    #[test]
    fn debug_level_stream_reconcile_event_is_gated_by_the_debug_flag() {
        let emit = || {
            tracing::debug!(
                target: "kuru.provider",
                operation = "responses-stream-reconcile",
                stage = "authoritative",
                item_index = 0_u64,
                item_id = "item-1",
                item_type = "message",
                text_len = 7_u64,
                "reconciled output item"
            );
        };

        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Arc::new(Ring::open(directory).unwrap());
        let subscriber = tracing_subscriber::registry().with(JsonLayer {
            ring: ring.clone(),
            debug: false,
            next_span: AtomicU64::new(1),
        });
        tracing::subscriber::with_default(subscriber, emit);
        ring.finish().unwrap();
        let records = std::fs::read_to_string(root.join("trace-0.jsonl")).unwrap_or_default();
        assert!(
            !records.contains("responses-stream-reconcile"),
            "a debug-level event must be dropped when debug is disabled"
        );

        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("diagnostics")).unwrap();
        let root = directory.path().to_path_buf();
        let ring = Arc::new(Ring::open(directory).unwrap());
        let subscriber = tracing_subscriber::registry().with(JsonLayer {
            ring: ring.clone(),
            debug: true,
            next_span: AtomicU64::new(1),
        });
        tracing::subscriber::with_default(subscriber, emit);
        ring.finish().unwrap();
        let records = (0..FILE_COUNT)
            .flat_map(|index| {
                std::fs::read_to_string(root.join(format!("trace-{index}.jsonl")))
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .map(|line| serde_json::from_str::<Value>(&line).unwrap())
            .collect::<Vec<_>>();
        let reconciled = records
            .iter()
            .find(|record| record["operation"] == "responses-stream-reconcile")
            .expect("a debug-level event must be retained when debug is enabled");
        assert_eq!(reconciled["target"], "kuru.provider");
        assert_eq!(reconciled["stage"], "authoritative");
        assert_eq!(reconciled["item_index"], 0);
        assert_eq!(reconciled["item_id"], "item-1");
        assert_eq!(reconciled["item_type"], "message");
        assert_eq!(reconciled["text_len"], 7);
    }
}
