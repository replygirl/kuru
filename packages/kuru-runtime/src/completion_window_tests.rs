//! Completion bounds shared by the turn and compaction model calls, measured
//! on tokio's paused test clock: every wait below is virtual time.

use std::{future::Future, pin::Pin, time::Duration};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{
    COMPLETION_TIMEOUT, Provider, ProviderEvent, ProviderSink, STREAM_IDLE_TIMEOUT, TextDeltaSource,
};
use kuru_core::{Completion, CompletionRequest, ContentBlock, ModelInfo, Usage};
use tokio::time::Instant;

use crate::actor::{COMPACTION_CALL, MODEL_CALL, bounded_completion};

/// The flat per-call bound both sites used before this change.
const PREVIOUS_FLAT_BOUND: Duration = Duration::from_secs(180);
const CALLS: [&str; 2] = [MODEL_CALL, COMPACTION_CALL];

fn delta(index: u64) -> ProviderEvent {
    ProviderEvent::TextDelta {
        item_id: "message".into(),
        output_index: 0,
        content_index: 0,
        source: TextDeltaSource::OutputText,
        text: format!("part {index} "),
    }
}

fn request() -> CompletionRequest {
    CompletionRequest {
        actor: "part".into(),
        instructions: String::new(),
        messages: vec![],
        current_message_count: None,
        context_budget: None,
        model: "fake".into(),
        effort: None,
        tools: vec![],
    }
}

fn completed() -> ProviderEvent {
    ProviderEvent::Completed(Completion {
        blocks: vec![ContentBlock::Text {
            text: "answer".into(),
        }],
        usage: Usage {
            input_tokens: Some(1),
            output_tokens: Some(1),
            cached_input_tokens: Some(0),
            reasoning_output_tokens: None,
        },
        stop_reason: None,
    })
}

/// Emits a delta every `interval`; after `deltas` of them it completes, and
/// with `None` it keeps streaming for as long as it is polled.
struct PacedProvider {
    interval: Duration,
    deltas: Option<u64>,
}

#[async_trait]
impl Provider for PacedProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let mut index = 0;
        while self.deltas.is_none_or(|deltas| index < deltas) {
            sink.emit(delta(index)).await?;
            index += 1;
            tokio::time::sleep(self.interval).await;
        }
        sink.emit(completed()).await
    }
}

/// Emits a delta, a second one after `pause`, then never another event.
struct FallsSilentProvider {
    pause: Duration,
}

#[async_trait]
impl Provider for FallsSilentProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(delta(0)).await?;
        tokio::time::sleep(self.pause).await;
        sink.emit(delta(1)).await?;
        std::future::pending().await
    }
}

#[derive(Default)]
struct Recording(Vec<ProviderEvent>);

impl ProviderSink for Recording {
    fn emit<'a>(
        &'a mut self,
        event: ProviderEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.0.push(event);
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test(start_paused = true)]
async fn a_stream_delivering_deltas_past_the_previous_flat_bound_completes() {
    kuru_memory::test_support::closing(async {
        let interval = STREAM_IDLE_TIMEOUT / 2;
        let deltas = 3;
        let streaming = interval * deltas;
        assert!(streaming > PREVIOUS_FLAT_BOUND && streaming < COMPLETION_TIMEOUT);
        for call in CALLS {
            let provider = PacedProvider {
                interval,
                deltas: Some(deltas.into()),
            };
            let mut observer = Recording::default();
            let start = Instant::now();
            let completion = bounded_completion(&provider, request(), &mut observer, call)
                .await
                .unwrap_or_else(|error| panic!("{call} failed while streaming: {error:#}"));
            assert_eq!(start.elapsed(), streaming, "{call}");
            assert_eq!(completion.text_projection(), "answer", "{call}");
            // The progress adapter forwards every event to the observer unchanged.
            let mut expected = (0..u64::from(deltas)).map(delta).collect::<Vec<_>>();
            expected.push(completed());
            assert_eq!(observer.0, expected, "{call}");
        }
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_provider_that_falls_silent_fails_one_window_after_its_last_event() {
    kuru_memory::test_support::closing(async {
        let pause = STREAM_IDLE_TIMEOUT / 3;
        for call in CALLS {
            let provider = FallsSilentProvider { pause };
            let mut observer = Recording::default();
            let start = Instant::now();
            let error = bounded_completion(&provider, request(), &mut observer, call)
                .await
                .unwrap_err();
            // The window re-arms on the second event rather than counting from
            // the start of the call.
            assert_eq!(start.elapsed(), pause + STREAM_IDLE_TIMEOUT, "{call}");
            assert_eq!(
                format!("{error:#}"),
                format!(
                    "{call}: no provider progress for {} s",
                    STREAM_IDLE_TIMEOUT.as_secs()
                )
            );
            assert_eq!(observer.0, vec![delta(0), delta(1)], "{call}");
        }
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_stream_that_never_settles_fails_at_the_total_completion_budget() {
    kuru_memory::test_support::closing(async {
        let interval = STREAM_IDLE_TIMEOUT / 2;
        for call in CALLS {
            let provider = PacedProvider {
                interval,
                deltas: None,
            };
            let mut observer = Recording::default();
            let start = Instant::now();
            let error = bounded_completion(&provider, request(), &mut observer, call)
                .await
                .unwrap_err();
            assert_eq!(start.elapsed(), COMPLETION_TIMEOUT, "{call}");
            assert_eq!(
                format!("{error:#}"),
                format!(
                    "{call} exceeded the {} s completion budget",
                    COMPLETION_TIMEOUT.as_secs()
                )
            );
            assert!(observer.0.len() > 1, "{call}");
        }
    })
    .await
}
