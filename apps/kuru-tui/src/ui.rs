#[cfg(test)]
use kuru_memory::MemoryStore;
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use crossterm::{
    event::{self, Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use kuru_core::{Mode, ModelInfo, Relationship};
use kuru_runtime::{Event, Harness, TurnOutput};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    text::Line,
};
use tokio::{
    sync::{Mutex, broadcast, mpsc},
    task::JoinHandle,
};
use unicode_width::UnicodeWidthChar;

use crate::cli::validate_effort;

mod render;
mod scene;
pub use render::draw;

const HELP: &str = "Enter send · Alt+Enter newline · F2 models · F3 effort · F4 mode · Esc cancel\n/help · /parts · /mode ifs|polyvagal|freudian|jungian · /model ID · /effort LEVEL\n/focus NAME|ID|auto · /relate KIND ID,ID · /memory ID · /dream · /undo-dream · /quit\n/memory-status · /memory-history\nModel, effort and mode selections are remembered for this project.";

#[derive(Debug, Clone, PartialEq)]
pub enum Picker {
    Models,
    Efforts,
    Modes,
}

#[derive(Debug)]
pub(crate) enum DispatchOutcome {
    Command(String),
    Turn(TurnOutput),
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
    pub async fn new(harness: &Harness, models: Vec<ModelInfo>) -> Result<Self> {
        let transcript: Vec<_> = harness
            .history()
            .await?
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect();
        let show_scene = transcript.is_empty();
        Ok(Self {
            completion_metadata: BTreeMap::new(),
            transcript,
            input: String::new(),
            cursor: 0,
            mode: harness.config.mode.to_string(),
            model: harness.config.model.clone(),
            effort: harness
                .config
                .effort
                .clone()
                .unwrap_or_else(|| "default".into()),
            session: harness.session.id.clone(),
            parts: harness
                .topology
                .parts
                .iter()
                .filter(|p| p.active)
                .map(|p| (p.id.clone(), format!("{} · {}", p.name, p.role)))
                .collect(),
            activity: vec![],
            busy: false,
            status: "Ready · /help for commands".into(),
            speaker: "pool".into(),
            picker: None,
            selected: 0,
            models,
            scroll: 0,
            frame: 0,
            motion: !reduced_motion(std::env::var("KURU_REDUCED_MOTION").ok().as_deref()),
            part_activity: BTreeMap::new(),
            relationships: live_relationships(harness),
            focus: harness.topology.focus.as_ref().map(|f| f.id.clone()),
            speaker_id: String::new(),
            routes: vec![],
            project: harness
                .cwd()
                .file_name()
                .unwrap_or(harness.cwd().as_os_str())
                .to_string_lossy()
                .into_owned(),
            turns: harness.session.turns,
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
        })
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

    fn refresh(&mut self, harness: &Harness) {
        self.turns = harness.session.turns;
        self.mode = harness.config.mode.to_string();
        self.model = harness.config.model.clone();
        self.effort = harness
            .config
            .effort
            .clone()
            .unwrap_or_else(|| "default".into());
        self.parts = harness
            .topology
            .parts
            .iter()
            .filter(|p| p.active)
            .map(|p| (p.id.clone(), format!("{} · {}", p.name, p.role)))
            .collect();
        self.relationships = live_relationships(harness);
        self.focus = harness.topology.focus.as_ref().map(|f| f.id.clone());
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

    fn complete_turn(&mut self, output: TurnOutput) {
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
        let limited = if output.limited {
            " · limited result"
        } else {
            ""
        };
        self.completion_metadata.insert(
            index,
            format!(
                "{} input tokens · {} output tokens{limited}",
                output.input_tokens, output.output_tokens
            ),
        );
        self.show_scene = false;
        self.status = if output.limited {
            "Complete · limited result"
        } else {
            "Complete"
        }
        .into();
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

fn reduced_motion(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
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
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "interactive mode requires a terminal; use kuru run PROMPT"
    );
    let mut guard = TerminalSession::enter(&mut io::stdout())?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let result = run_loop(&mut terminal, harness, models).await;
    // Ratatui's Drop may show its cursor. Finish that while terminal output
    // processing is still active, before restoring the caller's console modes.
    drop(terminal);
    let restored = guard.restore();
    match result {
        Ok(()) => restored,
        Err(error) => Err(error),
    }
}

async fn run_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    harness: Harness,
    models: Vec<ModelInfo>,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let mut view = View::new(&harness, models).await?;
    let mut events = harness.subscribe();
    let harness = Arc::new(Mutex::new(harness));
    let (tx, mut rx) = mpsc::channel::<(u64, Result<DispatchOutcome>)>(8);
    let mut job: Option<JoinHandle<()>> = None;
    let mut quit_pending = false;
    let mut generation = 0u64;
    let started = Instant::now();
    let mut dirty = true;
    loop {
        loop {
            match events.try_recv() {
                Ok(event) => {
                    view.event(event);
                    dirty = true;
                }
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    view.activity.push(format!("{n} activity events omitted"));
                    dirty = true;
                }
                Err(_) => break,
            }
        }
        while let Ok((completed_generation, message)) = rx.try_recv() {
            if completed_generation != generation {
                continue;
            }
            while let Ok(event) = events.try_recv() {
                view.event(event);
            }
            view.busy = false;
            dirty = true;
            job = None;
            if quit_pending {
                message?;
                return Ok(());
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
                Ok(DispatchOutcome::Turn(output)) => view.complete_turn(output),
                Err(error) => {
                    view.transcript.push(("error".into(), format!("{error:#}")));
                    view.show_scene = false;
                    view.status = "Failed · details in conversation".into();
                    view.completion_locked = true;
                }
            }
            let h = harness.lock().await;
            view.refresh(&h);
            view.settle();
        }
        dirty |= view.advance_animation(started.elapsed());
        if dirty {
            terminal
                .draw(|frame| draw(frame, &view))
                .map_err(|e| anyhow::anyhow!("terminal draw: {e}"))?;
            dirty = false;
        }
        let wait = if view.busy {
            Duration::from_millis(25)
        } else {
            Duration::from_millis(100)
        };
        if event::poll(wait)? {
            let (redraw, command) = view.terminal_event(event::read()?);
            dirty |= redraw;
            if let Some(command) = command {
                if command == "/cancel" {
                    if let Some(job) = job.take() {
                        job.abort();
                        let _ = job.await;
                    }
                    while let Ok(event) = events.try_recv() {
                        view.event(event);
                    }
                    generation += 1;
                    quit_pending = false;
                    view.busy = false;
                    view.settle();
                    view.refresh(&*harness.lock().await);
                    view.notice = None;
                    view.status = "Cancelled · turn interrupted".into();
                    view.completion_locked = true;
                } else if command == "/quit" {
                    if let Some(job) = job.take() {
                        job.abort();
                        let _ = job.await;
                    }
                    view.begin_operation();
                    view.part_activity.clear();
                    view.routes.clear();
                    view.status = "Closing session".into();
                    quit_pending = true;
                    generation += 1;
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
                    generation += 1;
                    let harness = harness.clone();
                    let tx = tx.clone();
                    let models = view.models.clone();
                    job = Some(tokio::spawn(async move {
                        let result = dispatch(&mut *harness.lock().await, &models, &command).await;
                        let _ = tx.send((generation, result)).await;
                    }));
                }
            }
        }
        tokio::task::yield_now().await;
    }
}

pub(crate) async fn dispatch(
    harness: &mut Harness,
    models: &[ModelInfo],
    command: &str,
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
        "/memory-status" => serde_json::to_string_pretty(&harness.memory_status().await?)?,
        "/memory-history" => serde_json::to_string_pretty(&harness.memory_revisions(20).await?)?,
        "/dream" => serde_json::to_string_pretty(&harness.dream().await?)?,
        "/undo-dream" => {
            harness.undo_dream().await?;
            "Previous membership restored.".into()
        }
        _ if command.starts_with('/') => anyhow::bail!("unknown command; use /help"),
        _ => return Ok(DispatchOutcome::Turn(harness.run(command).await?)),
    };
    Ok(DispatchOutcome::Command(feedback))
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_connectors::DemoProvider;
    use kuru_core::Config;
    use ratatui::backend::TestBackend;

    async fn fixture() -> (tempfile::TempDir, Harness, Vec<ModelInfo>) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        };
        let h = Harness::new(
            config,
            dir.path(),
            MemoryStore::temporary().await.unwrap(),
            Arc::new(DemoProvider),
            None,
        )
        .await
        .unwrap();
        let models = vec![ModelInfo {
            id: "demo".into(),
            name: "Demo".into(),
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("low".into()),
        }];
        (dir, h, models)
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn command_text(outcome: DispatchOutcome) -> String {
        match outcome {
            DispatchOutcome::Command(text) => text,
            DispatchOutcome::Turn(_) => panic!("expected command feedback"),
        }
    }

    #[tokio::test]
    async fn terminal_dispatch_ignores_releases_without_losing_input_or_animation() {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let (_dir, h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
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

    #[tokio::test]
    async fn ambient_clock_ignores_editing_and_preserves_busy_focus_and_static_behavior() {
        let (_dir, h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
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

    #[tokio::test]
    async fn activity_summarizes_routing_without_disclosing_peer_contents_or_state_notes() {
        let (_dir, mut h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
        let from = h.topology.parts[0].id.clone();
        let to = h.topology.parts[1].id.clone();
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
        let relation = h
            .relate(
                kuru_core::RelationshipKind::Alliance,
                vec![from.clone(), to.clone()],
            )
            .await
            .unwrap();
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
        assert!(activity.contains(&h.topology.parts[1].name));
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
        h.topology.parts[0].active = false;
        view.refresh(&h);
        assert!(view.relationships.is_empty());
        assert!(view.routes.is_empty());
        assert!(!view.parts.iter().any(|(id, _)| id == &from));
    }

    #[tokio::test]
    async fn response_activity_never_becomes_transcript_content() {
        let (_dir, h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
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

    #[tokio::test]
    async fn completed_turn_keeps_returned_facts_when_activity_arrives_late() {
        let (_dir, mut h, models) = fixture().await;
        let members = h.topology.parts[..2]
            .iter()
            .map(|part| part.id.clone())
            .collect::<Vec<_>>();
        let relation = h
            .relate(kuru_core::RelationshipKind::Alliance, members)
            .await
            .unwrap();
        let mut view = View::new(&h, models).await.unwrap();
        view.complete_turn(TurnOutput {
            session: h.session.id.clone(),
            speaker: relation.id.clone(),
            text: "AUTHORITATIVE_RESPONSE".into(),
            relationship: Some(relation.clone()),
            input_tokens: 13,
            output_tokens: 29,
            limited: true,
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
                "13 input tokens · 29 output tokens · limited result".into()
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
        assert_eq!(view.status, "Complete · limited result");
    }

    #[tokio::test]
    async fn completion_metadata_stays_with_its_answer_at_wide_and_narrow_sizes() {
        let (_dir, h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
        view.complete_turn(TurnOutput {
            session: h.session.id.clone(),
            speaker: h.topology.parts[0].id.clone(),
            text: "COMPLETION_TEXT".into(),
            relationship: None,
            input_tokens: 8,
            output_tokens: 5,
            limited: true,
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
            for expected in [
                "COMPLETION_TEXT",
                "8 input tokens",
                "5 output tokens",
                "limited result",
            ] {
                assert!(
                    screen.contains(expected),
                    "{size:?} omitted {expected}:\n{screen}"
                );
            }
        }
    }

    #[tokio::test]
    async fn unicode_editor_supports_midline_editing_navigation_newlines_and_send() {
        let (_dir, h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
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

    #[tokio::test]
    async fn pickers_select_models_efforts_modes_and_handle_empty_catalogs() {
        let (_dir, h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
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

    #[tokio::test]
    async fn picker_search_and_paste_preserve_drafts_and_busy_settings_are_explained() {
        let (_dir, h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
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

    #[tokio::test]
    async fn rendering_handles_small_terminals_long_chats_activity_and_popups() {
        let (_dir, h, models) = fixture().await;
        let mut view = View::new(&h, models).await.unwrap();
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
            session: h.session.id.clone(),
            speaker: "speaker".into(),
            text: "Completed task".into(),
            relationship: None,
            input_tokens: 8,
            output_tokens: 5,
            limited: true,
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
        assert!(text.contains("limited result"));
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

    #[tokio::test]
    async fn slash_commands_change_real_runtime_state_and_validate_errors() {
        let (_dir, mut h, models) = fixture().await;
        assert!(
            command_text(dispatch(&mut h, &models, "/parts").await.unwrap()).contains("manager")
        );
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
        assert!(matches!(
            dispatch(&mut h, &models, "hello").await.unwrap(),
            DispatchOutcome::Turn(_)
        ));
        assert!(
            command_text(
                dispatch(&mut h, &models, &format!("/memory {}", ids[0]))
                    .await
                    .unwrap()
            )
            .contains("hello")
        );
        assert!(
            command_text(dispatch(&mut h, &models, "/dream").await.unwrap()).contains("summaries")
        );
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
}
