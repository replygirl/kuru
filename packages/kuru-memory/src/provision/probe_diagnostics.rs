//! Gated facts about the cold probe. These observations never advance readiness.
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

pub(super) struct ProbeDiagnostics {
    start: Instant,
    enabled: bool,
}

impl ProbeDiagnostics {
    pub(super) fn new() -> Self {
        #[cfg(feature = "test-support")]
        let enabled = crate::service::fixture_startup_stages_enabled();
        #[cfg(not(feature = "test-support"))]
        let enabled = false;
        Self {
            start: Instant::now(),
            enabled,
        }
    }

    fn line(&self, phase: &'static str, facts: std::fmt::Arguments<'_>) -> Option<String> {
        self.enabled.then(|| {
            format!(
                "memory cold-probe phase={phase} elapsed-ms={} {facts}",
                self.start.elapsed().as_millis()
            )
        })
    }

    pub(super) fn phase(&self, phase: &'static str) {
        if let Some(line) = self.line(phase, format_args!("")) {
            eprintln!("{line}");
        }
    }

    pub(super) fn refusal(
        &self,
        child: &crate::engine::Child,
        stdout: &PipeFacts,
        stderr: &PipeFacts,
        timed_out: bool,
    ) {
        if self.enabled {
            let snapshot = child.probe_diagnostic();
            if let Some(line) = self.line(
                "refused",
                format_args!(
                    "timed-out={timed_out} stdout=[{stdout}] stderr=[{stderr}] {snapshot}"
                ),
            ) {
                eprintln!("{line}");
            }
        }
    }
}

#[derive(Default)]
pub(super) struct PipeFacts {
    bytes: AtomicU64,
    eof: AtomicBool,
}

impl PipeFacts {
    pub(super) fn read(&self, bytes: usize) {
        if bytes == 0 {
            self.eof.store(true, Ordering::Relaxed);
        } else {
            self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        }
    }
}

impl std::fmt::Display for PipeFacts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "bytes={} eof={}",
            self.bytes.load(Ordering::Relaxed),
            self.eof.load(Ordering::Relaxed)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn probe_pipe_facts_distinguish_partial_eof_and_bounded_refusal() {
        let (mut write, read) = tokio::io::duplex(8192);
        write.write_all(b"body").await.unwrap();
        let facts = PipeFacts::default();
        let drain = super::super::bounded_output(read, &facts);
        tokio::pin!(drain);
        assert!(futures::poll!(&mut drain).is_pending());
        assert_eq!(facts.to_string(), "bytes=4 eof=false");
        drop(write);
        assert_eq!(drain.await.unwrap(), b"body");
        assert_eq!(facts.to_string(), "bytes=4 eof=true");

        let (mut write, read) = tokio::io::duplex(8192);
        write
            .write_all(&vec![b'X'; (super::super::OUTPUT_LIMIT + 1) as usize])
            .await
            .unwrap();
        let facts = PipeFacts::default();
        let error = super::super::bounded_output(read, &facts)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Dolt version output exceeded its limit");
        assert_eq!(facts.to_string(), "bytes=4097 eof=false");
    }

    #[test]
    fn probe_diagnostics_gate_keeps_observations_separate_from_progress() {
        let mut observations = ProbeDiagnostics {
            start: Instant::now(),
            enabled: false,
        };
        assert!(observations.line("created", format_args!("")).is_none());
        observations.enabled = true;
        let (ticks, advances) = crate::progress::OpenTicks::new();
        ticks.advance();
        let facts = PipeFacts::default();
        facts.read(12);
        let line = observations
            .line("refused", format_args!("stdout=[{facts}]"))
            .unwrap();
        assert!(line.contains("phase=refused"));
        assert!(line.contains("stdout=[bytes=12 eof=false]"));
        assert!(line.len() < 256);
        assert_eq!(advances.borrow().count, 1);
        facts.read(0);
        assert_eq!(facts.to_string(), "bytes=12 eof=true");
    }
}
