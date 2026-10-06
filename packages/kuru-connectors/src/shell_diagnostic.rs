//! Source-free diagnostics for built-in shell operational failures.

use anyhow::{Result, ensure};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::Notify;

use crate::{MAX_BYTES, redaction};

pub const MAX_SHELL_PREVIEW_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug)]
pub struct ShellPreview {
    pub stream: ShellStream,
    pub text: String,
    pub truncated: bool,
}

/// Owned, bounded copies of the latest independently captured streams.
#[derive(Clone, Debug, Default)]
pub struct ShellPreviewSnapshot {
    pub stdout: Option<ShellPreview>,
    pub stderr: Option<ShellPreview>,
    pub sequence: u64,
    /// Updates omitted while a consumer copied a snapshot.
    pub omitted: u64,
}

#[derive(Default)]
struct PreviewState {
    stdout: Option<ShellPreview>,
    stderr: Option<ShellPreview>,
}

#[derive(Default)]
struct ProgressInner {
    latest: Mutex<PreviewState>,
    sequence: AtomicU64,
    omitted: AtomicU64,
    changed: Notify,
}

/// A tool-specific latest-preview handle. Pipe drainage never waits for a
/// consumer: contended updates are visibly omitted, and notifications coalesce.
/// Consumers receive owned copies, never a guard to retain during drawing.
#[derive(Clone, Default)]
pub struct ShellProgress(Arc<ProgressInner>);

impl ShellProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn publish(&self, preview: ShellPreview) {
        if let Ok(mut latest) = self.0.latest.try_lock() {
            match preview.stream {
                ShellStream::Stdout => latest.stdout = Some(preview),
                ShellStream::Stderr => latest.stderr = Some(preview),
            }
            self.0.sequence.fetch_add(1, Ordering::Release);
        } else {
            self.0.omitted.fetch_add(1, Ordering::Release);
            self.0.sequence.fetch_add(1, Ordering::Release);
        }
        self.0.changed.notify_one();
    }

    pub fn snapshot(&self) -> ShellPreviewSnapshot {
        let latest = self
            .0
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ShellPreviewSnapshot {
            stdout: latest.stdout.clone(),
            stderr: latest.stderr.clone(),
            sequence: self.0.sequence.load(Ordering::Acquire),
            omitted: self.0.omitted.load(Ordering::Acquire),
        }
    }

    /// Wait for a newer publication attempt. A single operation consumer owns
    /// this wait; any number of callers may take independent snapshots.
    pub async fn changed_after(&self, sequence: u64) -> ShellPreviewSnapshot {
        loop {
            let notified = self.0.changed.notified();
            let latest = self.snapshot();
            if latest.sequence != sequence {
                return latest;
            }
            notified.await;
        }
    }
}

pub(crate) const DIAGNOSTIC_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellFailureCategory {
    TimedOut,
    /// Reported only by the Unix process-group shell.
    #[cfg(unix)]
    Cancelled,
    CaptureFailed,
    /// Reported only by the Unix process-group shell.
    #[cfg(unix)]
    OwnershipLost,
    /// Reported only by the Unix process-group shell.
    #[cfg(unix)]
    CleanupUnconfirmed,
    OperationFailed,
}

impl ShellFailureCategory {
    fn text(self) -> &'static str {
        match self {
            Self::TimedOut => "shell timed out",
            #[cfg(unix)]
            Self::Cancelled => "shell cancelled",
            Self::CaptureFailed => "shell capture failed",
            #[cfg(unix)]
            Self::OwnershipLost => "shell ownership lost",
            #[cfg(unix)]
            Self::CleanupUnconfirmed => "shell cleanup unconfirmed",
            Self::OperationFailed => "shell operation failed",
        }
    }
}

/// A source-free public error.  Its sole field has already crossed the shell
/// diagnostic boundary, so alternate formatting and error-chain traversal have
/// no raw operation source to recover.
#[derive(Debug)]
pub(crate) struct ProjectedShellDiagnostic(pub(crate) String);

impl std::fmt::Display for ProjectedShellDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProjectedShellDiagnostic {}

pub(crate) fn failure(category: ShellFailureCategory, stderr: &ShellCapture) -> anyhow::Error {
    failure_with_cleanup(category, false, stderr)
}

pub(crate) fn failure_with_cleanup(
    category: ShellFailureCategory,
    cleanup_unconfirmed: bool,
    stderr: &ShellCapture,
) -> anyhow::Error {
    let cleanup = cleanup_unconfirmed.then_some("; cleanup: unconfirmed ownership retained");
    ProjectedShellDiagnostic(format!(
        "{}{}; stderr: {}",
        category.text(),
        cleanup.unwrap_or_default(),
        stderr.diagnostic()
    ))
    .into()
}

/// A bounded captured stream. Text is finalized only after EOF; operational
/// diagnostics deliberately withhold bytes from an incomplete pipe.
pub(crate) struct ShellCapture {
    projection: Option<redaction::StreamingProjection>,
    preview_projection: Option<redaction::StreamingProjection>,
    text: Option<String>,
    eof: bool,
    #[cfg(test)]
    pub(crate) bytes: Vec<u8>,
}

impl ShellCapture {
    pub(crate) fn new() -> Self {
        Self {
            projection: Some(redaction::StreamingProjection::new(MAX_BYTES)),
            preview_projection: None,
            text: None,
            eof: false,
            #[cfg(test)]
            bytes: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) async fn read(&mut self, reader: &mut (impl AsyncRead + Unpin)) -> Result<()> {
        self.read_with_progress(reader, None, ShellStream::Stdout)
            .await
    }

    pub(crate) async fn read_with_progress(
        &mut self,
        reader: &mut (impl AsyncRead + Unpin),
        progress: Option<&ShellProgress>,
        stream: ShellStream,
    ) -> Result<()> {
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer).await?;
            if count == 0 {
                self.eof = true;
                return self.finish();
            }
            self.projection
                .as_mut()
                .expect("shell capture was already finished")
                .push(&buffer[..count])?;
            if let Some(progress) = progress {
                let preview = self.preview_projection.get_or_insert_with(|| {
                    redaction::StreamingProjection::new(MAX_SHELL_PREVIEW_BYTES)
                });
                preview.push(&buffer[..count])?;
                let (text, truncated) = preview.preview();
                if !text.is_empty() {
                    progress.publish(ShellPreview {
                        stream,
                        text,
                        truncated,
                    });
                }
            }
            #[cfg(test)]
            self.bytes.extend_from_slice(&buffer[..count]);
        }
    }

    pub(crate) fn finish(&mut self) -> Result<()> {
        ensure!(self.eof, "cannot finish shell capture before EOF");
        if self.text.is_some() {
            return Ok(());
        }
        self.text = Some(
            self.projection
                .take()
                .expect("shell capture was already finished")
                .finish()?,
        );
        Ok(())
    }

    pub(crate) fn text(&self) -> &str {
        self.text
            .as_deref()
            .expect("shell capture was not finalized")
    }

    #[cfg(test)]
    pub(crate) fn eof(&self) -> bool {
        self.eof
    }

    fn diagnostic(&self) -> String {
        if !self.eof {
            return "<pending EOF>".to_owned();
        }
        self.text
            .as_deref()
            .map(|text| redaction::truncate_tool_output(text, DIAGNOSTIC_BYTES))
            .unwrap_or_else(|| "<unavailable>".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn safe_preview_coalesces_independent_streams_without_waiting_for_a_consumer() {
        let progress = ShellProgress::new();
        let preview = |stream, text: &str| ShellPreview {
            stream,
            text: text.to_owned(),
            truncated: false,
        };
        progress.publish(preview(ShellStream::Stderr, "stderr retained"));
        for index in 0..1000 {
            progress.publish(preview(ShellStream::Stdout, &format!("stdout {index}")));
        }
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.stdout.unwrap().text, "stdout 999");
        assert_eq!(snapshot.stderr.unwrap().text, "stderr retained");
        assert_eq!(snapshot.sequence, 1001);
        assert_eq!(snapshot.omitted, 0);
        // This same-thread publication would deadlock if pipe drainage ever
        // waited for the consumer's snapshot lock.
        {
            let _held = progress.0.latest.lock().unwrap();
            progress.publish(preview(ShellStream::Stdout, "omitted"));
        }
        let omitted = progress.changed_after(snapshot.sequence).await;
        assert_eq!(omitted.sequence, 1002);
        assert_eq!(omitted.omitted, 1);
        assert_eq!(omitted.stdout.unwrap().text, "stdout 999");
        progress.publish(preview(ShellStream::Stdout, "next safe update"));
        let next = progress.changed_after(omitted.sequence).await;
        assert_eq!(next.stdout.unwrap().text, "next safe update");
        assert_eq!(next.stderr.unwrap().text, "stderr retained");
    }

    #[tokio::test]
    async fn safe_partial_preview_does_not_finalize_incomplete_stderr() -> Result<()> {
        use tokio::io::AsyncWriteExt as _;
        let (mut reader, mut writer) = tokio::io::duplex(256);
        let progress = ShellProgress::new();
        let mut capture = ShellCapture::new();
        let first_sequence = progress.snapshot().sequence;
        writer.write_all(b"SAFE sk-proj-abcdefgh").await?;
        {
            let read =
                capture.read_with_progress(&mut reader, Some(&progress), ShellStream::Stderr);
            tokio::pin!(read);
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::select! {
                    result = &mut read => anyhow::bail!("read finished before EOF: {result:?}"),
                    result = progress.changed_after(first_sequence) => Ok::<_, anyhow::Error>(result),
                }
            }).await??;
        }
        let preview = progress.snapshot().stderr.expect("safe preview absent");
        assert_eq!(preview.stream, ShellStream::Stderr);
        assert!(preview.text.contains("SAFE") && !preview.text.contains("sk-proj-"));
        assert_eq!(capture.diagnostic(), "<pending EOF>");
        let second_sequence = progress.snapshot().sequence;
        writer.write_all(b"ijklmnop0123456789 end\n").await?;
        {
            let read =
                capture.read_with_progress(&mut reader, Some(&progress), ShellStream::Stderr);
            tokio::pin!(read);
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::select! {
                    result = &mut read => anyhow::bail!("read finished before EOF: {result:?}"),
                    result = progress.changed_after(second_sequence) => Ok::<_, anyhow::Error>(result),
                }
            }).await??;
        }
        let preview = progress.snapshot().stderr.expect("second preview absent");
        assert!(
            preview.text.contains("[REDACTED:recognized-secret]")
                && !preview.text.contains("sk-proj-")
        );
        assert!(preview.text.len() <= MAX_SHELL_PREVIEW_BYTES);
        assert_eq!(capture.diagnostic(), "<pending EOF>");
        writer.shutdown().await?;
        capture
            .read_with_progress(&mut reader, Some(&progress), ShellStream::Stderr)
            .await?;
        assert!(capture.eof() && capture.text().contains("end"));
        assert!(!capture.text().contains("sk-proj-"));
        Ok(())
    }

    #[test]
    fn diagnostic_never_finalizes_partial_stderr() {
        let capture = ShellCapture::new();
        let error = failure(ShellFailureCategory::TimedOut, &capture);
        assert_eq!(error.to_string(), "shell timed out; stderr: <pending EOF>");
        assert_eq!(error.chain().count(), 1);
    }

    #[tokio::test]
    async fn eof_finalizes_before_another_stream_or_process_completes() {
        let mut source = b"useful sk-abcdefghijklmnop detail".as_slice();
        let mut capture = ShellCapture::new();
        capture.read(&mut source).await.unwrap();
        let error = failure(ShellFailureCategory::TimedOut, &capture);
        assert_eq!(
            error.to_string(),
            "shell timed out; stderr: useful [REDACTED:recognized-secret] detail"
        );
        assert_eq!(error.chain().count(), 1);
    }
}
