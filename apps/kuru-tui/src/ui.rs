use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use crossterm::{
    event::{
        self, Event as TerminalEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::{Stream, StreamExt};
use kuru_core::{Mode, ModelInfo, Relationship};
use kuru_runtime::{
    CancellationToken, ControlledTurnOutput, Event, Harness, INTERRUPTION_ROLE, INTERRUPTION_TEXT,
    ResponseOutcome, TurnLimitReason, TurnOutput, turn_was_cancelled,
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    text::Line,
};
use tokio::{
    sync::{Mutex, broadcast, mpsc},
    task::JoinHandle,
    time::{Instant as TokioInstant, sleep_until},
};
use unicode_width::UnicodeWidthChar;

use crate::{cli::validate_effort, memory_notice::MemoryNotice};

mod render;
#[cfg(test)]
mod runtime_tests;
mod scene;
pub use render::draw;

const HELP: &str = "Enter send · Alt+Enter newline · F2 models · F3 effort · F4 mode · Esc cancel\n/help · /parts · /mode ifs|polyvagal|freudian|jungian · /model ID · /effort LEVEL\n/focus NAME|ID|auto · /relate KIND ID,ID · /memory ID · /notes ID · /retry · /dream · /undo-dream · /quit\n/memory-status · /memory-history\nModel, effort and mode selections are remembered for this project.";
const ACTIVITY_DRAIN_CAP: usize = 256;
const INTERRUPTION_REFRESH_NOTICE: &str =
    "Persisted interruption status could not be refreshed · reopen the session to inspect it";

#[derive(Debug, Clone, PartialEq)]
pub enum Picker {
    Models,
    Efforts,
    Modes,
}

#[derive(Debug)]
pub(crate) enum DispatchOutcome {
    Command(String),
    Turn(ControlledTurnOutput),
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub turns: usize,
    pub mode: String,
    pub model: String,
    pub effort: String,
    pub parts: Vec<(String, String)>,
    pub relationships: Vec<Relationship>,
    pub focus: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InitialViewData {
    pub transcript: Vec<(String, String)>,
    pub session: String,
    pub project: String,
    pub motion: bool,
    pub runtime: RuntimeSnapshot,
}

#[derive(Debug, Clone)]
pub struct View {
    pub transcript: Vec<(String, String)>,
    completion_metadata: BTreeMap<usize, String>,
    pub input: String,
    pub cursor: usize,
    pub mode: String,
    pub model: String,
    pub effort: String,
    pub session: String,
    pub parts: Vec<(String, String)>,
    pub activity: Vec<String>,
    pub busy: bool,
    pub status: String,
    pub speaker: String,
    pub picker: Option<Picker>,
    pub selected: usize,
    pub models: Vec<ModelInfo>,
    pub scroll: u16,
    pub frame: u64,
    pub motion: bool,
    pub part_activity: BTreeMap<String, String>,
    pub relationships: Vec<Relationship>,
    pub focus: Option<String>,
    pub speaker_id: String,
    pub routes: Vec<(String, String)>,
    pub project: String,
    pub turns: usize,
    pub focused: bool,
    pub show_scene: bool,
    pub query: String,
    pub notice: Option<String>,
    pub operation_ms: u64,
    clock_ms: u64,
    last_frame_ms: u64,
    operation_start: Option<u64>,
    notice_until: u64,
    completion_locked: bool,
}

impl View {
    pub fn from_initial(initial: InitialViewData, models: Vec<ModelInfo>) -> Self {
        let InitialViewData {
            transcript,
            session,
            project,
            motion,
            runtime,
        } = initial;
        let show_scene = transcript.is_empty();
        Self {
            completion_metadata: BTreeMap::new(),
            transcript,
            input: String::new(),
            cursor: 0,
            mode: runtime.mode,
            model: runtime.model,
            effort: runtime.effort,
            session,
            parts: runtime.parts,
            activity: vec![],
            busy: false,
            status: "Ready · /help for commands".into(),
            speaker: "pool".into(),
            picker: None,
            selected: 0,
            models,
            scroll: 0,
            frame: 0,
            motion,
            part_activity: BTreeMap::new(),
            relationships: runtime.relationships,
            focus: runtime.focus,
            speaker_id: String::new(),
            routes: vec![],
            project,
            turns: runtime.turns,
            focused: true,
            show_scene,
            query: String::new(),
            notice: None,
            operation_ms: 0,
            clock_ms: 0,
            last_frame_ms: 0,
            operation_start: None,
            notice_until: 0,
            completion_locked: false,
        }
    }

    /// Ambient frames are slower than busy indicators; all motion uses this
    /// supplied clock so scenes never infer state or read wall time themselves.
    pub fn advance_animation(&mut self, elapsed: Duration) -> bool {
        let now = elapsed.as_millis().min(u64::MAX as u128) as u64;
        self.clock_ms = now;
        let mut dirty = false;
        if self.notice.is_some() && now >= self.notice_until {
            self.notice = None;
            dirty = true;
        }
        if let Some(start) = self.operation_start {
            let duration = now.saturating_sub(start);
            dirty |= duration / 1000 != self.operation_ms / 1000;
            self.operation_ms = duration;
        }
        let interval = if self.busy { 80 } else { 250 };
        if self.motion && self.focused && now.saturating_sub(self.last_frame_ms) >= interval {
            self.frame = now / 80;
            self.last_frame_ms = now;
            return true;
        }
        dirty
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some(message.into());
        self.notice_until = self.clock_ms.saturating_add(5000);
    }

    fn begin_operation(&mut self) {
        self.busy = true;
        self.completion_locked = false;
        self.notice = None;
        self.operation_start = Some(self.clock_ms);
        self.operation_ms = 0;
    }

    pub fn apply_runtime(&mut self, runtime: RuntimeSnapshot) {
        self.turns = runtime.turns;
        self.mode = runtime.mode;
        self.model = runtime.model;
        self.effort = runtime.effort;
        self.parts = runtime.parts;
        self.relationships = runtime.relationships;
        self.focus = runtime.focus;
        self.part_activity
            .retain(|id, _| self.parts.iter().any(|(part, _)| part == id));
        self.routes.retain(|(from, to)| {
            self.parts.iter().any(|(id, _)| id == from) && self.parts.iter().any(|(id, _)| id == to)
        });
    }

    fn actor_name(&self, id: &str) -> String {
        self.parts
            .iter()
            .find(|(part, _)| part == id)
            .map(|(_, name)| name.split(" · ").next().unwrap_or(name).to_owned())
            .or_else(|| {
                self.relationships
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| r.kind.to_string())
            })
            .unwrap_or_else(|| id.chars().take(16).collect())
    }

    fn settle(&mut self) {
        self.operation_start = None;
        for phase in self.part_activity.values_mut() {
            if phase != "error" {
                *phase = "idle".into();
            }
        }
    }

    pub fn options(&self) -> Vec<String> {
        let options: Vec<String> = match self.picker {
            Some(Picker::Models) => self.models.iter().map(|m| m.id.clone()).collect(),
            Some(Picker::Efforts) => {
                let mut efforts = vec!["default".to_owned()];
                if let Some(model) = self.models.iter().find(|m| m.id == self.model) {
                    efforts.extend(
                        model
                            .efforts
                            .iter()
                            .filter(|e| e.as_str() != "default")
                            .cloned(),
                    );
                }
                efforts
            }
            Some(Picker::Modes) => ["ifs", "polyvagal", "freudian", "jungian"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            None => vec![],
        };
        let query = self.query.to_lowercase();
        options
            .into_iter()
            .filter(|option| option.to_lowercase().contains(&query))
            .collect()
    }

    fn open_picker(&mut self, picker: Picker) {
        if self.busy {
            self.notify("Finish or cancel the current turn to change settings.");
            return;
        }
        self.query.clear();
        let current = match picker {
            Picker::Models => &self.model,
            Picker::Efforts => &self.effort,
            Picker::Modes => &self.mode,
        }
        .clone();
        self.picker = Some(picker);
        self.selected = self
            .options()
            .iter()
            .position(|option| option == &current)
            .unwrap_or(0);
    }

    pub fn event(&mut self, event: Event) {
        if matches!(
            event.kind.as_str(),
            "active" | "idle" | "speaker" | "tool" | "error"
        ) {
            self.part_activity
                .insert(event.actor.clone(), event.kind.clone());
        }
        let mut detail = event.detail.clone();
        if event.kind == "peer" {
            // Show routing, never the private message contained in the envelope.
            detail = "peer message".into();
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&event.detail)
                && let Some(to) = value
                    .pointer("/params/message/metadata/recipient")
                    .and_then(|v| v.as_str())
            {
                detail = format!("→ {}", self.actor_name(to));
                self.routes.push((event.actor.clone(), to.into()));
                if self.routes.len() > 6 {
                    self.routes.remove(0);
                }
            }
        } else if event.kind == "relationship" {
            detail = "relationship updated".into();
            if let Ok(relation) = serde_json::from_str::<Relationship>(&event.detail) {
                detail = format!(
                    "{} · {}",
                    relation.kind,
                    relation
                        .members
                        .iter()
                        .map(|id| self.actor_name(id))
                        .collect::<Vec<_>>()
                        .join(" + ")
                );
                self.focus = Some(relation.id.clone());
                self.relationships.retain(|r| r.id != relation.id);
                self.relationships.push(relation);
            }
        } else if event.kind == "state" {
            detail = "modeled state updated".into();
        }
        if event.kind == "speaker" && !self.completion_locked {
            self.speaker_id = event.actor.clone();
            self.speaker = self
                .parts
                .iter()
                .find(|(id, _)| id == &event.actor)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| {
                    format!(
                        "{} · {}",
                        event.detail,
                        event.actor.chars().take(8).collect::<String>()
                    )
                });
        }
        if event.kind == "response" {
            detail = "response activity".into();
        }
        let activity_status = format!("{} · {}", event.kind, self.actor_name(&event.actor));
        if !self.completion_locked {
            self.status = activity_status.clone();
        }
        let detail: String = detail
            .chars()
            .filter(|c| !c.is_control())
            .take(140)
            .collect();
        self.activity.push(format!("{activity_status} · {detail}"));
        if self.activity.len() > 100 {
            self.activity.remove(0);
        }
    }

    pub fn complete_turn(&mut self, output: TurnOutput) {
        let outcome = outcome_summary(&output);
        let speaker = output
            .relationship
            .as_ref()
            .map(|relationship| {
                let members = relationship
                    .members
                    .iter()
                    .map(|id| self.actor_name(id))
                    .collect::<Vec<_>>();
                if members.is_empty() {
                    relationship.kind.to_string()
                } else {
                    format!("{} · {}", relationship.kind, members.join(" + "))
                }
            })
            .unwrap_or_else(|| self.actor_name(&output.speaker));
        self.speaker_id = output.speaker;
        self.speaker = speaker.clone();
        let index = self.transcript.len();
        self.transcript.push((speaker, output.text));
        self.completion_metadata.insert(
            index,
            match &outcome {
                Some(outcome) => format!(
                    "{} input tokens · {} output tokens · {outcome}",
                    output.input_tokens, output.output_tokens
                ),
                None => format!(
                    "{} input tokens · {} output tokens",
                    output.input_tokens, output.output_tokens
                ),
            },
        );
        self.show_scene = false;
        self.status =
            outcome.map_or_else(|| "Complete".into(), |value| format!("Complete · {value}"));
        self.completion_locked = true;
    }

    pub fn terminal_event(&mut self, event: TerminalEvent) -> (bool, Option<String>) {
        let command = match event {
            TerminalEvent::Key(key) if key.kind == KeyEventKind::Release => return (false, None),
            TerminalEvent::Key(key) => self.key(key),
            TerminalEvent::Paste(text) => {
                self.paste(&text);
                None
            }
            TerminalEvent::FocusGained => {
                self.focused = true;
                None
            }
            TerminalEvent::FocusLost => {
                self.focused = false;
                None
            }
            TerminalEvent::Resize(_, _) => None,
            _ => return (false, None),
        };
        (true, command)
    }

    pub fn key(&mut self, key: KeyEvent) -> Option<String> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Some(if self.busy { "/cancel" } else { "/quit" }.into());
        }
        if self.picker.is_some() {
            let len = self.options().len();
            match key.code {
                KeyCode::Esc => self.picker = None,
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => self.selected = (self.selected + 1).min(len.saturating_sub(1)),
                KeyCode::Enter => {
                    let value = self.options().get(self.selected).cloned();
                    value.as_ref()?;
                    let command = match self.picker.take() {
                        Some(Picker::Models) => "/model",
                        Some(Picker::Efforts) => "/effort",
                        _ => "/mode",
                    };
                    return value.map(|v| format!("{command} {v}"));
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if self.query.len() < 256 {
                        self.query.push(c);
                    }
                    self.selected = 0;
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.selected = 0;
                }
                _ => {}
            }
            return None;
        }
        match key.code {
            KeyCode::F(2) => {
                self.open_picker(Picker::Models);
            }
            KeyCode::F(3) => {
                self.open_picker(Picker::Efforts);
            }
            KeyCode::F(4) => {
                self.open_picker(Picker::Modes);
            }
            KeyCode::Esc if self.busy => return Some("/cancel".into()),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(10),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT) =>
            {
                self.insert('\n')
            }
            KeyCode::Enter if !self.busy => {
                let text = std::mem::take(&mut self.input);
                self.cursor = 0;
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => self.insert(c),
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.input[..self.cursor]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, _)| i);
                self.input.drain(previous..self.cursor);
                self.cursor = previous;
            }
            KeyCode::Delete if self.cursor < self.input.len() => {
                let next =
                    self.cursor + self.input[self.cursor..].chars().next().unwrap().len_utf8();
                self.input.drain(self.cursor..next);
            }
            KeyCode::Left => {
                self.cursor = self.input[..self.cursor]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, _)| i)
            }
            KeyCode::Right if self.cursor < self.input.len() => {
                self.cursor += self.input[self.cursor..].chars().next().unwrap().len_utf8()
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.len(),
            _ => {}
        }
        None
    }

    pub fn paste(&mut self, text: &str) {
        if self.picker.is_some() {
            for c in text.chars().filter(|c| !c.is_control()) {
                if self.query.len() + c.len_utf8() > 256 {
                    break;
                }
                self.query.push(c);
            }
            self.selected = 0;
        } else {
            for c in text.chars() {
                self.insert(c);
            }
        }
    }

    fn insert(&mut self, c: char) {
        if self.input.len() + c.len_utf8() <= 131_072 {
            self.input.insert(self.cursor, c);
            self.cursor += c.len_utf8();
        }
    }
}

fn outcome_summary(output: &TurnOutput) -> Option<String> {
    let mut labels = Vec::new();
    match output.limit_reasons.as_deref() {
        Some(reasons) => {
            for reason in reasons {
                let label = match reason {
                    TurnLimitReason::ToolCalls => "tool-call budget reached",
                    TurnLimitReason::PeerRounds => "peer-round budget reached",
                    TurnLimitReason::LegacyUnspecified => "legacy limit · cause unspecified",
                };
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
            if output.limited
                && labels.is_empty()
                && output.response_outcome != Some(ResponseOutcome::Empty)
            {
                labels.push("limited result · cause unspecified");
            }
        }
        None if output.limited => labels.push("legacy limit · cause unspecified"),
        None => {}
    }
    if output.response_outcome == Some(ResponseOutcome::Empty) {
        labels.push("empty response");
    }
    (!labels.is_empty()).then(|| labels.join(" · "))
}

fn reduced_motion(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// Read the initial TUI presentation from the runtime at the adapter boundary.
/// `View` itself remains a synchronous owned value.
pub async fn project_initial_view(harness: &Harness) -> Result<InitialViewData> {
    let transcript = harness
        .history()
        .await?
        .into_iter()
        .map(|message| project_transcript_message(&message.role, message.content))
        .collect();
    Ok(InitialViewData {
        transcript,
        session: harness.session.id.clone(),
        project: harness
            .cwd()
            .file_name()
            .unwrap_or(harness.cwd().as_os_str())
            .to_string_lossy()
            .into_owned(),
        motion: !reduced_motion(std::env::var("KURU_REDUCED_MOTION").ok().as_deref()),
        runtime: project_runtime_snapshot(harness),
    })
}

fn project_transcript_message(role: &str, content: String) -> (String, String) {
    if role == INTERRUPTION_ROLE {
        ("kuru".into(), content)
    } else {
        (role.into(), content)
    }
}

async fn append_missing_interruption_markers(harness: &Harness, view: &mut View) -> Result<()> {
    let displayed = view
        .transcript
        .iter()
        .filter(|(speaker, body)| speaker == "kuru" && body == INTERRUPTION_TEXT)
        .count();
    let durable = harness
        .history()
        .await?
        .into_iter()
        .filter(|message| message.role == INTERRUPTION_ROLE && message.content == INTERRUPTION_TEXT)
        .count();
    for _ in displayed..durable {
        view.transcript
            .push(("kuru".into(), INTERRUPTION_TEXT.into()));
        view.show_scene = false;
    }
    Ok(())
}

fn present_interruption_refresh(view: &mut View, result: Result<()>) {
    if result.is_err() {
        view.notify(INTERRUPTION_REFRESH_NOTICE);
    }
}

/// Project mutable runtime state without letting the renderer access a Harness.
pub fn project_runtime_snapshot(harness: &Harness) -> RuntimeSnapshot {
    RuntimeSnapshot {
        turns: harness.session.turns,
        mode: harness.config.mode.to_string(),
        model: harness.config.model.clone(),
        effort: harness
            .config
            .effort
            .clone()
            .unwrap_or_else(|| "default".into()),
        parts: harness
            .topology
            .parts
            .iter()
            .filter(|part| part.active)
            .map(|part| (part.id.clone(), format!("{} · {}", part.name, part.role)))
            .collect(),
        relationships: live_relationships(harness),
        focus: harness
            .topology
            .focus
            .as_ref()
            .map(|focus| focus.id.clone()),
    }
}

fn live_relationships(harness: &Harness) -> Vec<Relationship> {
    harness
        .topology
        .relationships
        .iter()
        .filter(|r| {
            r.members.iter().all(|id| {
                harness
                    .topology
                    .parts
                    .iter()
                    .any(|p| p.active && p.id == *id)
            })
        })
        .cloned()
        .collect()
}

fn editor_layout(input: &str, cursor: usize, width: usize) -> (Vec<Line<'static>>, usize, usize) {
    let mut lines = vec![String::new()];
    let mut x = 0;
    let mut position = (0, 0);
    for (index, c) in input.char_indices() {
        let cells = c.width().unwrap_or(0).min(width);
        if c != '\n' && x + cells > width {
            lines.push(String::new());
            x = 0;
        }
        if index == cursor {
            position = (x, lines.len() - 1);
        }
        if c == '\n' {
            lines.push(String::new());
            x = 0;
        } else {
            lines.last_mut().unwrap().push(c);
            x += cells;
        }
    }
    if cursor == input.len() {
        if x == width {
            lines.push(String::new());
            x = 0;
        }
        position = (x, lines.len() - 1);
    }
    (
        lines.into_iter().map(Line::from).collect(),
        position.0,
        position.1,
    )
}

/// Own terminal initialization and exact console restoration across errors.
pub struct TerminalSession {
    restored: bool,
    #[cfg(windows)]
    console: kuru_platform::windows::console::ConsoleModeGuard,
}

impl TerminalSession {
    pub fn enter(output: &mut impl io::Write) -> Result<Self> {
        // The guard precedes raw mode and every escape write: even partial
        // initialization must restore the caller's original native state.
        let guard = Self {
            restored: false,
            #[cfg(windows)]
            console: kuru_platform::windows::console::ConsoleModeGuard::capture()?,
        };
        enable_raw_mode()?;
        #[cfg(windows)]
        guard.console.enable_virtual_terminal_output()?;
        execute!(
            output,
            EnterAlternateScreen,
            event::EnableBracketedPaste,
            event::EnableFocusChange
        )?;
        Ok(guard)
    }

    pub fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        let raw = disable_raw_mode();
        let screen = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            event::DisableBracketedPaste,
            event::DisableFocusChange
        );
        #[cfg(windows)]
        let modes = self.console.restore();
        raw?;
        screen?;
        #[cfg(windows)]
        modes?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub async fn run(harness: Harness, models: Vec<ModelInfo>) -> Result<()> {
    run_with_notice(harness, models, None).await
}

/// App-private notice delivery keeps the public UI adapter independent of
/// durable presentation state.
pub(crate) async fn run_with_notice(
    harness: Harness,
    models: Vec<ModelInfo>,
    notice: Option<MemoryNotice>,
) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "interactive mode requires a terminal; use kuru run PROMPT"
    );
    let mut guard = TerminalSession::enter(&mut io::stdout())?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let result = run_loop_with_notice(&mut terminal, harness, models, notice).await;
    // Ratatui's Drop may show its cursor. Finish that while terminal output
    // processing is still active, before restoring the caller's console modes.
    drop(terminal);
    let restored = guard.restore();
    match result {
        Ok(()) => restored,
        Err(error) => Err(error),
    }
}

async fn run_loop_with_notice<B: Backend>(
    terminal: &mut Terminal<B>,
    harness: Harness,
    models: Vec<ModelInfo>,
    notice: Option<MemoryNotice>,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    run_loop_with_stream_and_notice(terminal, harness, models, EventStream::new(), notice).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WakeSource {
    Terminal,
    Completion,
    Activity,
    Animation,
}

impl WakeSource {
    fn following(self) -> Self {
        match self {
            Self::Terminal => Self::Completion,
            Self::Completion => Self::Activity,
            Self::Activity => Self::Animation,
            Self::Animation => Self::Terminal,
        }
    }
}

#[derive(Debug)]
enum Wake {
    Terminal(Option<io::Result<TerminalEvent>>),
    Completion(Option<(u64, Result<DispatchOutcome>)>),
    Activity(Result<Event, broadcast::error::RecvError>),
    Animation,
}

struct Scheduler {
    first: WakeSource,
}

#[derive(Debug, Clone, Copy)]
struct WakeAvailability {
    input: bool,
    completion: bool,
    activity: bool,
}

async fn next_wake<S>(
    scheduler: &Scheduler,
    input: &mut S,
    rx: &mut mpsc::Receiver<(u64, Result<DispatchOutcome>)>,
    events: &mut broadcast::Receiver<Event>,
    available: WakeAvailability,
    animation_at: TokioInstant,
) -> Wake
where
    S: Stream<Item = io::Result<TerminalEvent>> + Unpin,
{
    // Every ready-source pass cooperates with sibling Tokio work before the
    // biased source rotation chooses the next wake.
    tokio::task::yield_now().await;
    match scheduler.first {
        WakeSource::Terminal => tokio::select! {
            biased;
            event = input.next(), if available.input => Wake::Terminal(event),
            completion = rx.recv(), if available.completion => Wake::Completion(completion),
            activity = events.recv(), if available.activity => Wake::Activity(activity),
            _ = sleep_until(animation_at) => Wake::Animation,
        },
        WakeSource::Completion => tokio::select! {
            biased;
            completion = rx.recv(), if available.completion => Wake::Completion(completion),
            activity = events.recv(), if available.activity => Wake::Activity(activity),
            _ = sleep_until(animation_at) => Wake::Animation,
            event = input.next(), if available.input => Wake::Terminal(event),
        },
        WakeSource::Activity => tokio::select! {
            biased;
            activity = events.recv(), if available.activity => Wake::Activity(activity),
            _ = sleep_until(animation_at) => Wake::Animation,
            event = input.next(), if available.input => Wake::Terminal(event),
            completion = rx.recv(), if available.completion => Wake::Completion(completion),
        },
        WakeSource::Animation => tokio::select! {
            biased;
            _ = sleep_until(animation_at) => Wake::Animation,
            event = input.next(), if available.input => Wake::Terminal(event),
            completion = rx.recv(), if available.completion => Wake::Completion(completion),
            activity = events.recv(), if available.activity => Wake::Activity(activity),
        },
    }
}

fn activity_still_open(open: bool, closed: bool) -> bool {
    open && !closed
}

impl Scheduler {
    fn new() -> Self {
        Self {
            first: WakeSource::Terminal,
        }
    }

    fn served(&mut self, source: WakeSource) {
        self.first = source.following();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActivityDrain {
    dirty: bool,
    closed: bool,
}

fn drain_activity(
    events: &mut broadcast::Receiver<Event>,
    view: &mut View,
    maximum: usize,
) -> ActivityDrain {
    let mut drain = ActivityDrain {
        dirty: false,
        closed: false,
    };
    let queued = events.len().min(maximum);
    for _ in 0..queued {
        match events.try_recv() {
            Ok(event) => {
                view.event(event);
                drain.dirty = true;
            }
            Err(broadcast::error::TryRecvError::Lagged(count)) => {
                view.activity
                    .push(format!("{count} activity events omitted"));
                drain.dirty = true;
            }
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(broadcast::error::TryRecvError::Closed) => {
                drain.closed = true;
                break;
            }
        }
    }
    if events.is_closed() && events.is_empty() {
        drain.closed = true;
    }
    drain
}

async fn project_runtime(harness: &Arc<Mutex<Harness>>) -> RuntimeSnapshot {
    let harness = harness.lock().await;
    project_runtime_snapshot(&harness)
}

async fn abort_and_fence(job: &mut Option<JoinHandle<()>>, generation: &mut u64) {
    *generation = generation.wrapping_add(1);
    if let Some(job) = job.take() {
        job.abort();
        let _ = job.await;
    }
}

async fn finish_loop(
    result: Result<()>,
    job: &mut Option<JoinHandle<()>>,
    cancellation: &mut Option<CancellationToken>,
    generation: &mut u64,
    harness: &Arc<Mutex<Harness>>,
) -> Result<()> {
    let Err(error) = result else {
        return Ok(());
    };
    if let Some(cancellation) = cancellation.take() {
        cancellation.cancel();
        *generation = generation.wrapping_add(1);
        if let Some(job) = job.take() {
            let _ = job.await;
        }
    } else {
        abort_and_fence(job, generation).await;
    }
    match harness.lock().await.shutdown(false).await {
        Ok(()) => Err(error),
        Err(cleanup) => Err(error).context(format!("TUI runtime cleanup also failed: {cleanup:#}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionState {
    Stale,
    Settled { quit: bool, activity_closed: bool },
}

async fn apply_completion(
    completion: (u64, Result<DispatchOutcome>),
    generation: u64,
    events: &mut broadcast::Receiver<Event>,
    view: &mut View,
    harness: &Arc<Mutex<Harness>>,
    job: &mut Option<JoinHandle<()>>,
    quit_pending: bool,
) -> Result<CompletionState> {
    let (completed_generation, message) = completion;
    if completed_generation != generation {
        return Ok(CompletionState::Stale);
    }

    let activity = drain_activity(events, view, ACTIVITY_DRAIN_CAP);
    view.busy = false;
    *job = None;
    if quit_pending {
        message?;
        return Ok(CompletionState::Settled {
            quit: true,
            activity_closed: activity.closed,
        });
    }
    match message {
        Ok(DispatchOutcome::Command(text)) => {
            if text.starts_with("Mode:")
                || text.starts_with("Model:")
                || text.starts_with("Effort:")
            {
                view.notify(format!("{text} · saved for this project"));
            } else if !text.is_empty() {
                view.transcript.push(("kuru".into(), text));
                view.show_scene = false;
            }
            view.status = "Complete".into();
            view.completion_locked = true;
        }
        Ok(DispatchOutcome::Turn(result)) if result.reused => {
            view.notify("Stored result reused · no new provider or tool work");
            view.status = "Complete · stored result reused".into();
            view.completion_locked = true;
        }
        Ok(DispatchOutcome::Turn(result)) => view.complete_turn(result.output),
        Err(error) if turn_was_cancelled(&error) => {
            let harness = harness.lock().await;
            let refresh = append_missing_interruption_markers(&harness, view).await;
            present_interruption_refresh(view, refresh);
            view.status = "Cancelled · turn interrupted".into();
            view.completion_locked = true;
        }
        Err(error) => {
            let harness = harness.lock().await;
            let refresh = append_missing_interruption_markers(&harness, view).await;
            present_interruption_refresh(view, refresh);
            view.transcript.push(("error".into(), format!("{error:#}")));
            view.show_scene = false;
            view.status = "Failed · details in conversation".into();
            view.completion_locked = true;
        }
    }
    view.apply_runtime(project_runtime(harness).await);
    view.settle();
    Ok(CompletionState::Settled {
        quit: false,
        activity_closed: activity.closed,
    })
}

// Keep the event loop's individually borrowed state visible here instead of
// introducing a second owner for terminal, runtime, and task lifetimes.
#[allow(clippy::too_many_arguments)]
async fn cancel_operation(
    completions: &mut mpsc::Receiver<(u64, Result<DispatchOutcome>)>,
    events: &mut broadcast::Receiver<Event>,
    view: &mut View,
    harness: &Arc<Mutex<Harness>>,
    job: &mut Option<JoinHandle<()>>,
    cancellation: &mut Option<CancellationToken>,
    generation: &mut u64,
    quit_pending: &mut bool,
) -> Result<bool> {
    if let Some(cancellation) = cancellation.take() {
        cancellation.cancel();
    }
    if let Some(active_job) = job.take() {
        active_job
            .await
            .context("cancelled TUI dispatch task failed")?;
        let completion = completions
            .recv()
            .await
            .context("cancelled TUI job did not publish its settled result")?;
        let state =
            apply_completion(completion, *generation, events, view, harness, job, false).await?;
        *generation = generation.wrapping_add(1);
        *quit_pending = false;
        return Ok(matches!(
            state,
            CompletionState::Settled {
                activity_closed: true,
                ..
            }
        ));
    }
    let activity = drain_activity(events, view, ACTIVITY_DRAIN_CAP);
    *generation = generation.wrapping_add(1);
    *quit_pending = false;
    view.busy = false;
    view.settle();
    view.apply_runtime(project_runtime(harness).await);
    view.notice = None;
    view.status = "Cancelled · turn interrupted".into();
    view.completion_locked = true;
    Ok(activity.closed)
}

#[cfg(test)]
async fn run_loop_with_stream<B, S>(
    terminal: &mut Terminal<B>,
    harness: Harness,
    models: Vec<ModelInfo>,
    input: S,
) -> Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    S: Stream<Item = io::Result<TerminalEvent>> + Unpin,
{
    run_loop_with_stream_and_notice(terminal, harness, models, input, None).await
}

async fn run_loop_with_stream_and_notice<B, S>(
    terminal: &mut Terminal<B>,
    harness: Harness,
    models: Vec<ModelInfo>,
    mut input: S,
    mut notice: Option<MemoryNotice>,
) -> Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    S: Stream<Item = io::Result<TerminalEvent>> + Unpin,
{
    let initial = project_initial_view(&harness).await?;
    let mut view = View::from_initial(initial, models);
    if let Some(notice) = &notice {
        view.transcript
            .push(("system".into(), notice.text().into()));
        view.show_scene = false;
    }
    let mut events = harness.subscribe();
    let harness = Arc::new(Mutex::new(harness));
    let (tx, mut rx) = mpsc::channel::<(u64, Result<DispatchOutcome>)>(8);
    let mut job: Option<JoinHandle<()>> = None;
    let mut cancellation: Option<CancellationToken> = None;
    let mut quit_pending = false;
    let mut generation = 0u64;
    let started = Instant::now();
    let mut dirty = true;
    let mut scheduler = Scheduler::new();
    let mut input_open = true;
    let mut completion_open = true;
    let mut activity_open = true;
    let mut animation_at = TokioInstant::now();

    let result: Result<()> = async {
        loop {
            if dirty {
                terminal
                    .draw(|frame| draw(frame, &view))
                    .map_err(|error| anyhow::anyhow!("terminal draw: {error}"))?;
                dirty = false;
                if let Some(notice) = notice.take() {
                    // This frame contains the system transcript entry. Persist
                    // only after it completed and before input is admitted.
                    notice.record().await?;
                }
            }

            let wake = next_wake(
                &scheduler,
                &mut input,
                &mut rx,
                &mut events,
                WakeAvailability {
                    input: input_open,
                    completion: completion_open,
                    activity: activity_open,
                },
                animation_at,
            )
            .await;

            match wake {
                Wake::Terminal(Some(Ok(event))) => {
                    scheduler.served(WakeSource::Terminal);
                    let (redraw, command) = view.terminal_event(event);
                    dirty |= redraw;
                    if let Some(command) = command {
                        if command == "/cancel" {
                            activity_open = activity_still_open(
                                activity_open,
                                cancel_operation(
                                    &mut rx,
                                    &mut events,
                                    &mut view,
                                    &harness,
                                    &mut job,
                                    &mut cancellation,
                                    &mut generation,
                                    &mut quit_pending,
                                )
                                .await?,
                            );
                            dirty = true;
                        } else if command == "/quit" {
                            if let Some(cancellation) = cancellation.take() {
                                cancellation.cancel();
                            }
                            if let Some(job) = job.take() {
                                let _ = job.await;
                            }
                            view.begin_operation();
                            view.part_activity.clear();
                            view.routes.clear();
                            view.status = "Closing session".into();
                            quit_pending = true;
                            generation = generation.wrapping_add(1);
                            let harness = harness.clone();
                            let tx = tx.clone();
                            job = Some(tokio::spawn(async move {
                                let result = harness
                                    .lock()
                                    .await
                                    .shutdown(true)
                                    .await
                                    .map(|()| DispatchOutcome::Command(String::new()));
                                let _ = tx.send((generation, result)).await;
                            }));
                        } else if command == "/help" {
                            view.transcript.push(("help".into(), HELP.into()));
                            view.show_scene = false;
                        } else if !view.busy {
                            if command == "/model" {
                                view.open_picker(Picker::Models);
                                continue;
                            }
                            if command == "/effort" {
                                view.open_picker(Picker::Efforts);
                                continue;
                            }
                            if command == "/mode" {
                                view.open_picker(Picker::Modes);
                                continue;
                            }
                            if !command.starts_with('/') {
                                view.transcript.push(("user".into(), command.clone()));
                                view.show_scene = false;
                                view.scroll = 0;
                            }
                            view.begin_operation();
                            view.status = if command.starts_with('/') {
                                "Updating session"
                            } else {
                                "Listening to the parts"
                            }
                            .into();
                            view.part_activity.clear();
                            view.routes.clear();
                            generation = generation.wrapping_add(1);
                            let harness = harness.clone();
                            let tx = tx.clone();
                            let models = view.models.clone();
                            let operation_cancellation = CancellationToken::new();
                            cancellation = Some(operation_cancellation.clone());
                            job = Some(tokio::spawn(async move {
                                let result = dispatch_controlled(
                                    &mut *harness.lock().await,
                                    &models,
                                    &command,
                                    &operation_cancellation,
                                )
                                .await;
                                let _ = tx.send((generation, result)).await;
                            }));
                        }
                    }
                }
                Wake::Terminal(Some(Err(error))) => {
                    scheduler.served(WakeSource::Terminal);
                    return Err(error).context("terminal input");
                }
                Wake::Terminal(None) => {
                    scheduler.served(WakeSource::Terminal);
                    input_open = false;
                    anyhow::bail!("terminal input closed");
                }
                Wake::Completion(Some(completion)) => {
                    scheduler.served(WakeSource::Completion);
                    if completion.0 == generation {
                        cancellation = None;
                    }
                    match apply_completion(
                        completion,
                        generation,
                        &mut events,
                        &mut view,
                        &harness,
                        &mut job,
                        quit_pending,
                    )
                    .await?
                    {
                        CompletionState::Stale => {}
                        CompletionState::Settled {
                            quit: true,
                            activity_closed,
                        } => {
                            activity_open = activity_still_open(activity_open, activity_closed);
                            return Ok(());
                        }
                        CompletionState::Settled {
                            quit: false,
                            activity_closed,
                        } => {
                            activity_open = activity_still_open(activity_open, activity_closed);
                            dirty = true;
                        }
                    }
                }
                Wake::Completion(None) => {
                    scheduler.served(WakeSource::Completion);
                    completion_open = false;
                    if job.is_some() {
                        anyhow::bail!("TUI dispatch completion channel closed");
                    }
                }
                Wake::Activity(Ok(event)) => {
                    scheduler.served(WakeSource::Activity);
                    view.event(event);
                    dirty = true;
                }
                Wake::Activity(Err(broadcast::error::RecvError::Lagged(count))) => {
                    scheduler.served(WakeSource::Activity);
                    view.activity
                        .push(format!("{count} activity events omitted"));
                    dirty = true;
                }
                Wake::Activity(Err(broadcast::error::RecvError::Closed)) => {
                    scheduler.served(WakeSource::Activity);
                    activity_open = false;
                }
                Wake::Animation => {
                    scheduler.served(WakeSource::Animation);
                    dirty |= view.advance_animation(started.elapsed());
                    let delay = if view.busy {
                        Duration::from_millis(25)
                    } else {
                        Duration::from_millis(100)
                    };
                    animation_at = TokioInstant::now() + delay;
                }
            }
        }
    }
    .await;
    finish_loop(
        result,
        &mut job,
        &mut cancellation,
        &mut generation,
        &harness,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn dispatch(
    harness: &mut Harness,
    models: &[ModelInfo],
    command: &str,
) -> Result<DispatchOutcome> {
    let cancellation = CancellationToken::new();
    dispatch_controlled(harness, models, command, &cancellation).await
}

async fn dispatch_controlled(
    harness: &mut Harness,
    models: &[ModelInfo],
    command: &str,
    cancellation: &CancellationToken,
) -> Result<DispatchOutcome> {
    harness.reconcile().await?;
    let (name, args) = command.split_once(' ').unwrap_or((command, ""));
    let args = args.trim();
    let feedback = match name {
        "/parts" => serde_json::to_string_pretty(&harness.topology)?,
        "/mode" => {
            harness.set_mode(args.parse::<Mode>()?).await?;
            format!("Mode: {args}")
        }
        "/model" => {
            ensure!(!args.is_empty(), "model ID required");
            let effort = models
                .iter()
                .find(|m| m.id == args)
                .and_then(|m| m.default_effort.clone());
            harness.set_model(args, effort).await?;
            format!("Model: {args}")
        }
        "/effort" => {
            let effort = if args == "default" {
                None
            } else {
                ensure!(!args.is_empty(), "effort required");
                Some(args)
            };
            validate_effort(models, &harness.config.model, effort)?;
            harness.set_effort(effort.map(str::to_owned)).await?;
            format!("Effort: {args}")
        }
        "/focus" => {
            harness
                .focus(if args == "auto" { None } else { Some(args) })
                .await?;
            format!("Speaking focus: {args}")
        }
        "/relate" => {
            let (kind, members) = args
                .split_once(' ')
                .context("usage: /relate alliance ID,ID")?;
            let relation = harness
                .relate(
                    kind.parse()?,
                    members.split(',').map(|m| m.trim().to_owned()).collect(),
                )
                .await?;
            format!("Activated {} · {}", relation.kind, relation.id)
        }
        "/memory" => serde_json::to_string_pretty(&harness.memory_for(args).await?)?,
        "/notes" => serde_json::to_string_pretty(&harness.notes_for(args, 100).await?)?,
        "/memory-status" => serde_json::to_string_pretty(&harness.memory_status().await?)?,
        "/memory-history" => serde_json::to_string_pretty(&harness.memory_revisions(20).await?)?,
        "/retry" => {
            return Ok(DispatchOutcome::Turn(
                harness.retry_last(cancellation).await?,
            ));
        }
        "/dream" => serde_json::to_string_pretty(&harness.dream_controlled(cancellation).await?)?,
        "/undo-dream" => {
            harness.undo_dream().await?;
            "Previous membership restored.".into()
        }
        _ if command.starts_with('/') => anyhow::bail!("unknown command; use /help"),
        _ => {
            let turn_id = uuid::Uuid::new_v4().to_string();
            return Ok(DispatchOutcome::Turn(
                harness
                    .run_local_controlled(command, None, &turn_id, cancellation)
                    .await?,
            ));
        }
    };
    Ok(DispatchOutcome::Command(feedback))
}
#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::{Framework, RelationshipKind};
    use ratatui::backend::TestBackend;

    fn runtime_snapshot() -> RuntimeSnapshot {
        let framework = Framework::builtin(Mode::Ifs);
        RuntimeSnapshot {
            turns: 2,
            mode: "ifs".into(),
            model: "demo".into(),
            effort: "default".into(),
            parts: framework
                .parts
                .into_iter()
                .map(|part| (part.id, format!("{} · {}", part.name, part.role)))
                .collect(),
            relationships: vec![],
            focus: None,
        }
    }

    fn fixture() -> View {
        View::from_initial(
            InitialViewData {
                transcript: vec![],
                session: "plain-session".into(),
                project: "plain-project".into(),
                motion: true,
                runtime: runtime_snapshot(),
            },
            vec![ModelInfo {
                id: "demo".into(),
                name: "Demo".into(),
                efforts: vec!["low".into(), "high".into()],
                default_effort: Some("low".into()),
            }],
        )
    }

    #[test]
    fn interruption_role_projects_as_a_fixed_kuru_marker() {
        assert_eq!(
            project_transcript_message(INTERRUPTION_ROLE, INTERRUPTION_TEXT.into()),
            ("kuru".into(), INTERRUPTION_TEXT.into())
        );
        assert_eq!(
            project_transcript_message("user", "ordinary prompt".into()),
            ("user".into(), "ordinary prompt".into())
        );
    }

    #[test]
    fn marker_refresh_failure_adds_a_fixed_notice_without_replacing_turn_status() {
        let mut view = fixture();
        view.status = "Failed · details in conversation".into();
        present_interruption_refresh(&mut view, Err(anyhow::anyhow!("private storage detail")));
        assert_eq!(view.status, "Failed · details in conversation");
        assert_eq!(view.notice.as_deref(), Some(INTERRUPTION_REFRESH_NOTICE));
        assert!(!view.notice.as_deref().unwrap().contains("private storage"));
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scheduler_rotates_ready_sources_and_yields_to_siblings() {
        use futures::stream;

        let mut input = stream::repeat_with(|| Ok(TerminalEvent::Resize(80, 24)));
        let (completion_tx, mut completion_rx) = mpsc::channel(1);
        completion_tx
            .send((7, Ok(DispatchOutcome::Command("completed".into()))))
            .await
            .unwrap();
        let (activity_tx, mut activity_rx) = broadcast::channel(4);
        activity_tx
            .send(Event {
                kind: "active".into(),
                actor: "part".into(),
                detail: "ready".into(),
            })
            .unwrap();
        let mut scheduler = Scheduler::new();
        let deadline = TokioInstant::now() - Duration::from_millis(1);
        let (progress_tx, mut progress_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            progress_tx.send(()).await.unwrap();
        });

        let wake = next_wake(
            &scheduler,
            &mut input,
            &mut completion_rx,
            &mut activity_rx,
            WakeAvailability {
                input: true,
                completion: true,
                activity: true,
            },
            deadline,
        )
        .await;
        assert!(matches!(
            wake,
            Wake::Terminal(Some(Ok(TerminalEvent::Resize(80, 24))))
        ));
        assert!(
            progress_rx.try_recv().is_ok(),
            "production next_wake did not yield to a ready sibling"
        );
        scheduler.served(WakeSource::Terminal);

        let wake = next_wake(
            &scheduler,
            &mut input,
            &mut completion_rx,
            &mut activity_rx,
            WakeAvailability {
                input: true,
                completion: true,
                activity: true,
            },
            deadline,
        )
        .await;
        assert!(matches!(
            wake,
            Wake::Completion(Some((7, Ok(DispatchOutcome::Command(_)))))
        ));
        scheduler.served(WakeSource::Completion);

        let wake = next_wake(
            &scheduler,
            &mut input,
            &mut completion_rx,
            &mut activity_rx,
            WakeAvailability {
                input: true,
                completion: true,
                activity: true,
            },
            deadline,
        )
        .await;
        assert!(matches!(wake, Wake::Activity(Ok(Event { .. }))));
        scheduler.served(WakeSource::Activity);

        let wake = next_wake(
            &scheduler,
            &mut input,
            &mut completion_rx,
            &mut activity_rx,
            WakeAvailability {
                input: true,
                completion: true,
                activity: true,
            },
            deadline,
        )
        .await;
        assert!(matches!(wake, Wake::Animation));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queued_completion_wakes_without_waiting_for_the_animation_deadline() {
        use futures::stream;

        let mut input = stream::pending();
        let (completion_tx, mut completion_rx) = mpsc::channel(1);
        completion_tx
            .send((3, Ok(DispatchOutcome::Command("ready".into()))))
            .await
            .unwrap();
        let (_activity_tx, mut activity_rx) = broadcast::channel(1);
        let scheduler = Scheduler::new();
        let wake = next_wake(
            &scheduler,
            &mut input,
            &mut completion_rx,
            &mut activity_rx,
            WakeAvailability {
                input: true,
                completion: true,
                activity: true,
            },
            TokioInstant::now() + Duration::from_secs(60),
        )
        .await;
        assert!(matches!(
            wake,
            Wake::Completion(Some((3, Ok(DispatchOutcome::Command(_)))))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn closed_activity_keeps_completion_and_terminal_input_usable() {
        use futures::stream;

        let mut input = stream::iter([Ok(TerminalEvent::Resize(80, 24))]);
        let (completion_tx, mut completion_rx) = mpsc::channel(1);
        completion_tx
            .send((8, Ok(DispatchOutcome::Command("ready".into()))))
            .await
            .unwrap();
        let (activity_tx, mut activity_rx) = broadcast::channel(1);
        drop(activity_tx);
        let mut scheduler = Scheduler {
            first: WakeSource::Activity,
        };
        let animation_at = TokioInstant::now() + Duration::from_secs(60);

        let wake = next_wake(
            &scheduler,
            &mut input,
            &mut completion_rx,
            &mut activity_rx,
            WakeAvailability {
                input: true,
                completion: true,
                activity: true,
            },
            animation_at,
        )
        .await;
        assert!(matches!(
            wake,
            Wake::Activity(Err(broadcast::error::RecvError::Closed))
        ));
        scheduler.served(WakeSource::Activity);
        let activity_open = activity_still_open(true, true);
        assert!(!activity_open);

        let wake = next_wake(
            &scheduler,
            &mut input,
            &mut completion_rx,
            &mut activity_rx,
            WakeAvailability {
                input: true,
                completion: true,
                activity: activity_open,
            },
            animation_at,
        )
        .await;
        assert!(matches!(
            wake,
            Wake::Terminal(Some(Ok(TerminalEvent::Resize(80, 24))))
        ));
        scheduler.served(WakeSource::Terminal);

        let wake = next_wake(
            &scheduler,
            &mut input,
            &mut completion_rx,
            &mut activity_rx,
            WakeAvailability {
                input: true,
                completion: true,
                activity: activity_open,
            },
            animation_at,
        )
        .await;
        assert!(matches!(
            wake,
            Wake::Completion(Some((8, Ok(DispatchOutcome::Command(_)))))
        ));
    }

    #[test]
    fn activity_drain_is_capped_and_closure_never_reopens() {
        let mut view = fixture();
        let (activity_tx, mut activity_rx) = broadcast::channel(512);
        for index in 0..(ACTIVITY_DRAIN_CAP + 44) {
            activity_tx
                .send(Event {
                    kind: "tool".into(),
                    actor: "part".into(),
                    detail: format!("work-{index}"),
                })
                .unwrap();
        }
        let drained = drain_activity(&mut activity_rx, &mut view, ACTIVITY_DRAIN_CAP);
        assert!(drained.dirty);
        assert!(!drained.closed);
        assert_eq!(activity_rx.len(), 44);

        drop(activity_tx);
        let drained = drain_activity(&mut activity_rx, &mut view, ACTIVITY_DRAIN_CAP);
        assert!(drained.closed);
        let activity_open = activity_still_open(false, drained.closed);
        assert!(
            !activity_open,
            "a closed activity receiver must stay disabled"
        );
        assert!(
            !activity_still_open(activity_open, false),
            "a later completion or cancellation drain must not reopen activity"
        );
    }

    #[tokio::test]
    async fn abort_fence_awaits_owned_dispatch_cancellation() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        struct OnDrop(Arc<AtomicBool>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let observed = dropped.clone();
        let (ready, entered) = tokio::sync::oneshot::channel();
        let mut job = Some(tokio::spawn(async move {
            let _drop = OnDrop(observed);
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        }));
        entered.await.unwrap();
        let mut generation = 9;
        abort_and_fence(&mut job, &mut generation).await;
        assert_eq!(generation, 10);
        assert!(job.is_none());
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn terminal_dispatch_ignores_releases_without_losing_input_or_animation() {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let mut view = fixture();
        view.motion = true;
        assert!(view.advance_animation(Duration::from_millis(250)));
        let clock = (view.frame, view.clock_ms, view.last_frame_ms);
        let release = |code, modifiers| {
            TerminalEvent::Key(KeyEvent::new_with_kind(
                code,
                modifiers,
                KeyEventKind::Release,
            ))
        };

        for focused in [true, false] {
            let event = if focused {
                TerminalEvent::FocusGained
            } else {
                TerminalEvent::FocusLost
            };
            assert_eq!(view.terminal_event(event), (true, None));
            assert_eq!(view.focused, focused);
            for (kind, expected) in [(KeyEventKind::Press, "a"), (KeyEventKind::Repeat, "aa")] {
                let event = TerminalEvent::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('a'),
                    KeyModifiers::NONE,
                    kind,
                ));
                assert_eq!(view.terminal_event(event), (true, None));
                assert_eq!(view.input, expected);
                assert_eq!(view.cursor, expected.len());
                assert_eq!(
                    view.terminal_event(release(KeyCode::Char('a'), KeyModifiers::NONE)),
                    (false, None)
                );
                assert_eq!(view.input, expected);
                assert_eq!(view.cursor, expected.len());
            }
            assert_eq!(
                view.terminal_event(TerminalEvent::Key(key(KeyCode::Enter))),
                (true, Some("aa".into()))
            );
            assert_eq!(
                view.terminal_event(release(KeyCode::Enter, KeyModifiers::NONE)),
                (false, None)
            );
            assert!(view.input.is_empty());
            assert_eq!(view.cursor, 0);
            assert_eq!(
                view.terminal_event(TerminalEvent::Key(KeyEvent::new(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL,
                ))),
                (true, Some("/quit".into()))
            );
            assert_eq!(
                view.terminal_event(release(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                (false, None)
            );
            assert_eq!(view.focused, focused);
            assert_eq!((view.frame, view.clock_ms, view.last_frame_ms), clock);
        }

        assert_eq!(
            view.terminal_event(TerminalEvent::Paste("猫\nx".into())),
            (true, None)
        );
        assert_eq!(
            view.terminal_event(TerminalEvent::Resize(80, 24)),
            (true, None)
        );
        assert_eq!(
            view.terminal_event(TerminalEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 2,
                row: 3,
                modifiers: KeyModifiers::NONE,
            })),
            (false, None)
        );
        assert_eq!((view.input.as_str(), view.cursor), ("猫\nx", 5));
        assert_eq!((view.frame, view.clock_ms, view.last_frame_ms), clock);
        assert!(!view.focused);
        assert!(!view.advance_animation(Duration::from_millis(500)));
        assert_eq!(view.frame, clock.0);
        assert_eq!(
            view.terminal_event(TerminalEvent::FocusGained),
            (true, None)
        );
        assert!(view.advance_animation(Duration::from_millis(501)));
        assert!(!view.advance_animation(Duration::from_millis(750)));
        assert!(view.advance_animation(Duration::from_millis(751)));
    }

    #[test]
    fn ambient_clock_ignores_editing_and_preserves_busy_focus_and_static_behavior() {
        let mut view = fixture();
        view.motion = true;
        assert!(!view.advance_animation(Duration::from_millis(249)));
        assert!(view.advance_animation(Duration::from_millis(250)));
        assert!(view.advance_animation(Duration::from_secs(5)));
        let ambient_frame = view.frame;
        view.key(key(KeyCode::Char('a')));
        view.paste(" pasted text");
        assert_eq!(view.frame, ambient_frame);
        assert!(!view.advance_animation(Duration::from_millis(5080)));
        assert!(!view.advance_animation(Duration::from_millis(5249)));
        assert!(view.advance_animation(Duration::from_millis(5250)));
        view.begin_operation();
        assert!(view.advance_animation(Duration::from_millis(5330)));
        view.settle();
        view.busy = false;
        view.focused = false;
        assert!(!view.advance_animation(Duration::from_secs(7)));
        view.focused = true;
        assert!(view.advance_animation(Duration::from_secs(8)));
        view.motion = false;
        let still = view.frame;
        view.key(key(KeyCode::Char('b')));
        assert!(!view.advance_animation(Duration::from_secs(9)));
        assert_eq!(view.frame, still);
        // Accessibility pauses ornament; the actual elapsed operation clock remains useful.
        view.begin_operation();
        assert!(view.advance_animation(Duration::from_secs(10)));
        assert_eq!(view.operation_ms, 1000);
        view.settle();
        view.notify("Saved");
        assert!(view.advance_animation(Duration::from_secs(15)));
        assert!(view.notice.is_none());
        for value in ["1", "true", "TRUE", "yes", "on"] {
            assert!(reduced_motion(Some(value)));
        }
        for value in [None, Some("0"), Some("false"), Some("")] {
            assert!(!reduced_motion(value));
        }
    }

    #[test]
    fn activity_summarizes_routing_without_disclosing_peer_contents_or_state_notes() {
        let mut view = fixture();
        let from = view.parts[0].0.clone();
        let to = view.parts[1].0.clone();
        let envelope =
            kuru_runtime::PeerMessage::new(&from, &to, "session", "PRIVATE MESSAGE").unwrap();
        for _ in 0..8 {
            view.event(Event {
                kind: "peer".into(),
                actor: from.clone(),
                detail: envelope.rpc().to_string(),
            });
        }
        assert_eq!(view.routes.len(), 6);
        assert_eq!(view.routes.last(), Some(&(from.clone(), to.clone())));
        view.event(Event {
            kind: "state".into(),
            actor: to.clone(),
            detail: "{\"activation\":0.9,\"note\":\"PRIVATE NOTE\"}".into(),
        });
        let relation =
            Relationship::new(RelationshipKind::Alliance, vec![from.clone(), to.clone()]).unwrap();
        view.event(Event {
            kind: "relationship".into(),
            actor: from.clone(),
            detail: serde_json::to_string(&relation).unwrap(),
        });
        view.event(Event {
            kind: "speaker".into(),
            actor: relation.id.clone(),
            detail: "alliance".into(),
        });
        assert_eq!(view.relationships, vec![relation.clone()]);
        assert_eq!(view.focus.as_deref(), Some(relation.id.as_str()));
        assert_eq!(view.speaker_id, relation.id);
        let activity = view.activity.join("\n");
        assert!(!activity.contains("PRIVATE"));
        assert!(activity.contains(view.parts[1].1.split_once(" · ").unwrap().0));
        view.event(Event {
            kind: "peer".into(),
            actor: to.clone(),
            detail: "malformed PRIVATE MESSAGE".into(),
        });
        assert!(!view.activity.last().unwrap().contains("PRIVATE"));
        view.event(Event {
            kind: "active".into(),
            actor: to.clone(),
            detail: "round 1".into(),
        });
        view.settle();
        assert_eq!(view.part_activity[&to], "idle");
        let mut next = runtime_snapshot();
        next.parts.retain(|(id, _)| id != &from);
        view.apply_runtime(next);
        assert!(view.relationships.is_empty());
        assert!(view.routes.is_empty());
        assert!(!view.parts.iter().any(|(id, _)| id == &from));
    }

    #[test]
    fn response_activity_never_becomes_transcript_content() {
        let mut view = fixture();
        view.event(Event {
            kind: "response".into(),
            actor: "misleading-speaker".into(),
            detail: "BROADCAST_RESPONSE_MUST_NOT_BE_A_FINAL_ANSWER".into(),
        });

        assert!(view.transcript.is_empty());
        assert!(
            !view
                .activity
                .join("\n")
                .contains("BROADCAST_RESPONSE_MUST_NOT_BE_A_FINAL_ANSWER")
        );
    }

    #[test]
    fn completed_turn_keeps_returned_facts_when_activity_arrives_late() {
        let mut view = fixture();
        let members = view.parts[..2]
            .iter()
            .map(|part| part.0.clone())
            .collect::<Vec<_>>();
        let relation = Relationship::new(RelationshipKind::Alliance, members).unwrap();
        view.complete_turn(TurnOutput {
            session: view.session.clone(),
            speaker: relation.id.clone(),
            text: "AUTHORITATIVE_RESPONSE".into(),
            relationship: Some(relation.clone()),
            input_tokens: 13,
            output_tokens: 29,
            limited: true,
            limit_reasons: Some(vec![kuru_runtime::TurnLimitReason::PeerRounds]),
            response_outcome: Some(kuru_runtime::ResponseOutcome::Text),
            events: vec![],
        });
        view.event(Event {
            kind: "speaker".into(),
            actor: "misleading-speaker".into(),
            detail: "misleading speaker".into(),
        });
        view.event(Event {
            kind: "response".into(),
            actor: "misleading-speaker".into(),
            detail: "DUPLICATE_RESPONSE_MUST_STAY_ACTIVITY".into(),
        });

        assert_eq!(view.speaker_id, relation.id);
        assert!(view.speaker.starts_with("alliance · "));
        assert_eq!(
            view.transcript
                .iter()
                .filter(|(_, text)| text == "AUTHORITATIVE_RESPONSE")
                .count(),
            1
        );
        assert_eq!(
            view.completion_metadata,
            BTreeMap::from([(
                0,
                "13 input tokens · 29 output tokens · peer-round budget reached".into()
            )])
        );
        assert!(
            !view
                .transcript
                .iter()
                .any(|(_, text)| text == "DUPLICATE_RESPONSE_MUST_STAY_ACTIVITY")
        );
        assert!(
            view.activity
                .last()
                .is_some_and(|activity| activity.starts_with("response · "))
        );
        assert_eq!(view.status, "Complete · peer-round budget reached");
    }

    #[test]
    fn completion_metadata_stays_with_its_answer_at_wide_and_narrow_sizes() {
        let mut view = fixture();
        view.complete_turn(TurnOutput {
            session: view.session.clone(),
            speaker: view.parts[0].0.clone(),
            text: "COMPLETION_TEXT".into(),
            relationship: None,
            input_tokens: 8,
            output_tokens: 5,
            limited: true,
            limit_reasons: Some(vec![kuru_runtime::TurnLimitReason::ToolCalls]),
            response_outcome: Some(kuru_runtime::ResponseOutcome::Text),
            events: vec![],
        });

        for size in [(120, 35), (60, 20)] {
            let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
            terminal.draw(|frame| draw(frame, &view)).unwrap();
            let screen = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            let normalized = screen.split_whitespace().collect::<Vec<_>>().join(" ");
            let answer = normalized
                .find("COMPLETION_TEXT")
                .unwrap_or_else(|| panic!("{size:?} omitted the answer:\n{screen}"));
            let metadata = normalized
                .find("8 input tokens · 5 output tokens · tool-call budget reached")
                .unwrap_or_else(|| panic!("{size:?} omitted the answer metadata:\n{screen}"));
            assert!(
                answer < metadata,
                "{size:?} did not keep metadata after its answer:\n{screen}"
            );
        }
    }

    #[test]
    fn empty_and_legacy_outcomes_render_without_inventing_a_budget() {
        for (limited, reasons, response, expected, absent) in [
            (
                true,
                Some(vec![]),
                Some(kuru_runtime::ResponseOutcome::Empty),
                "empty response",
                "budget reached",
            ),
            (
                true,
                None,
                None,
                "legacy limit · cause unspecified",
                "empty response",
            ),
        ] {
            let mut view = fixture();
            view.complete_turn(TurnOutput {
                session: view.session.clone(),
                speaker: view.parts[0].0.clone(),
                text: "explanatory fallback".into(),
                relationship: None,
                input_tokens: 8,
                output_tokens: 0,
                limited,
                limit_reasons: reasons,
                response_outcome: response,
                events: vec![],
            });
            let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
            terminal.draw(|frame| draw(frame, &view)).unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains(expected), "{rendered}");
            assert!(!rendered.contains(absent), "{rendered}");
        }
    }

    #[test]
    fn unicode_editor_supports_midline_editing_navigation_newlines_and_send() {
        let mut view = fixture();
        for c in "a猫🌿".chars() {
            view.key(key(KeyCode::Char(c)));
        }
        assert_eq!(view.cursor, 8);
        view.key(key(KeyCode::Left));
        view.key(key(KeyCode::Backspace));
        assert_eq!(view.input, "a🌿");
        assert_eq!(view.cursor, 1);
        view.key(key(KeyCode::Delete));
        assert_eq!(view.input, "a");
        view.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        view.key(key(KeyCode::Char('b')));
        assert_eq!(view.input, "a\nb");
        view.key(key(KeyCode::Home));
        view.key(key(KeyCode::Right));
        assert_eq!(view.cursor, 1);
        view.key(key(KeyCode::End));
        assert_eq!(view.cursor, 3);
        assert_eq!(view.key(key(KeyCode::Enter)).unwrap(), "a\nb");
        assert!(view.input.is_empty());
        assert_eq!(view.cursor, 0);
        assert!(view.key(key(KeyCode::Enter)).is_none());
        view.key(key(KeyCode::PageUp));
        assert_eq!(view.scroll, 10);
        view.key(key(KeyCode::PageDown));
        assert_eq!(view.scroll, 0);
        view.busy = true;
        assert_eq!(view.key(key(KeyCode::Esc)).unwrap(), "/cancel");
        assert_eq!(
            view.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
                .unwrap(),
            "/cancel"
        );
        view.busy = false;
        assert_eq!(
            view.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
                .unwrap(),
            "/quit"
        );
        assert!(
            view.key(KeyEvent::new_with_kind(
                KeyCode::Char('x'),
                KeyModifiers::NONE,
                KeyEventKind::Release
            ))
            .is_none()
        );
    }

    #[test]
    fn pickers_select_models_efforts_modes_and_handle_empty_catalogs() {
        let mut view = fixture();
        for (function, expected) in [
            (2, "/model demo"),
            (3, "/effort low"),
            (4, "/mode polyvagal"),
        ] {
            view.key(key(KeyCode::F(function)));
            view.key(key(KeyCode::Down));
            assert_eq!(view.key(key(KeyCode::Enter)).unwrap(), expected);
        }
        view.key(key(KeyCode::F(4)));
        view.key(key(KeyCode::Down));
        view.key(key(KeyCode::Up));
        assert_eq!(view.selected, 0);
        view.key(key(KeyCode::Esc));
        assert!(view.picker.is_none());
        view.models.clear();
        view.key(key(KeyCode::F(2)));
        assert!(view.key(key(KeyCode::Enter)).is_none());
        view.key(key(KeyCode::Esc));
        view.key(key(KeyCode::F(3)));
        assert_eq!(view.options(), vec!["default"]);
        assert_eq!(view.key(key(KeyCode::Enter)).unwrap(), "/effort default");
    }

    #[test]
    fn picker_search_and_paste_preserve_drafts_and_busy_settings_are_explained() {
        let mut view = fixture();
        view.paste("An unsent 猫 draft");
        let draft = view.input.clone();
        view.key(key(KeyCode::F(4)));
        view.paste("JUN\n");
        assert_eq!(view.query, "JUN");
        assert_eq!(view.options(), vec!["jungian"]);
        assert_eq!(view.input, draft);
        view.key(key(KeyCode::Char('x')));
        assert!(view.options().is_empty());
        assert!(view.key(key(KeyCode::Enter)).is_none());
        view.key(key(KeyCode::Backspace));
        assert_eq!(
            view.key(key(KeyCode::Enter)).as_deref(),
            Some("/mode jungian")
        );
        assert_eq!(view.input, draft);
        view.begin_operation();
        view.key(key(KeyCode::F(2)));
        assert!(view.picker.is_none());
        assert!(view.notice.as_ref().unwrap().contains("current turn"));
        assert_eq!(view.input, draft);
    }

    #[test]
    fn rendering_handles_small_terminals_long_chats_activity_and_popups() {
        let mut view = fixture();
        for size in [(120, 35), (60, 20), (10, 5), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
            terminal.draw(|f| draw(f, &view)).unwrap();
        }
        view.transcript
            .push(("user".into(), "long text ".repeat(200)));
        view.input = "line one\n猫 line two".into();
        view.cursor = view.input.len();
        view.busy = true;
        for i in 0..110 {
            view.event(Event {
                kind: "tool".into(),
                actor: "peer".into(),
                detail: format!("call{i}"),
            });
        }
        assert_eq!(view.activity.len(), 100);
        view.complete_turn(TurnOutput {
            session: view.session.clone(),
            speaker: "speaker".into(),
            text: "Completed task".into(),
            relationship: None,
            input_tokens: 8,
            output_tokens: 5,
            limited: true,
            limit_reasons: Some(vec![kuru_runtime::TurnLimitReason::ToolCalls]),
            response_outcome: Some(kuru_runtime::ResponseOutcome::Text),
            events: vec![],
        });
        assert_eq!(view.transcript.last().unwrap().1, "Completed task");
        let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
        terminal.draw(|f| draw(f, &view)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("KURU"));
        assert!(text.contains("PARTS /"));
        assert!(text.contains("Completed task"));
        assert!(text.contains("8 input tokens"));
        assert!(text.contains("tool-call budget reached"));
        view.picker = Some(Picker::Models);
        terminal.draw(|f| draw(f, &view)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Models"));
        assert!(text.contains("Enter selects"));
    }

    #[test]
    fn applying_runtime_snapshot_preserves_initial_editor_and_completion_state() {
        let initial_runtime = runtime_snapshot();
        let original_parts = initial_runtime.parts.clone();
        let mut view = View::from_initial(
            InitialViewData {
                transcript: vec![("user".into(), "initial transcript".into())],
                session: "initial-session".into(),
                project: "initial-project".into(),
                motion: true,
                runtime: initial_runtime,
            },
            vec![],
        );
        view.input = "editor draft".into();
        view.cursor = view.input.len();
        view.complete_turn(TurnOutput {
            session: "initial-session".into(),
            speaker: original_parts[0].0.clone(),
            text: "typed completion".into(),
            relationship: None,
            input_tokens: 3,
            output_tokens: 5,
            limited: false,
            limit_reasons: Some(vec![]),
            response_outcome: Some(kuru_runtime::ResponseOutcome::Text),
            events: vec![],
        });
        let next_framework = Framework::builtin(Mode::Freudian);
        view.apply_runtime(RuntimeSnapshot {
            turns: 9,
            mode: "freudian".into(),
            model: "next-model".into(),
            effort: "high".into(),
            parts: next_framework
                .parts
                .into_iter()
                .map(|part| (part.id, format!("{} · {}", part.name, part.role)))
                .collect(),
            relationships: vec![],
            focus: None,
        });
        assert_eq!(
            view.transcript[0],
            ("user".into(), "initial transcript".into())
        );
        assert_eq!(view.transcript[1].1, "typed completion");
        assert_eq!(view.session, "initial-session");
        assert_eq!(view.project, "initial-project");
        assert!(view.motion);
        assert_eq!(view.input, "editor draft");
        assert_eq!(view.cursor, "editor draft".len());
        assert_eq!(
            view.completion_metadata[&1],
            "3 input tokens · 5 output tokens"
        );
        assert_eq!(view.mode, "freudian");
        assert_eq!(view.model, "next-model");
        assert_eq!(view.effort, "high");
        assert_eq!(view.turns, 9);
        assert_ne!(view.parts, original_parts);
    }
}
