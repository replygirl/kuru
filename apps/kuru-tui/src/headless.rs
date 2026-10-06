//! One-shot input and bounded public delivery. Workers own stdio only.

use std::{
    future::Future,
    io::{self, IsTerminal, Read, Write},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use kuru_connectors::{is_permission_denied, project_text};
use kuru_runtime::{
    CancellationToken, ControlledTurnOutput, Event, FacingProgress, Harness, TurnInputMismatch,
    turn_was_cancelled,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, oneshot};

const INPUT_LIMIT: usize = 131_072;
const PIPE_LABEL: &str = "\n\nPiped input (literal data):\n";
const RECORD_LIMIT: usize = 256 * 1024;
const TEXT_LIMIT: usize = 32 * 1024;

/// One signal subscription survives input, setup, work and final delivery.
pub(crate) type RunSignal = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    StreamJson,
}

#[derive(Debug)]
pub(crate) struct RunExit(pub(crate) i32);

impl std::fmt::Display for RunExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "headless invocation exited with status {}", self.0)
    }
}
impl std::error::Error for RunExit {}

pub(crate) fn compose_input(
    argv: Option<&str>,
    terminal: bool,
    reader: impl Read,
) -> Result<String> {
    let mut piped = Vec::new();
    if !terminal {
        reader
            .take((INPUT_LIMIT + 1) as u64)
            .read_to_end(&mut piped)?;
        ensure!(
            piped.len() <= INPUT_LIMIT,
            "stdin exceeds the 131072-byte input limit"
        );
    }
    let piped = String::from_utf8(piped).context("stdin must be UTF-8")?;
    let prompt = match (argv, piped.is_empty()) {
        (Some(argv), true) => argv.to_owned(),
        (Some(argv), false) => format!("{argv}{PIPE_LABEL}{piped}"),
        (None, _) => piped,
    };
    ensure!(
        !prompt.trim().is_empty() && prompt.len() <= INPUT_LIMIT,
        "prompt must contain 1–131072 bytes; provide an argument or piped UTF-8 input"
    );
    Ok(prompt)
}

/// This runs before any project authority. The ordinary thread is not a Tokio
/// blocking task: interrupted stdin cannot hold runtime shutdown open.
pub(crate) async fn read_input(argv: Option<String>) -> Result<(String, RunSignal)> {
    let mut signal: RunSignal = Box::pin(crate::cli::ctrl_c_cancellation());
    // Register the same signal future that remains in select. A ready signal
    // has already completed and must never be polled again.
    if let std::task::Poll::Ready(result) = futures::poll!(&mut signal) {
        result?;
        return Err(RunExit(130).into());
    }
    if io::stdin().is_terminal() {
        return compose_input(argv.as_deref(), true, io::empty())
            .map(|prompt| (prompt, signal))
            .map_err(|error| anyhow::Error::new(RunExit(2)).context(error));
    }
    let (sender, receiver) = oneshot::channel();
    thread::Builder::new()
        .name("kuru-stdin".into())
        .spawn(move || {
            let result = compose_input(argv.as_deref(), false, io::stdin().lock());
            let _ = sender.send(result);
        })
        .context("start bounded stdin reader")?;
    #[cfg(feature = "test-support")]
    if std::env::var_os("KURU_HEADLESS_STDIN_READY_FIXTURE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        writeln!(io::stderr().lock(), "headless stdin signal ready")?;
    }
    let prompt = tokio::select! {
        biased;
        result = receiver => result.context("stdin reader stopped")?
            .map_err(|error| anyhow::Error::new(RunExit(2)).context(error)),
        signal = &mut signal => {
            signal?;
            return Err(RunExit(130).into());
        }
    }?;
    Ok((prompt, signal))
}

struct WriteRequest {
    bytes: Vec<u8>,
    reply: oneshot::Sender<io::Result<()>>,
}

/// At most one submitted write. No authority handle crosses into this worker.
struct OutputWriter {
    sender: Option<mpsc::SyncSender<WriteRequest>>,
    worker: Option<thread::JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
    acknowledgement: Option<oneshot::Receiver<io::Result<()>>>,
}

impl OutputWriter {
    fn new() -> Result<Self> {
        Self::with_output(io::stdout())
    }

    fn with_output(mut output: impl Write + Send + 'static) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<WriteRequest>(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let worker = thread::Builder::new()
            .name("kuru-stdout".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let result = output
                        .write_all(&request.bytes)
                        .and_then(|()| output.flush());
                    let failed = result.is_err();
                    let _ = request.reply.send(result);
                    if failed {
                        break;
                    }
                }
            })
            .context("start stdout writer")?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            stopped,
            acknowledgement: None,
        })
    }

    fn submit(&mut self, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            self.acknowledgement.is_none(),
            "stdout write is already pending"
        );
        let (reply, acknowledgement) = oneshot::channel();
        self.sender
            .as_ref()
            .context("stdout writer is closed")?
            .try_send(WriteRequest { bytes, reply })
            .map_err(|_| anyhow::anyhow!("stdout writer stopped"))?;
        self.acknowledgement = Some(acknowledgement);
        Ok(())
    }

    async fn acknowledge(&mut self) -> Result<()> {
        self.acknowledgement
            .as_mut()
            .context("no stdout write is pending")?
            .await
            .context("stdout writer stopped")?
            .map_err(|error| {
                if error.kind() == io::ErrorKind::BrokenPipe {
                    anyhow::Error::new(RunExit(141))
                } else {
                    error.into()
                }
            })?;
        self.acknowledgement = None;
        Ok(())
    }

    fn try_acknowledge(&mut self) -> Result<bool> {
        let Some(receiver) = &mut self.acknowledgement else {
            return Ok(true);
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return Ok(false),
            Err(oneshot::error::TryRecvError::Closed) => anyhow::bail!("stdout writer stopped"),
        };
        self.acknowledgement = None;
        result.map_err(|error| {
            if error.kind() == io::ErrorKind::BrokenPipe {
                anyhow::Error::new(RunExit(141))
            } else {
                error.into()
            }
        })?;
        Ok(true)
    }

    fn join_finished(&mut self) -> Result<()> {
        ensure!(self.acknowledgement.is_none(), "stdout is not acknowledged");
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("stdout writer panicked"))?;
        }
        Ok(())
    }
}

impl Drop for OutputWriter {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.sender.take();
        // A kernel-blocked stdout-only thread has no project authority. Do not
        // join it on interruption; process exit ends it after owned cleanup.
    }
}

fn public_text(input: &str, limit: usize) -> (String, bool) {
    let projected = project_text(input).unwrap_or_else(|_| "[unavailable]".into());
    let mut text = String::new();
    let mut truncated = false;
    for ch in projected
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
    {
        if text.len() + ch.len_utf8() > limit {
            truncated = true;
            break;
        }
        text.push(ch);
    }
    (text, truncated)
}

struct PublicRecord {
    kind: &'static str,
    detail: Value,
}

struct Encoder {
    turn_id: String,
    turn: String,
    seq: u64,
    status: Option<PublicRecord>,
    snapshot: Option<PublicRecord>,
    broadcast_omitted: u64,
    coalesced: u64,
    watch_loss_unknown: bool,
    last_progress: Option<(u32, u64)>,
    data_after_gap: bool,
    prefer_snapshot: bool,
}

impl Encoder {
    fn new(turn_id: &str) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"kuru.headless-turn.v1\0");
        digest.update(turn_id.as_bytes());
        let turn = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            turn_id: turn_id.into(),
            turn,
            seq: 0,
            status: None,
            snapshot: None,
            broadcast_omitted: 0,
            coalesced: 0,
            watch_loss_unknown: false,
            last_progress: None,
            data_after_gap: false,
            prefer_snapshot: false,
        }
    }

    fn encode(&mut self, record: PublicRecord) -> Result<Vec<u8>> {
        self.seq = self
            .seq
            .checked_add(1)
            .context("stream sequence overflow")?;
        let mut bytes = serde_json::to_vec(&json!({"version":1,"seq":self.seq,
            "kind":record.kind,"turn":self.turn,"detail":record.detail}))?;
        ensure!(
            bytes.len() < RECORD_LIMIT,
            "public stream record exceeded its bound"
        );
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn event(&mut self, event: Event) {
        let detail = match event {
            Event::Active { .. } => json!({"phase":"active"}),
            Event::Idle { .. } => json!({"phase":"idle"}),
            Event::SpeakerSelection { .. } => json!({"phase":"selecting"}),
            Event::ToolStarted { call_id, name, .. } => json!({"tool":"started",
                "call_id": public_text(&call_id, 256).0, "name":public_text(&name, 256).0}),
            Event::ToolSettled { observation, .. } => json!({"tool":"settled",
                "call_id":public_text(&observation.call_id,256).0,
                "name":public_text(&observation.name,256).0, "outcome":observation.outcome}),
            Event::Budget { reason, .. } => json!({"phase":"limited", "reason":reason}),
            Event::Response { .. } => json!({"phase":"response-ready"}),
            Event::Error { .. } => json!({"phase":"error"}),
            _ => return,
        };
        if self
            .status
            .replace(PublicRecord {
                kind: "status",
                detail,
            })
            .is_some()
        {
            self.coalesced = self.coalesced.saturating_add(1);
        }
    }

    fn progress(&mut self, progress: FacingProgress) {
        if progress.turn_id != self.turn_id {
            return;
        }
        let current = (progress.request_round, progress.seq);
        if self
            .last_progress
            .is_some_and(|previous| current <= previous)
        {
            return;
        }
        let (text, truncated) = public_text(&progress.text_tail, TEXT_LIMIT);
        self.last_progress = Some(current);
        // Watch is replaceable, not a delivery counter. Source sequence gaps
        // do not prove a number of omitted text tokens or snapshots.
        self.watch_loss_unknown = true;
        if self
            .snapshot
            .replace(PublicRecord {
                kind: "snapshot",
                detail: json!({
            "request_round":progress.request_round,"snapshot_seq":progress.seq,
            "text":text,"truncated":truncated || progress.text_truncated,
            "delivery":"replaceable-snapshot"}),
            })
            .is_some()
        {
            self.coalesced = self.coalesced.saturating_add(1);
        }
    }

    fn next(&mut self) -> Option<PublicRecord> {
        if std::mem::take(&mut self.data_after_gap)
            && let Some(record) = self.next_data()
        {
            return Some(record);
        }
        if self.broadcast_omitted > 0 || self.coalesced > 0 || self.watch_loss_unknown {
            self.data_after_gap = true;
            return Some(PublicRecord {
                kind: "gap",
                detail: json!({
                "broadcast_omitted":std::mem::take(&mut self.broadcast_omitted),
                "observed_coalesced":std::mem::take(&mut self.coalesced),
                "watch_loss":if std::mem::take(&mut self.watch_loss_unknown) {"unknown"} else {"not-observed"}}),
            });
        }
        self.next_data()
    }

    fn next_data(&mut self) -> Option<PublicRecord> {
        let record = if self.prefer_snapshot {
            self.snapshot.take().or_else(|| self.status.take())
        } else {
            self.status.take().or_else(|| self.snapshot.take())
        };
        if record.is_some() {
            self.prefer_snapshot = !self.prefer_snapshot;
        }
        record
    }
}

pub(crate) struct Delivery {
    pub(crate) result: Result<ControlledTurnOutput>,
    writer: OutputWriter,
    encoder: Option<Encoder>,
    json: bool,
    output_error: Option<anyhow::Error>,
    signalled: bool,
    cancellation: CancellationToken,
    signal: Option<RunSignal>,
}

enum Intake {
    Event(std::result::Result<Event, broadcast::error::RecvError>),
    Progress(std::result::Result<(), tokio::sync::watch::error::RecvError>),
}

/// Alternates which already-ready source is polled first. The outer drive
/// still gives controlled completion, cancellation and stdout ACK priority.
async fn intake(
    events: &mut broadcast::Receiver<Event>,
    progress: &mut tokio::sync::watch::Receiver<Option<FacingProgress>>,
    events_open: bool,
    progress_open: bool,
    prefer_progress: bool,
) -> Intake {
    if prefer_progress {
        tokio::select! {
            biased;
            changed = progress.changed(), if progress_open => Intake::Progress(changed),
            event = events.recv(), if events_open => Intake::Event(event),
        }
    } else {
        tokio::select! {
            biased;
            event = events.recv(), if events_open => Intake::Event(event),
            changed = progress.changed(), if progress_open => Intake::Progress(changed),
        }
    }
}

pub(crate) async fn drive(
    harness: &mut Harness,
    prompt: &str,
    turn_id: &str,
    json: bool,
    format: Option<OutputFormat>,
    mut signal: RunSignal,
) -> Result<Delivery> {
    let mut writer = OutputWriter::new()?;
    let mut encoder = format.map(|_| Encoder::new(turn_id));
    let cancellation = CancellationToken::new();
    // Setup may have queued SIGINT on the input-registered listener. Observe
    // it before admitting any user turn or starting its provider work.
    if let std::task::Poll::Ready(result) = futures::poll!(&mut signal) {
        cancellation.cancel();
        let signalled = result.is_ok();
        return Ok(Delivery {
            result: Err(RunExit(130).into()),
            writer,
            encoder,
            json,
            output_error: Some(result.err().unwrap_or_else(|| RunExit(130).into())),
            signalled,
            cancellation,
            signal: None,
        });
    }
    if let Some(encoder) = &mut encoder {
        writer.submit(encoder.encode(PublicRecord {
            kind: "started",
            detail: json!({
            "session":public_text(&harness.session.id,256).0}),
        })?)?;
    }
    let mut events = harness.subscribe();
    let mut progress = harness.subscribe_progress();
    let operation = harness.run_local_controlled(prompt, None, turn_id, &cancellation);
    tokio::pin!(operation);
    let mut output_error = None;
    let mut signalled = false;
    let mut signal_consumed = false;
    let mut events_open = encoder.is_some();
    let mut progress_open = encoder.is_some();
    let mut prefer_progress = false;
    let result = loop {
        if writer.acknowledgement.is_none()
            && let Some(encoder) = &mut encoder
            && let Some(record) = encoder.next()
        {
            match encoder
                .encode(record)
                .and_then(|bytes| writer.submit(bytes))
            {
                Ok(()) => {}
                Err(error) => {
                    output_error = Some(error);
                    cancellation.cancel();
                    break operation.await;
                }
            }
        }
        tokio::select! {
            biased;
            result = &mut operation => break result,
            signal = &mut signal => {
                signal_consumed = true;
                cancellation.cancel();
                let result = operation.await;
                match signal {
                    Ok(()) => {signalled = true; output_error = Some(RunExit(130).into());},
                    Err(error) => output_error = Some(error),
                }
                break result;
            },
            result = writer.acknowledge(), if writer.acknowledgement.is_some() => {
                if let Err(error) = result {output_error = Some(error); cancellation.cancel(); break operation.await;}
            },
            observation = intake(&mut events, &mut progress, events_open, progress_open, prefer_progress),
                if events_open || progress_open => {
                prefer_progress = !prefer_progress;
                match observation {
                    Intake::Event(event) => match event {
                    Ok(event) => encoder.as_mut().expect("stream encoder").event(event),
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        let encoder = encoder.as_mut().expect("stream encoder");
                        encoder.broadcast_omitted = encoder.broadcast_omitted.saturating_add(count);
                    },
                    Err(broadcast::error::RecvError::Closed) => events_open = false,
                    },
                    Intake::Progress(changed) => {
                if changed.is_err() {progress_open = false;}
                else if let Some(update) = progress.borrow_and_update().clone() {
                    encoder.as_mut().expect("stream encoder").progress(update);
                }
                    },
                }
            }
        }
    };
    Ok(Delivery {
        result,
        writer,
        encoder,
        json,
        output_error,
        signalled,
        cancellation: cancellation.clone(),
        signal: (!signal_consumed).then_some(signal),
    })
}

fn classify(error: &anyhow::Error) -> (&'static str, &'static str, i32) {
    if turn_was_cancelled(error) {
        ("cancelled", "unresolved", 130)
    } else if matches!(error.downcast_ref::<RunExit>(), Some(RunExit(130))) {
        ("cancelled", "no-new-admission", 130)
    } else if error.is::<TurnInputMismatch>() {
        ("rejected", "no-new-admission", 2)
    } else if is_permission_denied(error) {
        ("denied", "unresolved", 3)
    } else {
        ("failed", "unresolved", 1)
    }
}

impl Delivery {
    /// Keep exit dreaming and cleanup on the original operation token. A
    /// signal cannot turn a completed journal into a cancelled journal.
    pub(crate) async fn shutdown(&mut self, harness: &mut Harness, dream: bool) -> Result<()> {
        let shutdown = harness.shutdown_controlled(dream, &self.cancellation);
        tokio::pin!(shutdown);
        let cleanup = loop {
            tokio::select! {
                biased;
                result = &mut shutdown => break result,
                signal = async { self.signal.as_mut().expect("retained signal").await }, if self.signal.is_some() => {
                    self.signal = None;
                    self.cancellation.cancel();
                    let cleanup = shutdown.await;
                    if let Err(signal_error) = signal {
                        return crate::cli::finish(Err(signal_error), cleanup, "shutdown");
                    }
                    self.signalled = true;
                    self.output_error = Some(RunExit(130).into());
                    break cleanup;
                }
                result = self.writer.acknowledge(), if self.writer.acknowledgement.is_some() => {
                    if let Err(error) = result {
                        self.output_error = Some(error);
                        self.cancellation.cancel();
                        break shutdown.await;
                    }
                }
            }
        };
        match cleanup {
            Err(error) if self.cancellation.is_cancelled() && turn_was_cancelled(&error) => Ok(()),
            result => result,
        }
    }

    fn terminal(&self) -> PublicRecord {
        let detail = match &self.result {
            Ok(result) => {
                let (answer, truncated) = public_text(&result.output.text, TEXT_LIMIT);
                json!({"status":"completed","settlement":"completed", "session":public_text(&result.output.session,256).0,
                    "answer":answer,"truncated":truncated,"input_tokens":result.output.input_tokens,
                    "output_tokens":result.output.output_tokens,"limited":result.output.limited,
                    "limit_reasons":result.output.limit_reasons,"response_outcome":result.output.response_outcome,
                    "reused":result.reused})
            }
            Err(error) => {
                let (status, settlement, _) = classify(error);
                json!({"status":status,"settlement":settlement})
            }
        };
        PublicRecord {
            kind: "terminal",
            detail,
        }
    }

    /// Called only after memory/diagnostics and the writer lease are released.
    pub(crate) async fn finish(mut self, authority_cleanup: Result<()>) -> Result<()> {
        // A cleanup failure does not rewrite the durable result or allow a
        // terminal claiming a clean successful invocation.
        authority_cleanup?;
        if self.signalled {
            // Never wait on output after the signal's owned settlement. This
            // is one best-effort record, only after the previous ACK completed.
            if self.writer.try_acknowledge().unwrap_or(false) && self.encoder.is_some() {
                let terminal = self.terminal();
                if let Some(encoder) = &mut self.encoder {
                    let bytes = encoder.encode(terminal)?;
                    if self.writer.submit(bytes).is_ok() {
                        let _ = self.writer.try_acknowledge();
                    }
                }
            }
            return Err(RunExit(130).into());
        }
        if let Some(error) = self.output_error.take() {
            return Err(error);
        }
        // Each acknowledged write wins a simultaneous signal. A signal cannot
        // change the result/journal; it only interrupts this delivery.
        if self.writer.acknowledgement.is_some() {
            self.wait_write().await?;
        }
        if self.encoder.is_some() {
            while let Some(record) = self.encoder.as_mut().and_then(Encoder::next) {
                let bytes = self
                    .encoder
                    .as_mut()
                    .expect("stream encoder")
                    .encode(record)?;
                self.writer.submit(bytes)?;
                self.wait_write().await?;
            }
        }
        let final_bytes = if self.encoder.is_some() {
            let terminal = self.terminal();
            self.encoder
                .as_mut()
                .map(|encoder| encoder.encode(terminal))
                .transpose()?
        } else if let Ok(result) = &self.result {
            let text = if self.json {
                serde_json::to_string_pretty(&result.output)?
            } else {
                result.output.text.clone()
            };
            Some(format!("{text}\n").into_bytes())
        } else {
            None
        };
        if let Some(bytes) = final_bytes {
            self.writer.submit(bytes)?;
            self.wait_write().await?;
        }
        self.writer.join_finished()?;
        match self.result {
            Ok(_) => Ok(()),
            Err(error) => {
                let (_, _, code) = classify(&error);
                Err(anyhow::Error::new(RunExit(code)).context(error))
            }
        }
    }

    async fn wait_write(&mut self) -> Result<()> {
        tokio::select! {
            biased;
            result = self.writer.acknowledge() => result,
            signal = async { self.signal.as_mut().expect("retained signal").await }, if self.signal.is_some() => {
                self.signal = None;
                signal?;
                Err(RunExit(130).into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_input_precedence_and_utf8_bounds_are_exact() {
        struct TerminalReader;
        impl Read for TerminalReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("terminal stdin must never be read")
            }
        }
        assert_eq!(
            compose_input(Some("  argument\n"), true, TerminalReader).unwrap(),
            "  argument\n"
        );
        assert_eq!(
            compose_input(Some("argument"), false, io::empty()).unwrap(),
            "argument"
        );
        assert_eq!(
            compose_input(None, false, "猫\r\n".as_bytes()).unwrap(),
            "猫\r\n"
        );
        assert_eq!(
            compose_input(Some("instruction"), false, b"literal".as_slice()).unwrap(),
            format!("instruction{PIPE_LABEL}literal")
        );
        assert!(compose_input(None, true, TerminalReader).is_err());
        assert!(compose_input(None, false, b" \n".as_slice()).is_err());
        assert!(compose_input(None, false, [0xff].as_slice()).is_err());
        let exact = "x".repeat(INPUT_LIMIT);
        assert_eq!(
            compose_input(None, false, exact.as_bytes()).unwrap().len(),
            INPUT_LIMIT
        );
        assert!(compose_input(Some("instruction"), false, exact.as_bytes()).is_err());
        assert!(compose_input(None, false, "x".repeat(INPUT_LIMIT + 1).as_bytes()).is_err());
    }

    fn progress(turn: &str, round: u32, seq: u64, text: &str) -> FacingProgress {
        FacingProgress {
            turn_id: turn.into(),
            request_round: round,
            seq,
            text_tail: text.into(),
            text_truncated: false,
            summary_tail: "PRIVATE_SUMMARY_SENTINEL".into(),
            summary_truncated: false,
            activity: "PRIVATE_ACTIVITY_SENTINEL".into(),
            activity_truncated: false,
        }
    }

    #[test]
    fn headless_allowlist_withholds_private_payloads_and_preserves_round_identity() {
        let mut encoder = Encoder::new("exact");
        encoder.event(Event::Peer {
            actor: "PRIVATE_ACTOR".into(),
            envelope: json!({"body":"PRIVATE_PEER"}),
        });
        encoder.event(Event::State {
            actor: "PRIVATE_ACTOR".into(),
            report: kuru_runtime::StateReport {
                activation: 1.0,
                note: "PRIVATE_STATE".into(),
            },
        });
        encoder.event(Event::Error {
            actor: "PRIVATE_ACTOR".into(),
            detail: "PRIVATE_ERROR".into(),
        });
        encoder.progress(progress("foreign", 9, 99, "FOREIGN_TEXT"));
        encoder.progress(progress(
            "exact",
            2,
            7,
            "public\u{1b}[2J api_key=sk-test-1234567890123456",
        ));
        encoder.progress(progress("exact", 1, 99, "OLD_ROUND"));
        let mut records = Vec::new();
        while let Some(record) = encoder.next() {
            records.extend(encoder.encode(record).unwrap());
        }
        let text = String::from_utf8(records).unwrap();
        for excluded in [
            "PRIVATE_",
            "FOREIGN_TEXT",
            "OLD_ROUND",
            "sk-test-1234567890123456",
            "\\u001b",
        ] {
            assert!(!text.contains(excluded), "{excluded}");
        }
        let records: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let snapshot = records
            .iter()
            .find(|record| record["kind"] == "snapshot")
            .unwrap();
        assert_eq!(snapshot["detail"]["request_round"], 2);
        assert_eq!(snapshot["detail"]["snapshot_seq"], 7);
        assert_eq!(snapshot["detail"]["delivery"], "replaceable-snapshot");
        assert!(
            records
                .iter()
                .all(|record| record["turn"].as_str().unwrap().len() == 64)
        );
    }

    #[test]
    fn headless_gaps_remain_distinct_and_do_not_starve_latest_data() {
        let mut encoder = Encoder::new("turn");
        encoder.broadcast_omitted = 11;
        encoder.progress(progress("turn", 1, 1, "first"));
        encoder.progress(progress("turn", 1, 2, "latest"));
        let gap = encoder.next().unwrap();
        assert_eq!(gap.kind, "gap");
        assert_eq!(gap.detail["broadcast_omitted"], 11);
        assert_eq!(gap.detail["observed_coalesced"], 1);
        assert_eq!(gap.detail["watch_loss"], "unknown");
        // New watch changes while a gap is writing still permit data next.
        encoder.progress(progress("turn", 1, 3, "newest"));
        let next = encoder.next().unwrap();
        assert_eq!(next.kind, "snapshot");
        assert_eq!(next.detail["text"], "newest");
        encoder.event(Event::Active {
            actor: "private".into(),
            detail: "private".into(),
        });
        encoder.progress(progress("turn", 1, 4, "later"));
        let _ = encoder.next();
        let first = encoder.next().unwrap();
        let second = encoder.next().unwrap();
        assert_eq!(
            [first.kind, second.kind]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            ["snapshot", "status"].into_iter().collect()
        );
    }

    #[test]
    fn headless_text_sequence_and_record_bounds_account_for_json_expansion() {
        let mut encoder = Encoder::new("turn");
        encoder.progress(progress("turn", 1, 1, &"猫\n".repeat(TEXT_LIMIT)));
        let _ = encoder.next();
        let snapshot = encoder.next().unwrap();
        assert!(snapshot.detail["text"].as_str().unwrap().len() <= TEXT_LIMIT);
        assert_eq!(snapshot.detail["truncated"], true);
        assert!(encoder.encode(snapshot).unwrap().len() <= RECORD_LIMIT);
        encoder.seq = u64::MAX;
        assert!(
            encoder
                .encode(PublicRecord {
                    kind: "status",
                    detail: json!({})
                })
                .is_err()
        );
        let mismatch: anyhow::Error = TurnInputMismatch.into();
        assert_eq!(classify(&mismatch), ("rejected", "no-new-admission", 2));
        let generic = anyhow::anyhow!("permission denied cancelled mismatch");
        assert_eq!(classify(&generic), ("failed", "unresolved", 1));
    }

    #[tokio::test]
    async fn headless_intake_alternates_continuously_ready_events_and_snapshots() {
        kuru_memory::test_support::closing(async {
            let (events, mut received) = broadcast::channel(4);
            let (progress_sender, mut progress_received) = tokio::sync::watch::channel(None);
            let mut prefer_progress = false;
            for seq in 1..=64 {
                // Keep broadcast continuously ready, including real lag.
                for _ in 0..8 {
                    let _ = events.send(Event::Response {
                        actor: "private".into(),
                    });
                }
                progress_sender.send_replace(Some(progress("exact", 1, seq, "snapshot")));
                assert!(matches!(
                    intake(
                        &mut received,
                        &mut progress_received,
                        true,
                        true,
                        prefer_progress
                    )
                    .await,
                    Intake::Event(_)
                ));
                prefer_progress = !prefer_progress;
                assert!(matches!(
                    intake(
                        &mut received,
                        &mut progress_received,
                        true,
                        true,
                        prefer_progress
                    )
                    .await,
                    Intake::Progress(Ok(()))
                ));
                assert_eq!(
                    progress_received.borrow_and_update().as_ref().unwrap().seq,
                    seq
                );
                prefer_progress = !prefer_progress;
            }
        })
        .await;
    }

    #[tokio::test]
    async fn headless_stdout_ack_is_checked_and_blocking_is_outside_async_polling() {
        kuru_memory::test_support::closing(async {
            struct Blocked {
                started: Option<oneshot::Sender<()>>,
                release: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
            }
            impl Write for Blocked {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    if let Some(started) = self.started.take() {
                        let _ = started.send(());
                    }
                    let (lock, wake) = &*self.release;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = wake.wait(released).unwrap();
                    }
                    Ok(bytes.len())
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            struct Release(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
            impl Drop for Release {
                fn drop(&mut self) {
                    let (lock, wake) = &*self.0;
                    *lock.lock().unwrap() = true;
                    wake.notify_all();
                }
            }
            let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
            let guard = Release(release.clone());
            let (started, observed) = oneshot::channel();
            let mut writer = OutputWriter::with_output(Blocked {
                started: Some(started),
                release,
            })
            .unwrap();
            writer.submit(vec![b'x'; 1024]).unwrap();
            observed.await.unwrap();
            assert!(writer.submit(vec![b'y']).is_err());
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            // This is genuinely pollable while the OS writer is held.
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::task::yield_now().await;
                assert!(cancellation.is_cancelled());
            })
            .await
            .unwrap();
            drop(guard);
            writer.acknowledge().await.unwrap();
            writer.join_finished().unwrap();

            struct Closed;
            impl Write for Closed {
                fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                    Err(io::ErrorKind::BrokenPipe.into())
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            let mut writer = OutputWriter::with_output(Closed).unwrap();
            writer.submit(b"record\n".to_vec()).unwrap();
            let error = writer.acknowledge().await.unwrap_err();
            assert_eq!(error.downcast_ref::<RunExit>().unwrap().0, 141);
        })
        .await;
    }
}
