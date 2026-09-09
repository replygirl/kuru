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
use kuru_runtime::{Event, Harness};
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
pub use render::draw;

const HELP: &str = "Enter send · Alt+Enter newline · F2 models · F3 effort · F4 mode · F6 motion · Esc cancel\n/help · /parts · /mode ifs|polyvagal|freudian|jungian · /model ID · /effort LEVEL\n/focus NAME|ID|auto · /relate KIND ID,ID · /memory ID · /dream · /undo-dream · /quit";

#[derive(Debug, Clone, PartialEq)]
pub enum Picker {
    Models,
    Efforts,
    Modes,
}

#[derive(Debug, Clone)]
pub struct View {
    pub transcript: Vec<(String, String)>,
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
}

impl View {
    pub fn new(harness: &Harness, models: Vec<ModelInfo>) -> Result<Self> {
        Ok(Self {
            transcript: harness
                .history()?
                .into_iter()
                .map(|m| (m.role, m.content))
                .collect(),
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
        })
    }

    /// Advance only while working or during the finite welcome sequence.
    /// Elapsed time is supplied by the loop so rendering remains deterministic.
    pub fn advance_animation(&mut self, elapsed: Duration) -> bool {
        let animated = self.motion
            && (self.busy || (self.transcript.is_empty() && elapsed < Duration::from_secs(4)));
        let next = (elapsed.as_millis() / 80).min(u64::MAX as u128) as u64;
        if animated && next != self.frame {
            self.frame = next;
            return true;
        }
        false
    }

    fn refresh(&mut self, harness: &Harness) {
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
        for phase in self.part_activity.values_mut() {
            if phase != "error" {
                *phase = "idle".into();
            }
        }
    }

    pub fn options(&self) -> Vec<String> {
        match self.picker {
            Some(Picker::Models) => self.models.iter().map(|m| m.id.clone()).collect(),
            Some(Picker::Efforts) => self
                .models
                .iter()
                .find(|m| m.id == self.model)
                .map(|m| m.efforts.clone())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| vec!["default".into()]),
            Some(Picker::Modes) => ["ifs", "polyvagal", "freudian", "jungian"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            None => vec![],
        }
    }

    fn open_picker(&mut self, picker: Picker) {
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
        if event.kind == "speaker" {
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
            self.transcript.push((self.speaker.clone(), event.detail));
        } else {
            self.status = format!("{} · {}", event.kind, self.actor_name(&event.actor));
            let detail: String = detail
                .chars()
                .filter(|c| !c.is_control())
                .take(140)
                .collect();
            self.activity.push(format!("{} · {}", self.status, detail));
            if self.activity.len() > 100 {
                self.activity.remove(0);
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Option<String> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Some(if self.busy { "/cancel" } else { "/quit" }.into());
        }
        if key.code == KeyCode::F(6) {
            self.motion = !self.motion;
            self.frame = 0;
            return None;
        }
        if self.picker.is_some() {
            let len = self.options().len();
            match key.code {
                KeyCode::Esc => self.picker = None,
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => self.selected = (self.selected + 1).min(len.saturating_sub(1)),
                KeyCode::Enter => {
                    let value = self.options().get(self.selected).cloned();
                    let command = match self.picker.take() {
                        Some(Picker::Models) => "/model",
                        Some(Picker::Efforts) => "/effort",
                        _ => "/mode",
                    };
                    return value.map(|v| format!("{command} {v}"));
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

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            event::DisableBracketedPaste
        );
    }
}

pub async fn run(harness: Harness, models: Vec<ModelInfo>) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "interactive mode requires a terminal; use kuru run PROMPT"
    );
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        event::EnableBracketedPaste
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    run_loop(&mut terminal, harness, models).await
}

async fn run_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    harness: Harness,
    models: Vec<ModelInfo>,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let mut view = View::new(&harness, models)?;
    let mut events = harness.subscribe();
    let harness = Arc::new(Mutex::new(harness));
    let (tx, mut rx) = mpsc::channel::<(u64, Result<String>)>(8);
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
            view.busy = false;
            dirty = true;
            job = None;
            if quit_pending {
                message?;
                return Ok(());
            }
            match message {
                Ok(text) => {
                    if !text.is_empty() {
                        view.transcript.push(("kuru".into(), text));
                    }
                    view.status = "Ready".into();
                }
                Err(error) => {
                    view.transcript.push(("error".into(), format!("{error:#}")));
                    view.status = "Ready · previous operation failed".into();
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
        let wait = if view.busy || (view.motion && started.elapsed() < Duration::from_secs(4)) {
            Duration::from_millis(25)
        } else {
            Duration::from_millis(100)
        };
        if event::poll(wait)? {
            dirty = true;
            let command = match event::read()? {
                TerminalEvent::Key(key) => view.key(key),
                TerminalEvent::Paste(text) => {
                    for c in text.chars() {
                        view.insert(c);
                    }
                    None
                }
                _ => None,
            };
            if let Some(command) = command {
                if command == "/cancel" {
                    if let Some(job) = job.take() {
                        job.abort();
                        let _ = job.await;
                    }
                    generation += 1;
                    quit_pending = false;
                    view.busy = false;
                    view.settle();
                    view.refresh(&*harness.lock().await);
                    view.status = "Cancelled".into();
                } else if command == "/quit" {
                    if let Some(job) = job.take() {
                        job.abort();
                        let _ = job.await;
                    }
                    view.busy = true;
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
                            .map(|()| String::new());
                        let _ = tx.send((generation, result)).await;
                    }));
                } else if command == "/help" {
                    view.transcript.push(("help".into(), HELP.into()));
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
                        view.scroll = 0;
                    }
                    view.busy = true;
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

pub async fn dispatch(
    harness: &mut Harness,
    models: &[ModelInfo],
    command: &str,
) -> Result<String> {
    let (name, args) = command.split_once(' ').unwrap_or((command, ""));
    let args = args.trim();
    match name {
        "/parts" => Ok(serde_json::to_string_pretty(&harness.topology)?),
        "/mode" => {
            harness.set_mode(args.parse::<Mode>()?)?;
            Ok(format!("Mode: {args}"))
        }
        "/model" => {
            ensure!(!args.is_empty(), "model ID required");
            harness.config.model = args.into();
            harness.config.effort = models
                .iter()
                .find(|m| m.id == args)
                .and_then(|m| m.default_effort.clone());
            Ok(format!("Model: {args}"))
        }
        "/effort" => {
            let effort = if args == "default" {
                None
            } else {
                ensure!(!args.is_empty(), "effort required");
                Some(args)
            };
            validate_effort(models, &harness.config.model, effort)?;
            harness.config.effort = effort.map(str::to_owned);
            Ok(format!("Effort: {args}"))
        }
        "/focus" => {
            harness.focus(if args == "auto" { None } else { Some(args) })?;
            Ok(format!("Speaking focus: {args}"))
        }
        "/relate" => {
            let (kind, members) = args
                .split_once(' ')
                .context("usage: /relate alliance ID,ID")?;
            let relation = harness.relate(
                kind.parse()?,
                members.split(',').map(|m| m.trim().to_owned()).collect(),
            )?;
            Ok(format!("Activated {} · {}", relation.kind, relation.id))
        }
        "/memory" => Ok(serde_json::to_string_pretty(&harness.memory_for(args)?)?),
        "/dream" => Ok(serde_json::to_string_pretty(&harness.dream().await?)?),
        "/undo-dream" => {
            harness.undo_dream()?;
            Ok("Previous membership restored.".into())
        }
        _ if command.starts_with('/') => anyhow::bail!("unknown command; use /help"),
        _ => {
            harness.run(command).await?;
            Ok(String::new())
        }
    }
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_connectors::DemoProvider;
    use kuru_core::{Config, MemoryStore};
    use ratatui::backend::TestBackend;

    fn fixture() -> (tempfile::TempDir, Harness, Vec<ModelInfo>) {
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
            MemoryStore::in_memory().unwrap(),
            Arc::new(DemoProvider),
            None,
        )
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

    #[tokio::test]
    async fn animation_has_a_fixed_cadence_finite_welcome_and_static_reduced_mode() {
        let (_dir, h, models) = fixture();
        let mut view = View::new(&h, models).unwrap();
        view.motion = true;
        assert!(!view.advance_animation(Duration::from_millis(79)));
        assert!(view.advance_animation(Duration::from_millis(80)));
        assert_eq!(view.frame, 1);
        assert!(!view.advance_animation(Duration::from_millis(159)));
        assert!(!view.advance_animation(Duration::from_secs(4)));
        assert_eq!(view.frame, 1);
        view.transcript.push(("user".into(), "a task".into()));
        assert!(!view.advance_animation(Duration::from_secs(5)));
        view.busy = true;
        assert!(view.advance_animation(Duration::from_secs(5)));
        view.key(key(KeyCode::F(6)));
        assert!(!view.motion);
        assert_eq!(view.frame, 0);
        assert!(!view.advance_animation(Duration::from_secs(6)));
        assert_eq!(view.key(key(KeyCode::Esc)).as_deref(), Some("/cancel"));
        view.key(key(KeyCode::F(6)));
        assert!(view.advance_animation(Duration::from_secs(7)));
        for value in ["1", "true", "TRUE", "yes", "on"] {
            assert!(reduced_motion(Some(value)));
        }
        for value in [None, Some("0"), Some("false"), Some("")] {
            assert!(!reduced_motion(value));
        }
    }

    #[tokio::test]
    async fn activity_summarizes_routing_without_disclosing_peer_contents_or_state_notes() {
        let (_dir, mut h, models) = fixture();
        let mut view = View::new(&h, models).unwrap();
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
    async fn unicode_editor_supports_midline_editing_navigation_newlines_and_send() {
        let (_dir, h, models) = fixture();
        let mut view = View::new(&h, models).unwrap();
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
        let (_dir, h, models) = fixture();
        let mut view = View::new(&h, models).unwrap();
        for (function, expected) in [
            (2, "/model demo"),
            (3, "/effort high"),
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
        view.key(key(KeyCode::F(3)));
        assert_eq!(view.options(), vec!["default"]);
        assert_eq!(view.key(key(KeyCode::Enter)).unwrap(), "/effort default");
    }

    #[tokio::test]
    async fn rendering_handles_small_terminals_long_chats_activity_and_popups() {
        let (_dir, h, models) = fixture();
        let mut view = View::new(&h, models).unwrap();
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
        view.event(Event {
            kind: "response".into(),
            actor: "speaker".into(),
            detail: "Completed task".into(),
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
        assert!(text.contains("Active parts"));
        assert!(text.contains("Completed task"));
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
        let (_dir, mut h, models) = fixture();
        assert!(
            dispatch(&mut h, &models, "/parts")
                .await
                .unwrap()
                .contains("manager")
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
        dispatch(&mut h, &models, "hello").await.unwrap();
        assert!(
            dispatch(&mut h, &models, &format!("/memory {}", ids[0]))
                .await
                .unwrap()
                .contains("hello")
        );
        assert!(
            dispatch(&mut h, &models, "/dream")
                .await
                .unwrap()
                .contains("summaries")
        );
        h.apply_dream(vec![kuru_runtime::DreamProposal::Add {
            name: "Extra".into(),
            role: h.topology.parts[0].role.clone(),
            instruction: "Complement".into(),
        }])
        .unwrap();
        dispatch(&mut h, &models, "/undo-dream").await.unwrap();
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
