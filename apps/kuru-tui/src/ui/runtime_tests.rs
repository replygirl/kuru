use std::{
    collections::VecDeque,
    convert::Infallible,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    thread,
    time::Duration,
};

use futures::{Stream, stream};
use kuru_connectors::{DemoProvider, Provider, ProviderEvent, ProviderSink};
use kuru_core::{Completion, CompletionRequest, Config, Mode, ModelInfo};
use kuru_memory::{MemoryStore, StorageRecord, test_support::open_options};
use kuru_runtime::{CancellationToken, ControlledTurnOutput, Harness, ResponseOutcome, TurnOutput};
use ratatui::{
    Terminal,
    backend::{Backend, ClearType, TestBackend, WindowSize},
    buffer::Cell,
    layout::{Constraint, Layout, Position, Size},
};
use tokio::{
    sync::{Notify, broadcast, mpsc},
    time::Instant as TokioInstant,
};

use super::{
    ACTIVITY_DRAIN_CAP, CompletionState, DispatchOutcome, Scheduler, TerminalEvent, View, Wake,
    WakeAvailability, apply_completion, cancel_operation, dispatch, next_wake,
    project_initial_view, project_runtime_snapshot, run_loop_with_stream,
    run_loop_with_stream_and_notice,
};
use crate::memory_notice::MemoryNotice;

async fn fixture() -> (tempfile::TempDir, Harness, Vec<ModelInfo>) {
    fixture_with_memory(temporary_memory().await).await
}

/// Held across the supervisor and engine spawns; see `crate::spawn_gate`.
async fn temporary_memory() -> MemoryStore {
    let _gate = crate::spawn_gate::spawning().await;
    MemoryStore::temporary().await.unwrap()
}

async fn fixture_with_memory(memory: MemoryStore) -> (tempfile::TempDir, Harness, Vec<ModelInfo>) {
    harness_with_provider(memory, Arc::new(DemoProvider)).await
}

async fn harness_with_provider(
    memory: MemoryStore,
    provider: Arc<dyn Provider>,
) -> (tempfile::TempDir, Harness, Vec<ModelInfo>) {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new(
        Config {
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        directory.path(),
        memory,
        provider,
        None,
    )
    .await
    .unwrap();
    let models = vec![ModelInfo {
        id: "demo".into(),
        name: "Demo".into(),
        efforts: vec!["low".into(), "high".into()],
        default_effort: Some("low".into()),
        metadata: Default::default(),
    }];
    (directory, harness, models)
}

struct CapturingProvider {
    requests: Mutex<Vec<CompletionRequest>>,
    started: Notify,
}

impl CapturingProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(vec![]),
            started: Notify::new(),
        })
    }

    async fn wait_for_request(&self) {
        let notified = self.started.notified();
        if self.requests.lock().unwrap().is_empty() {
            tokio::time::timeout(Duration::from_secs(10), notified)
                .await
                .expect("provider did not receive the TUI turn");
        }
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Provider for CapturingProvider {
    async fn models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        sink: &mut dyn ProviderSink,
    ) -> anyhow::Result<()> {
        self.requests.lock().unwrap().push(request);
        self.started.notify_waiters();
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "captured response",
            vec![],
            0,
            0,
        )))
        .await
    }
}

fn command_text(outcome: DispatchOutcome) -> String {
    match outcome {
        DispatchOutcome::Command(text) => text,
        DispatchOutcome::Turn(_) => panic!("expected command feedback"),
    }
}

fn seed_stale_runtime(view: &mut View) {
    view.turns = usize::MAX;
    view.mode = "stale-mode".into();
    view.model = "stale-model".into();
    view.effort = "stale-effort".into();
    view.parts = vec![("stale-part".into(), "Stale · role".into())];
    view.relationships.clear();
    view.focus = Some("stale-focus".into());
    view.part_activity
        .insert("stale-part".into(), "active".into());
    view.routes.push(("stale-part".into(), "missing".into()));
}

async fn assert_runtime_projection(view: &View, harness: &Arc<tokio::sync::Mutex<Harness>>) {
    let runtime = {
        let harness = harness.lock().await;
        project_runtime_snapshot(&harness)
    };
    assert_eq!(view.turns, runtime.turns);
    assert_eq!(view.mode, runtime.mode);
    assert_eq!(view.model, runtime.model);
    assert_eq!(view.effort, runtime.effort);
    assert_eq!(view.parts, runtime.parts);
    assert_eq!(view.relationships, runtime.relationships);
    assert_eq!(view.focus, runtime.focus);
    assert!(view.part_activity.is_empty());
    assert!(view.routes.is_empty());
}

#[tokio::test]
async fn slash_commands_change_real_runtime_state_and_validate_errors() {
    let (_dir, mut h, models) = fixture().await;
    assert!(command_text(dispatch(&mut h, &models, "/parts").await.unwrap()).contains("manager"));
    dispatch(&mut h, &models, "/mode freudian").await.unwrap();
    assert_eq!(h.config.mode, Mode::Freudian);
    dispatch(&mut h, &models, "/model demo").await.unwrap();
    assert_eq!(h.config.effort.as_deref(), Some("low"));
    dispatch(&mut h, &models, "/effort high").await.unwrap();
    assert_eq!(h.config.effort.as_deref(), Some("high"));
    assert!(
        dispatch(&mut h, &models, "/effort impossible")
            .await
            .is_err()
    );
    dispatch(&mut h, &models, "/effort default").await.unwrap();
    assert!(h.config.effort.is_none());
    let ids = h.topology.parts[..2]
        .iter()
        .map(|p| p.id.clone())
        .collect::<Vec<_>>();
    dispatch(&mut h, &models, &format!("/focus {}", ids[0]))
        .await
        .unwrap();
    assert!(h.topology.focus.is_some());
    dispatch(&mut h, &models, "/focus auto").await.unwrap();
    assert!(h.topology.focus.is_none());
    dispatch(
        &mut h,
        &models,
        &format!("/relate alliance {},{}", ids[0], ids[1]),
    )
    .await
    .unwrap();
    assert_eq!(h.topology.relationships.len(), 1);
    let first = match dispatch(&mut h, &models, "hello").await.unwrap() {
        DispatchOutcome::Turn(result) => result,
        DispatchOutcome::Command(_) => panic!("ordinary input did not run a turn"),
    };
    assert!(!first.reused);
    let usage_before_retry = h.session_usage().await.unwrap();
    let cost = command_text(dispatch(&mut h, &models, "/cost").await.unwrap());
    assert!(cost.contains(&format!(
        "{} provider invocations",
        usage_before_retry.invocation_count
    )));
    assert!(cost.contains("API-equivalent estimate (not a subscription charge)"));
    let history = h.history().await.unwrap();
    let retry = match dispatch(&mut h, &models, "/retry").await.unwrap() {
        DispatchOutcome::Turn(result) => result,
        DispatchOutcome::Command(_) => panic!("retry did not return a turn"),
    };
    assert!(retry.reused);
    assert_eq!(retry.output.text, first.output.text);
    assert_eq!(h.history().await.unwrap(), history);
    assert_eq!(h.session_usage().await.unwrap(), usage_before_retry);
    assert!(
        command_text(
            dispatch(&mut h, &models, &format!("/memory {}", ids[0]))
                .await
                .unwrap()
        )
        .contains("hello")
    );
    assert!(command_text(dispatch(&mut h, &models, "/dream").await.unwrap()).contains("summaries"));
    let notes: serde_json::Value = serde_json::from_str(&command_text(
        dispatch(&mut h, &models, &format!("/notes {}", ids[0]))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(notes["identity"], ids[0]);
    assert_eq!(notes["requested_limit"], 100);
    assert_eq!(notes["truncated"], false);
    assert!(notes["notes"].as_array().is_some_and(|notes| {
        notes.iter().any(|note| {
            note["sequence"].as_i64().is_some()
                && note["content"] == "[demo] Offline demo memory consolidation."
        })
    }));
    h.apply_dream(vec![kuru_runtime::DreamProposal::Add {
        name: "Extra".into(),
        role: h.topology.parts[0].role.clone(),
        instruction: "Complement".into(),
    }])
    .await
    .unwrap();
    dispatch(&mut h, &models, "/undo-dream").await.unwrap();
    let status: serde_json::Value = serde_json::from_str(&command_text(
        dispatch(&mut h, &models, "/memory-status").await.unwrap(),
    ))
    .unwrap();
    assert_eq!(status["engine"], "dolt");
    let revisions: serde_json::Value = serde_json::from_str(&command_text(
        dispatch(&mut h, &models, "/memory-history").await.unwrap(),
    ))
    .unwrap();
    assert!(
        revisions
            .as_array()
            .is_some_and(|revisions| !revisions.is_empty())
    );
    for bad in [
        "/unknown",
        "/relate",
        "/mode unknown",
        "/model",
        "/effort",
        "/memory missing",
    ] {
        assert!(dispatch(&mut h, &models, bad).await.is_err(), "{bad}");
    }
}

#[tokio::test]
async fn retry_without_a_local_submission_is_narrowly_rejected() {
    let (_dir, mut harness, models) = fixture().await;
    let error = dispatch(&mut harness, &models, "/retry").await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("this session has no local submission to retry")
    );
    assert!(harness.history().await.unwrap().is_empty());
}

struct BlockingProvider {
    started: AtomicUsize,
    dropped: AtomicUsize,
    wake: Notify,
}

impl BlockingProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            started: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
            wake: Notify::new(),
        })
    }

    async fn wait_started(&self) -> io::Result<()> {
        let notified = self.wake.notified();
        if self.started.load(Ordering::SeqCst) > 0 {
            return Ok(());
        }
        tokio::time::timeout(Duration::from_secs(5), notified)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "provider did not start"))?;
        if self.started.load(Ordering::SeqCst) == 0 {
            return Err(io::Error::other("provider start notification was spurious"));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Provider for BlockingProvider {
    async fn models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, _: CompletionRequest, _: &mut dyn ProviderSink) -> anyhow::Result<()> {
        struct Dropped<'a>(&'a AtomicUsize);
        impl Drop for Dropped<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let _dropped = Dropped(&self.dropped);
        self.started.fetch_add(1, Ordering::SeqCst);
        self.wake.notify_one();
        std::future::pending::<anyhow::Result<()>>().await
    }
}

async fn blocking_fixture(provider: Arc<BlockingProvider>) -> (tempfile::TempDir, Harness) {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new(
        Config {
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        directory.path(),
        temporary_memory().await,
        provider,
        None,
    )
    .await
    .unwrap();
    (directory, harness)
}

#[derive(Debug, Clone, Copy)]
enum Failure {
    Eof,
    Read,
    Draw,
}

fn controlled_input(
    provider: Arc<BlockingProvider>,
    failure: Failure,
) -> impl Stream<Item = io::Result<TerminalEvent>> + Unpin {
    let mut events = VecDeque::from([
        TerminalEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('s'),
            crossterm::event::KeyModifiers::NONE,
        )),
        TerminalEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        )),
    ]);
    Box::pin(stream::unfold(
        (provider, failure, false),
        move |(provider, failure, released)| {
            let event = events.pop_front();
            async move {
                if let Some(event) = event {
                    return Some((Ok(event), (provider, failure, released)));
                }
                if released {
                    return None;
                }
                if let Err(error) = provider.wait_started().await {
                    return Some((Err(error), (provider, failure, true)));
                }
                match failure {
                    Failure::Eof => None,
                    Failure::Read => Some((
                        Err(io::Error::other("injected terminal read failure")),
                        (provider, failure, true),
                    )),
                    Failure::Draw => {
                        Some((Ok(TerminalEvent::Resize(80, 24)), (provider, failure, true)))
                    }
                }
            }
        },
    ))
}

struct FailingBackend {
    inner: TestBackend,
    provider: Arc<BlockingProvider>,
    fail_initial_draw: bool,
}

impl FailingBackend {
    fn new(provider: Arc<BlockingProvider>) -> Self {
        Self {
            inner: TestBackend::new(80, 24),
            provider,
            fail_initial_draw: false,
        }
    }

    fn initially_failing(provider: Arc<BlockingProvider>) -> Self {
        Self {
            inner: TestBackend::new(80, 24),
            provider,
            fail_initial_draw: true,
        }
    }
}

fn backend_result<T>(result: std::result::Result<T, Infallible>) -> io::Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(never) => match never {},
    }
}

impl Backend for FailingBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        if self.fail_initial_draw || self.provider.started.load(Ordering::SeqCst) > 0 {
            return Err(io::Error::other("injected backend failure"));
        }
        backend_result(self.inner.draw(content))
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        backend_result(self.inner.hide_cursor())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        backend_result(self.inner.show_cursor())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        backend_result(self.inner.get_cursor_position())
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        backend_result(self.inner.set_cursor_position(position))
    }

    fn clear(&mut self) -> io::Result<()> {
        backend_result(self.inner.clear())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        backend_result(self.inner.clear_region(clear_type))
    }

    fn size(&self) -> io::Result<Size> {
        backend_result(self.inner.size())
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        backend_result(self.inner.window_size())
    }

    fn flush(&mut self) -> io::Result<()> {
        backend_result(self.inner.flush())
    }
}

async fn persistent_store() -> (tempfile::TempDir, kuru_memory::OpenOptions, MemoryStore) {
    let root = tempfile::tempdir().unwrap();
    let options = open_options(
        root.path().join("data"),
        format!("project/{}", "1".repeat(64)),
    )
    .unwrap();
    let store = {
        // Held across the supervisor and engine spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        MemoryStore::open(options.clone()).await.unwrap()
    };
    (root, options, store)
}

async fn reopen_store(options: kuru_memory::OpenOptions) -> MemoryStore {
    // Held across the supervisor and engine spawns; see `crate::spawn_gate`.
    let _gate = crate::spawn_gate::spawning().await;
    MemoryStore::open(options).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_run_notice_is_drawn_before_input_then_persisted_outside_harness_history() {
    let (data_root, options, store) = persistent_store().await;
    let notice = MemoryNotice::pending(store.clone()).await.unwrap().unwrap();
    let (project, harness, models) = fixture_with_memory(store.clone()).await;
    assert!(harness.history().await.unwrap().is_empty());

    let (input_tx, input_rx) = mpsc::channel(1);
    let input = Box::pin(stream::unfold(input_rx, |mut input_rx| async move {
        input_rx.recv().await.map(|event| (Ok(event), input_rx))
    }));
    let loop_task = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
        let result =
            run_loop_with_stream_and_notice(&mut terminal, harness, models, input, Some(notice))
                .await;
        let buffer = terminal.backend().buffer();
        let screen = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        let rows = Layout::vertical([
            Constraint::Length(if buffer.area.height >= 16 { 2 } else { 1 }),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(5), // Empty editor: two input rows, one control row, and borders.
            Constraint::Length(1),
        ])
        .split(buffer.area);
        let transcript =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(34)]).split(rows[1])[0];
        let transcript_text = (transcript.y..transcript.bottom())
            .map(|y| {
                (transcript.x..transcript.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        (result, screen, transcript_text)
    });

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if MemoryNotice::pending(store.clone())
                .await
                .unwrap()
                .is_none()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("notice was not recorded after a completed initial frame");
    input_tx
        .send(TerminalEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        )))
        .await
        .unwrap();
    let (result, screen, transcript_text) =
        tokio::time::timeout(Duration::from_secs(10), loop_task)
            .await
            .expect("notice loop did not exit")
            .expect("notice loop task panicked");
    result.unwrap();
    assert!(screen.contains("Memory is ready at"), "{screen}");
    let transcript_text = transcript_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        transcript_text.contains("Read notes with kuru memory notes ID;"),
        "{transcript_text}"
    );

    drop(store);
    let reopened = reopen_store(options).await;
    assert!(
        MemoryNotice::pending(reopened.clone())
            .await
            .unwrap()
            .is_none()
    );
    let snapshot = reopened.begin_active_export().await.unwrap();
    let page = snapshot.page(None).await.unwrap();
    assert!(page.records.iter().all(|record| match record {
        StorageRecord::Message { content, .. } => !content.contains("Memory is ready at"),
        StorageRecord::State { .. } => true,
    }));
    reopened.close().await.unwrap();
    drop(project);
    drop(data_root);
}

#[tokio::test]
async fn failed_initial_tui_draw_never_marks_the_notice_shown() {
    let (data_root, options, store) = persistent_store().await;
    let notice = MemoryNotice::pending(store.clone()).await.unwrap().unwrap();
    let provider = BlockingProvider::new();
    let (project, harness, models) = fixture_with_memory(store.clone()).await;
    let mut terminal = Terminal::new(FailingBackend::initially_failing(provider)).unwrap();
    let input = Box::pin(stream::empty());
    let error =
        run_loop_with_stream_and_notice(&mut terminal, harness, models, input, Some(notice))
            .await
            .unwrap_err();
    assert!(format!("{error:#}").contains("injected backend failure"));

    drop(store);
    let reopened = reopen_store(options).await;
    assert!(
        MemoryNotice::pending(reopened.clone())
            .await
            .unwrap()
            .is_some()
    );
    reopened.close().await.unwrap();
    drop(project);
    drop(data_root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notice_text_never_reaches_the_provider_request_for_a_real_tui_turn() {
    let (data_root, _options, store) = persistent_store().await;
    let notice = MemoryNotice::pending(store.clone()).await.unwrap().unwrap();
    let provider = CapturingProvider::new();
    let (project, harness, models) = harness_with_provider(store, provider.clone()).await;
    let (input_tx, input_rx) = mpsc::channel(8);
    let input = Box::pin(stream::unfold(input_rx, |mut input_rx| async move {
        input_rx.recv().await.map(|event| (Ok(event), input_rx))
    }));
    let loop_task = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
        run_loop_with_stream_and_notice(&mut terminal, harness, models, input, Some(notice)).await
    });
    for character in "turn reaches provider".chars() {
        input_tx
            .send(TerminalEvent::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(character),
                crossterm::event::KeyModifiers::NONE,
            )))
            .await
            .unwrap();
    }
    input_tx
        .send(TerminalEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        )))
        .await
        .unwrap();
    provider.wait_for_request().await;
    let requests = provider.requests();
    assert!(requests.iter().any(|request| {
        request
            .messages
            .iter()
            .any(|message| message.plain_text() == Some("turn reaches provider"))
    }));
    assert!(requests.iter().all(|request| {
        request
            .messages
            .iter()
            .all(|message| !message.text_projection().contains("Memory is ready at"))
    }));

    drop(input_tx);
    let error = tokio::time::timeout(Duration::from_secs(10), loop_task)
        .await
        .expect("TUI loop did not clean up after input closure")
        .expect("TUI loop task panicked")
        .unwrap_err();
    assert!(format!("{error:#}").contains("terminal input closed"));
    drop(project);
    drop(data_root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_loop_eof_read_and_draw_failures_abort_the_owned_provider_before_returning() {
    for failure in [Failure::Eof, Failure::Read, Failure::Draw] {
        let provider = BlockingProvider::new();
        let (_directory, harness) = blocking_fixture(provider.clone()).await;
        let input = controlled_input(provider.clone(), failure);
        let error = tokio::time::timeout(Duration::from_secs(15), async {
            match failure {
                Failure::Eof | Failure::Read => {
                    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
                    run_loop_with_stream(&mut terminal, harness, vec![], input).await
                }
                Failure::Draw => {
                    let mut terminal =
                        Terminal::new(FailingBackend::new(provider.clone())).unwrap();
                    run_loop_with_stream(&mut terminal, harness, vec![], input).await
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{failure:?} loop timeout: provider started={}, dropped={}",
                provider.started.load(Ordering::SeqCst),
                provider.dropped.load(Ordering::SeqCst)
            )
        })
        .unwrap_err();
        let expected = match failure {
            Failure::Eof => "terminal input closed",
            Failure::Read => "terminal input: injected terminal read failure",
            Failure::Draw => "terminal draw: injected backend failure",
        };
        assert!(format!("{error:#}").contains(expected), "{error:#}");
        let started = provider.started.load(Ordering::SeqCst);
        let dropped = provider.dropped.load(Ordering::SeqCst);
        assert!(started > 0, "{expected} returned before provider entry");
        assert!(
            dropped >= started,
            "{expected} returned before every active provider future dropped: started={started}, dropped={dropped}"
        );
    }
}

#[tokio::test]
async fn apply_completion_orders_current_outcomes_and_ignores_stale_generations() {
    let (_directory, harness, _) = fixture().await;
    let mut events = harness.subscribe();
    let harness = Arc::new(tokio::sync::Mutex::new(harness));
    let initial = {
        let harness = harness.lock().await;
        project_initial_view(&harness).await.unwrap()
    };
    let mut view = View::from_initial(initial, vec![]);
    let mut job = None;

    seed_stale_runtime(&mut view);
    view.begin_operation();
    let session = view.session.clone();
    let outcome = apply_completion(
        (
            4,
            Ok(DispatchOutcome::Turn(ControlledTurnOutput {
                output: TurnOutput {
                    session,
                    speaker: view.parts[0].0.clone(),
                    text: "authoritative completion".into(),
                    relationship: None,
                    input_tokens: 11,
                    output_tokens: 7,
                    limited: false,
                    limit_reasons: Some(vec![]),
                    response_outcome: Some(ResponseOutcome::Text),
                    events: vec![],
                },
                reused: false,
            })),
        ),
        4,
        &mut events,
        &mut view,
        &harness,
        &mut job,
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        CompletionState::Settled { quit: false, .. }
    ));
    assert_eq!(
        view.transcript.last().unwrap().1,
        "authoritative completion"
    );
    assert_eq!(view.status, "Complete");
    assert!(!view.busy);
    assert!(view.operation_start.is_none());
    assert_eq!(
        view.completion_metadata[&0],
        "11 input tokens · 7 output tokens"
    );
    assert_runtime_projection(&view, &harness).await;

    let transcript = view.transcript.clone();
    let outcome = apply_completion(
        (
            5,
            Ok(DispatchOutcome::Turn(ControlledTurnOutput {
                output: TurnOutput {
                    session: view.session.clone(),
                    speaker: view.parts[0].0.clone(),
                    text: "authoritative completion".into(),
                    relationship: None,
                    input_tokens: 11,
                    output_tokens: 7,
                    limited: false,
                    limit_reasons: Some(vec![]),
                    response_outcome: Some(ResponseOutcome::Text),
                    events: vec![],
                },
                reused: true,
            })),
        ),
        5,
        &mut events,
        &mut view,
        &harness,
        &mut job,
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        CompletionState::Settled { quit: false, .. }
    ));
    assert_eq!(view.transcript, transcript);
    assert_eq!(view.status, "Complete · stored result reused");
    assert!(
        view.notice
            .as_deref()
            .is_some_and(|notice| notice.contains("no new provider or tool work"))
    );

    seed_stale_runtime(&mut view);
    view.begin_operation();
    let outcome = apply_completion(
        (5, Ok(DispatchOutcome::Command("Mode: freudian".into()))),
        5,
        &mut events,
        &mut view,
        &harness,
        &mut job,
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        CompletionState::Settled { quit: false, .. }
    ));
    assert_eq!(view.status, "Complete");
    assert!(
        view.notice
            .as_deref()
            .is_some_and(|notice| notice.contains("saved for this project"))
    );
    assert!(view.operation_start.is_none());
    assert_runtime_projection(&view, &harness).await;

    seed_stale_runtime(&mut view);
    view.begin_operation();
    let outcome = apply_completion(
        (6, Err(anyhow::anyhow!("controlled dispatch failure"))),
        6,
        &mut events,
        &mut view,
        &harness,
        &mut job,
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        CompletionState::Settled { quit: false, .. }
    ));
    assert_eq!(view.status, "Failed · details in conversation");
    assert!(view.transcript.last().is_some_and(
        |(role, text)| role == "error" && text.contains("controlled dispatch failure")
    ));
    assert!(view.operation_start.is_none());
    assert_runtime_projection(&view, &harness).await;

    seed_stale_runtime(&mut view);
    view.begin_operation();
    view.completion_locked = true;
    job = Some(tokio::spawn(std::future::pending()));
    let before = (
        view.transcript.clone(),
        view.status.clone(),
        view.busy,
        view.operation_start,
        view.turns,
        view.mode.clone(),
        view.model.clone(),
        view.effort.clone(),
        view.parts.clone(),
        view.relationships.clone(),
        view.focus.clone(),
        view.completion_locked,
    );
    let outcome = apply_completion(
        (6, Ok(DispatchOutcome::Command("STALE".into()))),
        7,
        &mut events,
        &mut view,
        &harness,
        &mut job,
        false,
    )
    .await
    .unwrap();
    assert_eq!(outcome, CompletionState::Stale);
    assert_eq!(
        (
            view.transcript,
            view.status,
            view.busy,
            view.operation_start,
            view.turns,
            view.mode,
            view.model,
            view.effort,
            view.parts,
            view.relationships,
            view.focus,
            view.completion_locked,
        ),
        before
    );
    assert!(
        job.is_some(),
        "a stale completion must not clear the live job"
    );
    job.take().unwrap().abort();
}

#[tokio::test]
async fn cancellation_consumes_an_answer_that_won_the_completion_race() {
    let (_directory, harness, _) = fixture().await;
    let mut events = harness.subscribe();
    let harness = Arc::new(tokio::sync::Mutex::new(harness));
    let initial = {
        let harness = harness.lock().await;
        project_initial_view(&harness).await.unwrap()
    };
    let mut view = View::from_initial(initial, vec![]);
    view.begin_operation();
    let output = TurnOutput {
        session: view.session.clone(),
        speaker: view.parts[0].0.clone(),
        text: "answer won cancellation".into(),
        relationship: None,
        input_tokens: 5,
        output_tokens: 3,
        limited: false,
        limit_reasons: Some(vec![]),
        response_outcome: Some(ResponseOutcome::Text),
        events: vec![],
    };
    let (tx, mut completions) = mpsc::channel(1);
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let mut job = Some(tokio::spawn(async move {
        while !worker_token.is_cancelled() {
            tokio::task::yield_now().await;
        }
        tx.send((
            7,
            Ok(DispatchOutcome::Turn(ControlledTurnOutput {
                output,
                reused: false,
            })),
        ))
        .await
        .unwrap();
    }));
    let mut cancellation = Some(token);
    let mut generation = 7;
    let mut quit_pending = false;
    assert!(
        !cancel_operation(
            &mut completions,
            &mut events,
            &mut view,
            &harness,
            &mut job,
            &mut cancellation,
            &mut generation,
            &mut quit_pending,
        )
        .await
        .unwrap()
    );
    assert_eq!(view.status, "Complete");
    assert_eq!(view.transcript.last().unwrap().1, "answer won cancellation");
    assert!(job.is_none());
    assert!(cancellation.is_none());
    assert_eq!(generation, 8);
    harness.lock().await.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn queued_completion_wakes_before_a_future_animation_deadline_and_reaches_completion() {
    let (_directory, harness, _) = fixture().await;
    let harness = Arc::new(tokio::sync::Mutex::new(harness));
    let initial = {
        let harness = harness.lock().await;
        project_initial_view(&harness).await.unwrap()
    };
    let mut view = View::from_initial(initial, vec![]);
    let mut input = stream::pending();
    let (completion_tx, mut completion_rx) = mpsc::channel(1);
    let (_activity_tx, mut events) = broadcast::channel(1);
    let deadline = TokioInstant::now() + Duration::from_secs(60);
    let frame = view.frame;
    view.begin_operation();
    let scheduler = Scheduler::new();

    let mut wake = Box::pin(next_wake(
        &scheduler,
        &mut input,
        &mut completion_rx,
        &mut events,
        WakeAvailability {
            input: true,
            completion: true,
            activity: true,
        },
        deadline,
    ));
    for stage in ["cooperative yield", "all-pending select"] {
        futures::future::poll_fn(|context| {
            assert!(
                matches!(wake.as_mut().poll(context), Poll::Pending),
                "scheduler unexpectedly resolved while reaching {stage}"
            );
            Poll::Ready(())
        })
        .await;
    }
    completion_tx
        .send((
            3,
            Ok(DispatchOutcome::Command("scheduler completion".into())),
        ))
        .await
        .unwrap();
    let wake = wake.await;
    let Wake::Completion(Some(completion)) = wake else {
        panic!("completion should wake before the future animation deadline");
    };
    let mut job = None;
    let result = apply_completion(
        completion,
        3,
        &mut events,
        &mut view,
        &harness,
        &mut job,
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        result,
        CompletionState::Settled { quit: false, .. }
    ));
    assert_eq!(view.frame, frame, "completion must not advance motion time");
    assert_eq!(
        view.transcript.last(),
        Some(&(String::from("kuru"), String::from("scheduler completion")))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_caps_activity_while_a_concurrent_sender_continues_producing() {
    let (_directory, harness, _) = fixture().await;
    let harness = Arc::new(tokio::sync::Mutex::new(harness));
    let initial = {
        let harness = harness.lock().await;
        project_initial_view(&harness).await.unwrap()
    };
    let mut view = View::from_initial(initial, vec![]);
    let (activity_tx, mut events) = broadcast::channel(1024);
    for index in 0..(ACTIVITY_DRAIN_CAP + 44) {
        activity_tx
            .send(kuru_runtime::Event::ToolStarted {
                actor: "part".into(),
                name: format!("queued-{index}"),
            })
            .unwrap();
    }
    let producer = {
        let activity_tx = activity_tx.clone();
        thread::spawn(move || {
            for index in 0..128 {
                activity_tx
                    .send(kuru_runtime::Event::ToolStarted {
                        actor: "part".into(),
                        name: format!("concurrent-{index}"),
                    })
                    .unwrap();
                thread::yield_now();
            }
        })
    };

    view.begin_operation();
    let mut job = None;
    let result = apply_completion(
        (
            9,
            Ok(DispatchOutcome::Command("completed after activity".into())),
        ),
        9,
        &mut events,
        &mut view,
        &harness,
        &mut job,
        false,
    )
    .await
    .unwrap();
    producer.join().unwrap();

    assert!(matches!(
        result,
        CompletionState::Settled { quit: false, .. }
    ));
    assert_eq!(
        view.activity.len(),
        100,
        "the view keeps its documented tail"
    );
    assert_eq!(
        events.len(),
        44 + 128,
        "completion drains one captured cap while the sender keeps later activity queued"
    );
    assert_eq!(
        view.transcript.last(),
        Some(&(
            String::from("kuru"),
            String::from("completed after activity")
        ))
    );
    assert!(!view.busy);
    assert!(view.operation_start.is_none());
    assert_runtime_projection(&view, &harness).await;
}
