use std::{
    io::{self, IsTerminal},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, ensure};
use crossterm::{
    event::{self, Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use kuru_core::{Mode, ModelInfo};
use kuru_runtime::{Event, Harness};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use tokio::{
    sync::{Mutex, broadcast, mpsc},
    task::JoinHandle,
};
use unicode_width::UnicodeWidthChar;

use crate::cli::validate_effort;

const HELP: &str = "Enter send · Alt+Enter newline · F2 models · F3 effort · F4 mode · Esc cancel\n/help · /parts · /mode ifs|polyvagal|freudian|jungian · /model ID · /effort LEVEL\n/focus NAME|ID|auto · /relate KIND ID,ID · /memory ID · /dream · /undo-dream · /quit";

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
        })
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

    pub fn event(&mut self, event: Event) {
        if event.kind == "speaker" {
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
            self.status = format!(
                "{} · {}",
                event.kind,
                event.actor.chars().take(8).collect::<String>()
            );
            self.activity
                .push(format!("{} {}", self.status, event.detail));
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
                self.picker = Some(Picker::Models);
                self.selected = 0;
            }
            KeyCode::F(3) => {
                self.picker = Some(Picker::Efforts);
                self.selected = 0;
            }
            KeyCode::F(4) => {
                self.picker = Some(Picker::Modes);
                self.selected = 0;
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

pub fn draw(frame: &mut ratatui::Frame<'_>, view: &View) {
    let area = frame.area();
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(5),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " KURU ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {} · {} · {}",
                view.mode, view.model, view.effort
            )),
        ])),
        rows[0],
    );
    let cols = Layout::horizontal(if area.width >= 90 {
        vec![Constraint::Min(40), Constraint::Length(32)]
    } else {
        vec![Constraint::Min(10), Constraint::Length(0)]
    })
    .split(rows[1]);
    let mut lines = vec![];
    if view.transcript.is_empty() {
        lines.push(Line::from(
            "A conversation with a pool of persistent peers.",
        ));
        lines.push(Line::from(
            "Type a task, or use /help to explore the parts.",
        ));
    }
    for (speaker, text) in &view.transcript {
        lines.push(Line::from(Span::styled(
            speaker,
            Style::default()
                .fg(if speaker == "user" {
                    Color::LightCyan
                } else {
                    Color::LightGreen
                })
                .add_modifier(Modifier::BOLD),
        )));
        lines.extend(text.lines().map(|line| Line::from(line.to_owned())));
        lines.push(Line::default());
    }
    let width = cols[0].width.saturating_sub(2).max(1) as usize;
    let line_count: usize = lines
        .iter()
        .map(|line| line.width().div_ceil(width).max(1))
        .sum();
    let offset = line_count
        .saturating_sub(cols[0].height.saturating_sub(2) as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((offset.saturating_sub(view.scroll), 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Conversation "),
            ),
        cols[0],
    );
    if cols[1].width > 0 {
        let sidebar = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(cols[1]);
        let parts = view
            .parts
            .iter()
            .map(|(id, name)| {
                ListItem::new(format!(
                    "{name}\n{}",
                    id.chars().take(8).collect::<String>()
                ))
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            List::new(parts).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Active parts "),
            ),
            sidebar[0],
        );
        let activity = view
            .activity
            .iter()
            .rev()
            .take(sidebar[1].height.saturating_sub(2) as usize)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        frame.render_widget(
            Paragraph::new(activity)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL).title(" Activity ")),
            sidebar[1],
        );
    }
    let (input_lines, cursor_x, cursor_y) = editor_layout(
        &view.input,
        view.cursor,
        rows[2].width.saturating_sub(2).max(1) as usize,
    );
    let input_scroll = cursor_y.saturating_sub(rows[2].height.saturating_sub(3) as usize);
    frame.render_widget(
        Paragraph::new(input_lines)
            .scroll((input_scroll.min(u16::MAX as usize) as u16, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightCyan))
                    .title(if view.busy {
                        " Compose next message · Esc cancels "
                    } else {
                        " Message · Enter sends · Alt+Enter newline "
                    }),
            ),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(format!(
            " {} │ {} │ {} │ F2 model · F3 effort · F4 mode",
            view.status,
            view.speaker,
            view.session.chars().take(8).collect::<String>()
        ))
        .style(Style::default().fg(Color::DarkGray)),
        rows[3],
    );
    if let Some(picker) = &view.picker {
        let options = view.options();
        let popup = centered(
            area,
            60.min(area.width),
            (options.len() as u16 + 2).min(area.height),
        );
        frame.render_widget(Clear, popup);
        let items = options
            .iter()
            .enumerate()
            .map(|(i, label)| {
                ListItem::new(label.clone()).style(if i == view.selected {
                    Style::default().bg(Color::LightCyan).fg(Color::Black)
                } else {
                    Style::default()
                })
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(view.selected));
        frame.render_stateful_widget(
            List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {picker:?} · Enter selects ")),
            ),
            popup,
            &mut state,
        );
    } else {
        frame.set_cursor_position((
            rows[2]
                .x
                .saturating_add(1 + cursor_x as u16)
                .min(area.right().saturating_sub(1)),
            rows[2]
                .y
                .saturating_add(1 + cursor_y.saturating_sub(input_scroll) as u16)
                .min(area.bottom().saturating_sub(1)),
        ));
    }
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

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
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
    loop {
        terminal
            .draw(|frame| draw(frame, &view))
            .map_err(|e| anyhow::anyhow!("terminal draw: {e}"))?;
        while let Ok((completed_generation, message)) = rx.try_recv() {
            if completed_generation != generation {
                continue;
            }
            view.busy = false;
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
            view.mode = h.config.mode.to_string();
            view.model = h.config.model.clone();
            view.effort = h.config.effort.clone().unwrap_or_else(|| "default".into());
            view.parts = h
                .topology
                .parts
                .iter()
                .filter(|p| p.active)
                .map(|p| (p.id.clone(), format!("{} · {}", p.name, p.role)))
                .collect();
        }
        loop {
            match events.try_recv() {
                Ok(event) => view.event(event),
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    view.activity.push(format!("{n} activity events omitted"))
                }
                Err(_) => break,
            }
        }
        if event::poll(Duration::from_millis(30))? {
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
                    view.status = "Cancelled".into();
                } else if command == "/quit" {
                    if let Some(job) = job.take() {
                        job.abort();
                        let _ = job.await;
                    }
                    view.busy = true;
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
                        view.picker = Some(Picker::Models);
                        view.selected = 0;
                        continue;
                    }
                    if command == "/effort" {
                        view.picker = Some(Picker::Efforts);
                        view.selected = 0;
                        continue;
                    }
                    if command == "/mode" {
                        view.picker = Some(Picker::Modes);
                        view.selected = 0;
                        continue;
                    }
                    if !command.starts_with('/') {
                        view.transcript.push(("user".into(), command.clone()));
                        view.scroll = 0;
                    }
                    view.busy = true;
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
