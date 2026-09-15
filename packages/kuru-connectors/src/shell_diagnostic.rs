//! Source-free diagnostics for built-in shell operational failures.

use anyhow::{Result, ensure};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{MAX_BYTES, redaction};

pub(crate) const DIAGNOSTIC_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellFailureCategory {
    TimedOut,
    Cancelled,
    CaptureFailed,
    OwnershipLost,
    CleanupUnconfirmed,
    OperationFailed,
}

impl ShellFailureCategory {
    fn text(self) -> &'static str {
        match self {
            Self::TimedOut => "shell timed out",
            Self::Cancelled => "shell cancelled",
            Self::CaptureFailed => "shell capture failed",
            Self::OwnershipLost => "shell ownership lost",
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
    text: Option<String>,
    eof: bool,
    #[cfg(test)]
    pub(crate) bytes: Vec<u8>,
}

impl ShellCapture {
    pub(crate) fn new() -> Self {
        Self {
            projection: Some(redaction::StreamingProjection::new(MAX_BYTES)),
            text: None,
            eof: false,
            #[cfg(test)]
            bytes: Vec::new(),
        }
    }

    pub(crate) async fn read(&mut self, reader: &mut (impl AsyncRead + Unpin)) -> Result<()> {
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
