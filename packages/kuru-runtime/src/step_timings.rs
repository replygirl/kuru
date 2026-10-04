//! Test-only step timings for a wait that can expire on a slow runner.
//!
//! A recorder is shared by clone between a test, the harness it drives and the
//! actor work that harness sends. Each marked step stores its name and the
//! time since the recorder was created. Nothing prints while a test passes: a
//! test reads [`StepTimings::render`] only on its failure path. The default
//! recorder is disabled and marks nothing, so every other test is unchanged.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct StepTimings(Option<Arc<Recorded>>);

struct Recorded {
    start: Instant,
    steps: Mutex<Vec<(String, Duration)>>,
}

impl StepTimings {
    pub(crate) fn recording() -> Self {
        Self(Some(Arc::new(Recorded {
            start: Instant::now(),
            steps: Mutex::new(vec![]),
        })))
    }

    /// Record that `step` has just completed.
    pub(crate) fn mark(&self, step: impl Into<String>) {
        if let Some(recorded) = &self.0 {
            let elapsed = recorded.start.elapsed();
            if let Ok(mut steps) = recorded.steps.lock() {
                steps.push((step.into(), elapsed));
            }
        }
    }

    /// How many steps have been marked; a fixture wait reads a change as
    /// progress. A disabled recorder always reports zero.
    pub(crate) fn completed(&self) -> usize {
        self.0.as_ref().map_or(0, |recorded| {
            recorded
                .steps
                .lock()
                .map(|steps| steps.len())
                .unwrap_or_default()
        })
    }

    /// The most recently completed step and when it completed, for a stall
    /// report that names where an operation went silent.
    pub(crate) fn last(&self) -> String {
        let Some(recorded) = &self.0 else {
            return "step timings were not recorded".into();
        };
        recorded
            .steps
            .lock()
            .ok()
            .and_then(|steps| {
                steps
                    .last()
                    .map(|(step, elapsed)| format!("{step} at +{:.1} ms", millis(*elapsed)))
            })
            .unwrap_or_else(|| "(no step completed)".into())
    }

    /// One line per completed step, in completion order, and the time now.
    pub(crate) fn render(&self) -> String {
        let Some(recorded) = &self.0 else {
            return "step timings were not recorded".into();
        };
        let now = recorded.start.elapsed();
        let steps = recorded
            .steps
            .lock()
            .map(|steps| steps.clone())
            .unwrap_or_default();
        let mut lines = steps
            .iter()
            .map(|(step, elapsed)| format!("  +{:>8.1} ms  {step}", millis(*elapsed)))
            .collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push("  (no step completed)".into());
        }
        lines.push(format!("  +{:>8.1} ms  (time of this report)", millis(now)));
        lines.join("\n")
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
