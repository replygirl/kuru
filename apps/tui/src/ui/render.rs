//! Pure terminal presentation. Runtime state supplies every activity label and edge.

use std::cell::RefCell;

use kuru_core::RelationshipKind;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{Picker, View, editor_layout};

const INK: Color = Color::Rgb(15, 19, 30);
const SURFACE: Color = Color::Rgb(21, 27, 42);
const RAISED: Color = Color::Rgb(30, 38, 57);
const EDGE: Color = Color::Rgb(57, 69, 92);
const TEXT: Color = Color::Rgb(222, 231, 244);
const MUTED: Color = Color::Rgb(145, 161, 184);
const MINT: Color = Color::Rgb(131, 231, 199);
const LILAC: Color = Color::Rgb(193, 166, 247);
const AMBER: Color = Color::Rgb(241, 200, 129);
const BLUE: Color = Color::Rgb(135, 191, 250);
const ROSE: Color = Color::Rgb(242, 149, 173);
const PALETTE: [Color; 6] = [MINT, BLUE, LILAC, ROSE, AMBER, LILAC];
const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

#[derive(Default)]
struct TranscriptCache {
    source: Vec<(String, String)>,
    width: u16,
    lines: Vec<Line<'static>>,
}

thread_local! {
    // One transcript per terminal thread, bounded by the public transcript itself.
    // Comparing strings is cheap; only changed content or widths rebuild layout.
    static TRANSCRIPT: RefCell<TranscriptCache> = RefCell::default();
}

fn style(color: Color) -> Style {
    Style::default().fg(color)
}

fn bold(color: Color) -> Style {
    style(color).add_modifier(Modifier::BOLD)
}

fn panel(title: impl Into<Line<'static>>, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style(EDGE))
        .style(style(TEXT).bg(SURFACE))
        .title(title.into().style(bold(color)))
}

fn inset(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(horizontal.min(area.width)),
        area.y.saturating_add(vertical.min(area.height)),
        area.width.saturating_sub(horizontal.saturating_mul(2)),
        area.height.saturating_sub(vertical.saturating_mul(2)),
    )
}

fn clipped(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for character in text.chars() {
        let cells = character.width().unwrap_or(0);
        if used + cells > width - 1 {
            break;
        }
        result.push(character);
        used += cells;
    }
    result.push('…');
    result
}

fn short_name(view: &View, id: &str) -> String {
    view.parts
        .iter()
        .find(|(part, _)| part == id)
        .map(|(_, label)| label.split(" · ").next().unwrap_or(label).to_owned())
        .unwrap_or_else(|| id.chars().take(8).collect())
}

fn identity_color(label: &str) -> Color {
    let role = label.rsplit(" · ").next().unwrap_or(label);
    match role {
        "self" | "ego" | "ventral" | "ventral_vagal" => MINT,
        "manager" | "persona" | "superego" => BLUE,
        "firefighter" | "id" | "sympathetic" => AMBER,
        "exile" | "shadow" | "dorsal" | "dorsal_vagal" => LILAC,
        _ => PALETTE[role.bytes().map(usize::from).sum::<usize>() % PALETTE.len()],
    }
}

fn relationship_color(kind: RelationshipKind) -> Color {
    match kind {
        RelationshipKind::Protection => BLUE,
        RelationshipKind::Polarization => AMBER,
        RelationshipKind::Alliance => LILAC,
    }
}

fn phase(view: &View) -> usize {
    if view.motion { view.frame as usize } else { 0 }
}

fn spinner(view: &View) -> &'static str {
    if view.motion {
        SPINNER[phase(view) % SPINNER.len()]
    } else {
        "◌"
    }
}

/// Draw from presentation state only; no timers, random values or runtime work.
pub fn draw(frame: &mut Frame<'_>, view: &View) {
    let area = frame.area();
    frame.render_widget(Block::default().style(style(TEXT).bg(INK)), area);
    if area.width == 0 || area.height == 0 {
        return;
    }
    if area.height < 5 || area.width < 8 {
        draw_tiny(frame, view, area);
        return;
    }
    let header_height = if area.height >= 14 { 3 } else { 1 };
    let composer_height = if area.height >= 16 { 5 } else { 3 };
    let rows = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(0),
        Constraint::Length(composer_height),
        Constraint::Length(1),
    ])
    .split(area);
    draw_header(frame, view, rows[0]);
    let show_sidebar = area.width >= 96 && rows[1].height >= 12;
    let columns = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(if show_sidebar { 34 } else { 0 }),
    ])
    .split(rows[1]);
    if show_sidebar {
        draw_conversation(frame, view, columns[0]);
        draw_sidebar(frame, view, columns[1]);
    } else if rows[1].height >= 12 && !view.parts.is_empty() {
        let compact =
            Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).split(columns[0]);
        draw_conversation(frame, view, compact[0]);
        draw_peer_strip(frame, view, compact[1]);
    } else {
        draw_conversation(frame, view, columns[0]);
    }
    draw_composer(frame, view, rows[2]);
    draw_footer(frame, view, rows[3]);
    if view.picker.is_some() {
        draw_picker(frame, view, area);
    }
}

fn draw_tiny(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let (lines, x, y) = editor_layout(&view.input, view.cursor, usize::from(area.width));
    let scroll = y.saturating_sub(usize::from(area.height.saturating_sub(1)));
    frame.render_widget(
        Paragraph::new(lines).scroll((scroll.min(u16::MAX as usize) as u16, 0)),
        area,
    );
    frame.set_cursor_position((
        area.x + (x as u16).min(area.width - 1),
        area.y + ((y - scroll) as u16).min(area.height - 1),
    ));
}

fn draw_header(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(
        "KURU"
            .chars()
            .enumerate()
            .map(|(index, character)| Span::styled(character.to_string(), bold(PALETTE[index]))),
    );
    spans.push(Span::styled("  /  ", style(EDGE)));
    spans.push(Span::styled(view.mode.to_uppercase(), bold(LILAC)));
    if area.width >= 48 {
        spans.push(Span::styled("   ", style(MUTED)));
        spans.push(Span::styled(
            clipped(&view.model, usize::from(area.width.saturating_sub(36))),
            style(TEXT),
        ));
        spans.push(Span::styled(format!("  {}", view.effort), style(AMBER)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if area.height >= 3 {
        let hint = if view.transcript.is_empty() {
            "A native pool of persistent peers"
        } else {
            "A shared conversation · independent memories"
        };
        frame.render_widget(
            Paragraph::new(format!(" {hint}")).style(style(MUTED)),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
        let shift = if view.busy { phase(view) / 2 } else { 0 };
        let line = Line::from(
            (0..area.width)
                .map(|x| {
                    let index = (usize::from(x) * PALETTE.len() / usize::from(area.width) + shift)
                        % PALETTE.len();
                    Span::styled("━", style(PALETTE[index]))
                })
                .collect::<Vec<_>>(),
        );
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(area.x, area.y + 2, area.width, 1),
        );
    }
}

fn draw_conversation(frame: &mut Frame<'_>, view: &View, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let title = Line::from(vec![
        Span::styled(" Conversation ", bold(TEXT)),
        Span::styled(
            if view.busy {
                format!(" {} working ", spinner(view))
            } else {
                " ● ready ".into()
            },
            style(if view.busy { AMBER } else { MINT }),
        ),
    ]);
    let block = panel(title, TEXT);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if view.transcript.is_empty() {
        draw_welcome(frame, view, inner);
        return;
    }
    let content = inset(inner, u16::from(inner.width >= 12), 0);
    TRANSCRIPT.with_borrow_mut(|cache| {
        if cache.width != content.width || cache.source != view.transcript {
            cache.lines = wrap_lines(conversation_lines(view), usize::from(content.width.max(1)));
            cache.source.clone_from(&view.transcript);
            cache.width = content.width;
        }
        let offset = cache
            .lines
            .len()
            .saturating_sub(usize::from(content.height))
            .saturating_sub(usize::from(view.scroll));
        let visible = cache
            .lines
            .iter()
            .skip(offset)
            .take(usize::from(content.height))
            .cloned()
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(visible), content);
    });
    if view.scroll > 0 && area.width > 28 {
        frame.render_widget(
            Paragraph::new(" ↑ history · PgDn to return ").style(style(AMBER).bg(SURFACE)),
            Rect::new(area.right() - 28, area.y, 26, 1),
        );
    }
}

fn conversation_lines(view: &View) -> Vec<Line<'static>> {
    let mut lines = vec![Line::default()];
    for (speaker, body) in &view.transcript {
        let color = match speaker.as_str() {
            "user" => BLUE,
            "system" => AMBER,
            _ => identity_color(speaker),
        };
        let label = if speaker == "user" { "you" } else { speaker };
        lines.push(Line::from(vec![
            Span::styled("● ", style(color)),
            Span::styled(label.to_owned(), bold(color)),
        ]));
        let mut code = false;
        for line in body.lines() {
            if let Some(language) = line.trim_start().strip_prefix("```") {
                code = !code;
                lines.push(Line::from(Span::styled(
                    if code {
                        format!(
                            "  ┌─ {}",
                            if language.is_empty() {
                                "code"
                            } else {
                                language
                            }
                        )
                    } else {
                        "  └─".into()
                    },
                    style(MUTED),
                )));
            } else if code {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", style(EDGE)),
                    Span::styled(line.to_owned(), style(AMBER).bg(RAISED)),
                ]));
            } else if let Some(quote) = line.strip_prefix("> ") {
                let mut spans = vec![Span::styled("  ▎ ", style(LILAC))];
                spans.extend(inline_spans(quote, MUTED));
                lines.push(Line::from(spans));
            } else if line.starts_with('#') && line.trim_start_matches('#').starts_with(' ') {
                lines.push(Line::from(Span::styled(
                    format!("  {}", line.trim_start_matches('#').trim_start()),
                    bold(LILAC),
                )));
            } else {
                let mut spans = vec![Span::raw("  ")];
                if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
                    spans.push(Span::styled("• ", style(MINT)));
                    spans.extend(inline_spans(item, TEXT));
                } else {
                    spans.extend(inline_spans(line, TEXT));
                }
                lines.push(Line::from(spans));
            }
        }
        lines.push(Line::default());
    }
    lines
}

fn inline_spans(text: &str, color: Color) -> Vec<Span<'static>> {
    let mut spans = vec![];
    let mut remaining = text;
    while !remaining.is_empty() {
        let marker = remaining
            .find('`')
            .map(|index| (index, "`", style(AMBER).bg(RAISED)))
            .into_iter()
            .chain(remaining.find("**").map(|index| (index, "**", bold(color))))
            .min_by_key(|(index, _, _)| *index);
        let Some((start, delimiter, selected)) = marker else {
            spans.push(Span::styled(remaining.to_owned(), style(color)));
            break;
        };
        let after = &remaining[start + delimiter.len()..];
        let Some(end) = after.find(delimiter) else {
            spans.push(Span::styled(remaining.to_owned(), style(color)));
            break;
        };
        if start > 0 {
            spans.push(Span::styled(remaining[..start].to_owned(), style(color)));
        }
        spans.push(Span::styled(after[..end].to_owned(), selected));
        remaining = &after[end + delimiter.len()..];
    }
    spans
}

/// Wrap once using terminal cells, retaining span styles and every source character.
fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut wrapped = vec![];
    for line in lines {
        let cells = line
            .spans
            .iter()
            .flat_map(|span| {
                span.content
                    .chars()
                    .map(|character| (character, line.style.patch(span.style)))
            })
            .collect::<Vec<_>>();
        if cells.is_empty() {
            wrapped.push(Line::default());
            continue;
        }
        let mut start = 0;
        while start < cells.len() {
            let mut end = start;
            let mut used = 0;
            let mut boundary = None;
            while end < cells.len() {
                let character = cells[end].0;
                let count = character.width().unwrap_or(0);
                if used + count > width {
                    break;
                }
                used += count;
                end += 1;
                if character.is_whitespace() {
                    boundary = Some(end);
                }
            }
            if end < cells.len()
                && let Some(word_end) = boundary.filter(|&index| index > start + 2)
            {
                end = word_end;
            }
            end = end.max(start + 1);
            let mut spans: Vec<Span<'static>> = vec![];
            for &(character, selected) in &cells[start..end] {
                if let Some(span) = spans.last_mut().filter(|span| span.style == selected) {
                    span.content.to_mut().push(character);
                } else {
                    spans.push(Span::styled(character.to_string(), selected));
                }
            }
            wrapped.push(Line::from(spans));
            start = end;
        }
    }
    wrapped
}

fn draw_welcome(frame: &mut Frame<'_>, view: &View, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    // Keep the introduction together even in a very tall terminal pane.
    let area = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(32) / 2,
        area.width,
        area.height.min(32),
    );
    let large = area.width >= 43 && area.height >= 13;
    let wordmark = [
        "██  ██  ██    ██  ██████   ██    ██",
        "██ ██   ██    ██  ██   ██  ██    ██",
        "████    ██    ██  ██████   ██    ██",
        "██ ██   ██    ██  ██   ██  ██    ██",
        "██  ██   ██████   ██   ██   ██████ ",
    ];
    let mut lines = vec![Line::default()];
    if large {
        let shift = phase(view) / 4;
        for line in wordmark {
            lines.push(
                Line::from(
                    line.chars()
                        .enumerate()
                        .map(|(index, character)| {
                            Span::styled(
                                character.to_string(),
                                bold(PALETTE[(index / 7 + shift) % PALETTE.len()]),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
                .alignment(Alignment::Center),
            );
        }
        lines.push(Line::default());
    } else {
        lines.push(
            Line::from(Span::styled("Many voices. One conversation.", bold(MINT)))
                .alignment(Alignment::Center),
        );
    }
    lines.push(
        Line::from(Span::styled(
            if large {
                "Many voices. One conversation."
            } else {
                "Your peers are ready."
            },
            style(TEXT),
        ))
        .alignment(Alignment::Center),
    );
    lines.push(
        Line::from(Span::styled(
            format!("{} · {} persistent peers", view.mode, view.parts.len()),
            style(LILAC),
        ))
        .alignment(Alignment::Center),
    );
    lines.push(Line::default());
    lines.push(
        Line::from(Span::styled(
            "Type a task to begin. /help opens the guide.",
            style(MUTED),
        ))
        .alignment(Alignment::Center),
    );
    let text_height = (lines.len() as u16).min(area.height);
    frame.render_widget(
        Paragraph::new(lines),
        Rect::new(area.x, area.y, area.width, text_height),
    );
    let graph_width = area.width.saturating_sub(2).min(72);
    let graph = Rect::new(
        area.x + area.width.saturating_sub(graph_width) / 2,
        area.y + text_height,
        graph_width,
        area.height.saturating_sub(text_height + 1).min(16),
    );
    if graph.height >= 9 && graph.width >= 40 && !view.parts.is_empty() {
        draw_constellation(frame, view, graph);
    } else if area.height >= text_height + 2 {
        frame.render_widget(
            Paragraph::new(
                Line::from(vec![
                    Span::styled("F2", bold(BLUE)),
                    Span::styled(" model    ", style(MUTED)),
                    Span::styled("F3", bold(AMBER)),
                    Span::styled(" effort    ", style(MUTED)),
                    Span::styled("F4", bold(LILAC)),
                    Span::styled(" framework", style(MUTED)),
                ])
                .alignment(Alignment::Center),
            ),
            Rect::new(area.x, area.y + text_height + 1, area.width, 1),
        );
    }
}

fn draw_constellation(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let count = view.parts.len().min(10);
    let center_x = f64::from(area.width) / 2.0;
    let center_y = f64::from(area.height - 2) / 2.0;
    let compact = area.width < 40;
    let radius_x = f64::from(area.width.saturating_sub(if compact { 8 } else { 24 })) / 2.0;
    let radius_y = f64::from(area.height.saturating_sub(5)) / 2.0;
    let positions = (0..count)
        .map(|index| {
            let angle =
                std::f64::consts::TAU * index as f64 / count as f64 - std::f64::consts::FRAC_PI_2;
            (
                area.x + (center_x + radius_x * angle.cos()).round() as u16,
                area.y + (center_y + radius_y * angle.sin()).round() as u16,
            )
        })
        .collect::<Vec<_>>();
    for relationship in &view.relationships {
        for (index, member) in relationship.members.iter().enumerate() {
            for other in &relationship.members[index + 1..] {
                if let (Some(a), Some(b)) = (
                    view.parts
                        .iter()
                        .take(count)
                        .position(|(id, _)| id == member),
                    view.parts
                        .iter()
                        .take(count)
                        .position(|(id, _)| id == other),
                ) {
                    draw_edge(
                        frame,
                        positions[a],
                        positions[b],
                        relationship_color(relationship.kind),
                    );
                }
            }
        }
    }
    for (sender, recipient) in &view.routes {
        if let (Some(a), Some(b)) = (
            view.parts
                .iter()
                .take(count)
                .position(|(id, _)| id == sender),
            view.parts
                .iter()
                .take(count)
                .position(|(id, _)| id == recipient),
        ) {
            draw_edge(frame, positions[a], positions[b], MINT);
            if view.busy && view.motion {
                let (start, end) = (positions[a], positions[b]);
                let dx = i32::from(end.0) - i32::from(start.0);
                let dy = i32::from(end.1) - i32::from(start.1);
                let steps = dx.abs().max(dy.abs());
                if steps > 1 {
                    let step = (phase(view) % (steps as usize - 1) + 1) as i32;
                    let x = i32::from(start.0) + dx * step / steps;
                    let y = i32::from(start.1) + dy * step / steps;
                    frame.render_widget(
                        Paragraph::new("•").style(bold(MINT)),
                        Rect::new(x as u16, y as u16, 1, 1),
                    );
                }
            }
        }
    }
    for (index, ((id, label), (x, y))) in view.parts.iter().zip(positions).enumerate() {
        let color = identity_color(label);
        let active = view
            .part_activity
            .get(id)
            .is_some_and(|state| matches!(state.as_str(), "active" | "tool"));
        let glyph = if active && view.busy {
            spinner(view)
        } else if id == &view.speaker_id {
            "◆"
        } else {
            "●"
        };
        if compact {
            frame.render_widget(
                Paragraph::new(format!("{glyph}{}", index + 1)).style(bold(color).bg(SURFACE)),
                Rect::new(x, y, 3, 1),
            );
        } else {
            frame.render_widget(
                Paragraph::new(glyph).style(bold(color).bg(SURFACE)),
                Rect::new(x, y, 1, 1),
            );
            let name = clipped(&short_name(view, id), 13);
            let width = name.width() as u16;
            let left = x
                .saturating_sub(width / 2)
                .max(area.x)
                .min(area.right().saturating_sub(width));
            frame.render_widget(
                Paragraph::new(name).style(style(color).bg(SURFACE)),
                Rect::new(left, y + 1, width, 1),
            );
        }
    }
    frame.render_widget(
        Paragraph::new(if compact {
            "numbered peers · mint routes"
        } else {
            "independent peers · shared presence"
        })
        .alignment(Alignment::Center)
        .style(style(MUTED)),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    );
}

fn draw_edge(frame: &mut Frame<'_>, start: (u16, u16), end: (u16, u16), color: Color) {
    let dx = i32::from(end.0) - i32::from(start.0);
    let dy = i32::from(end.1) - i32::from(start.1);
    let steps = dx.abs().max(dy.abs()).max(1);
    for step in 1..steps {
        let x = i32::from(start.0) + dx * step / steps;
        let y = i32::from(start.1) + dy * step / steps;
        frame.render_widget(
            Paragraph::new("·").style(style(color)),
            Rect::new(x as u16, y as u16, 1, 1),
        );
    }
}

fn draw_peer_strip(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let mut spans = vec![
        Span::styled(" ● ", style(MINT)),
        Span::styled(format!("{} peers  ", view.parts.len()), style(MUTED)),
    ];
    for (index, (id, label)) in view.parts.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", style(EDGE)));
        }
        spans.push(Span::styled(
            short_name(view, id),
            if id == &view.speaker_id {
                bold(identity_color(label))
            } else {
                style(identity_color(label))
            },
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new(" /parts explores the pool · /relate connects peers").style(style(MUTED)),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
}

fn draw_sidebar(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let block = panel(" Active parts ", MINT);
    let mut inner = inset(block.inner(area), 1, 0);
    frame.render_widget(block, area);
    let numbered = inner.height >= 30 && !view.transcript.is_empty() && !view.parts.is_empty();
    if numbered {
        draw_constellation(frame, view, Rect::new(inner.x, inner.y, inner.width, 10));
        inner = Rect::new(inner.x, inner.y + 11, inner.width, inner.height - 11);
    }
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{} peers · equal standing", view.parts.len()),
            style(MUTED),
        )),
        Line::default(),
    ];
    let reserve = if !view.relationships.is_empty() { 5 } else { 3 };
    let detailed = usize::from(inner.height) >= view.parts.len() * 2 + usize::from(reserve) + 2;
    let limit =
        (usize::from(inner.height.saturating_sub(reserve)) / if detailed { 2 } else { 1 }).max(1);
    for (index, (id, label)) in view.parts.iter().take(limit).enumerate() {
        let phase = view.part_activity.get(id).map_or("idle", String::as_str);
        let (glyph, state, state_color) = if id == &view.speaker_id && view.busy {
            ("◆", "speaking", MINT)
        } else {
            match phase {
                "active" if view.busy => (spinner(view), "thinking", AMBER),
                "tool" if view.busy => (spinner(view), "tool", BLUE),
                "error" => ("!", "error", ROSE),
                _ => ("○", "ready", MUTED),
            }
        };
        let name = short_name(view, id);
        let prefix = if numbered {
            format!("{} {glyph} ", index + 1)
        } else {
            format!("{glyph} ")
        };
        let label_width = usize::from(
            inner
                .width
                .saturating_sub(state.width() as u16 + prefix.width() as u16 + 2),
        );
        let name = clipped(&name, label_width);
        let spacing = label_width.saturating_sub(name.width()) + 1;
        lines.push(Line::from(vec![
            Span::styled(prefix, style(state_color)),
            Span::styled(name, bold(identity_color(label))),
            Span::raw(" ".repeat(spacing)),
            Span::styled(state, style(state_color)),
        ]));
        if detailed {
            lines.push(Line::from(Span::styled(
                format!("  {}", label.rsplit(" · ").next().unwrap_or(label)),
                style(MUTED),
            )));
        }
    }
    if view.parts.len() > limit {
        lines.push(Line::from(Span::styled(
            format!("+{} more · /parts", view.parts.len() - limit),
            style(MUTED),
        )));
    }
    if !view.relationships.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("Relationships", bold(LILAC))));
        for relationship in &view.relationships {
            if lines.len() + 2 > usize::from(inner.height) {
                break;
            }
            let speaking = view.busy && view.speaker_id == relationship.id;
            let focused = view.focus.as_deref() == Some(relationship.id.as_str());
            lines.push(Line::from(Span::styled(
                format!(
                    "{} {}{}",
                    if speaking { "◆" } else { "◇" },
                    relationship.kind,
                    if speaking {
                        " · speaking"
                    } else if view.speaker_id == relationship.id {
                        " · last speaker"
                    } else if focused {
                        " · focused"
                    } else {
                        ""
                    }
                ),
                style(relationship_color(relationship.kind)),
            )));
            lines.push(Line::from(Span::styled(
                relationship
                    .members
                    .iter()
                    .map(|id| short_name(view, id))
                    .collect::<Vec<_>>()
                    .join(" + "),
                style(TEXT),
            )));
        }
    }
    if !view.routes.is_empty() && lines.len() + 3 <= usize::from(inner.height) {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("Peer exchange", bold(BLUE))));
        for (from, to) in view.routes.iter().rev().take(2) {
            lines.push(Line::from(Span::styled(
                format!("{} → {}", short_name(view, from), short_name(view, to)),
                style(MUTED),
            )));
        }
    }
    if lines.len() + 3 <= usize::from(inner.height) {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("Activity", bold(AMBER))));
        let available = usize::from(inner.height).saturating_sub(lines.len());
        if view.activity.is_empty() {
            lines.push(Line::from(Span::styled(
                "Waiting for your next idea.",
                style(MUTED),
            )));
        } else {
            lines.extend(view.activity.iter().rev().take(available).map(|text| {
                Line::from(Span::styled(
                    clipped(text, usize::from(inner.width)),
                    style(MUTED),
                ))
            }));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_composer(frame: &mut Frame<'_>, view: &View, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let color = if view.busy { LILAC } else { MINT };
    let title = if view.busy {
        " Compose next message "
    } else {
        " Message "
    };
    let block = panel(title, color)
        .border_style(style(color))
        .style(style(TEXT).bg(RAISED));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let prompt_width = u16::from(inner.width >= 4) * 2;
    if prompt_width > 0 {
        frame.render_widget(
            Paragraph::new("›").style(bold(color)),
            Rect::new(inner.x, inner.y, 1, 1),
        );
    }
    let input = Rect::new(
        inner.x + prompt_width,
        inner.y,
        inner.width.saturating_sub(prompt_width),
        inner.height,
    );
    if input.width == 0 || input.height == 0 {
        return;
    }
    let (lines, x, y) = editor_layout(&view.input, view.cursor, usize::from(input.width));
    let scroll = y.saturating_sub(usize::from(input.height.saturating_sub(1)));
    if view.input.is_empty() {
        frame.render_widget(
            Paragraph::new(if view.busy {
                "Keep a thought here while the pool works…"
            } else {
                "What shall we explore or build?"
            })
            .style(style(MUTED)),
            input,
        );
    } else {
        frame.render_widget(
            Paragraph::new(lines).scroll((scroll.min(u16::MAX as usize) as u16, 0)),
            input,
        );
    }
    if area.width >= 38 {
        let hint = if view.busy {
            " Esc cancel · Alt+Enter newline "
        } else {
            " Enter send · Alt+Enter newline "
        };
        frame.render_widget(
            Paragraph::new(hint).style(style(MUTED).bg(RAISED)),
            Rect::new(
                area.right().saturating_sub(hint.width() as u16 + 2),
                area.bottom() - 1,
                hint.width() as u16,
                1,
            ),
        );
    }
    if view.picker.is_none() {
        frame.set_cursor_position((
            input.x + (x as u16).min(input.width - 1),
            input.y + ((y - scroll) as u16).min(input.height - 1),
        ));
    }
}

fn draw_footer(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let detailed = area.width >= 90;
    let mut controls = vec![];
    if area.width >= 50 {
        controls.extend([
            Span::styled(" F2", bold(BLUE)),
            Span::styled(if detailed { " model " } else { " " }, style(MUTED)),
            Span::styled("F3", bold(AMBER)),
            Span::styled(if detailed { " effort " } else { " " }, style(MUTED)),
            Span::styled("F4", bold(LILAC)),
            Span::styled(if detailed { " mode " } else { " " }, style(MUTED)),
        ]);
    }
    controls.push(Span::styled(" F6", bold(MINT)));
    controls.push(Span::styled(
        if view.motion {
            " Motion: on "
        } else {
            " Motion: reduced "
        },
        style(MUTED),
    ));
    let controls = Line::from(controls);
    let control_width = (controls.width() as u16).min(area.width);
    let status_width = area.width.saturating_sub(control_width);
    let status = view.status.split(" · ").next().unwrap_or(&view.status);
    let color = if status.to_lowercase().contains("error") {
        ROSE
    } else if view.busy {
        AMBER
    } else {
        MINT
    };
    let mut status_spans = vec![Span::styled(
        if view.busy {
            format!(" {} ", spinner(view))
        } else {
            " ● ".into()
        },
        style(color),
    )];
    let label = if status_width >= 25 && view.speaker != "pool" {
        format!("{status} · {}", view.speaker)
    } else {
        status.to_owned()
    };
    status_spans.push(Span::styled(
        clipped(&label, usize::from(status_width.saturating_sub(3))),
        style(MUTED),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(status_spans)),
        Rect::new(area.x, area.y, status_width, 1),
    );
    frame.render_widget(
        Paragraph::new(controls),
        Rect::new(area.x + status_width, area.y, control_width, 1),
    );
}

fn draw_picker(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let options = view.options();
    let width = area.width.saturating_sub(4).clamp(1, 72);
    let height = (options.len() as u16 + 5)
        .min(area.height.saturating_sub(2))
        .max(1);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let (title, color, description) = match view.picker {
        Some(Picker::Models) => ("Models", BLUE, "Choose the voice behind your peers"),
        Some(Picker::Efforts) => ("Efforts", AMBER, "Choose a supported reasoning effort"),
        _ => ("Modes", LILAC, "Choose the shape of the peer pool"),
    };
    let block = panel(format!(" {title} · Enter selects "), color)
        .border_style(style(color))
        .style(style(TEXT).bg(RAISED));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height == 0 {
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(u16::from(inner.height >= 4) * 2),
        Constraint::Min(1),
        Constraint::Length(u16::from(inner.height >= 3)),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(format!(" {description}")).style(style(MUTED)),
        rows[0],
    );
    let items = options
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let current = match view.picker {
                Some(Picker::Models) => label == &view.model,
                Some(Picker::Efforts) => label == &view.effort,
                _ => label == &view.mode,
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{} {label}", if current { "●" } else { "○" })),
                Span::raw(if index == view.selected { "  ←" } else { "" }),
            ]))
        })
        .collect::<Vec<_>>();
    let mut selected = ListState::default().with_selected(Some(view.selected));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol(" › ")
            .highlight_style(bold(INK).bg(color)),
        rows[1],
        &mut selected,
    );
    frame.render_widget(
        Paragraph::new(" ↑↓ move · Enter selects · Esc back").style(style(MUTED)),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
            .collect()
    }

    #[test]
    fn wrapping_preserves_unicode_spaces_and_styles_at_word_boundaries() {
        let original = vec![Line::from(vec![
            Span::styled("Build ", bold(MINT)),
            Span::styled("猫 tools safely", style(AMBER)),
        ])];
        let wrapped = wrap_lines(original, 9);
        assert_eq!(text(&wrapped), "Build 猫 tools safely");
        assert_eq!(text(&wrapped[..1]), "Build 猫 ");
        assert!(wrapped.iter().all(|line| line.width() <= 9));
        assert_eq!(wrapped[0].spans[0].style, bold(MINT));
        assert_eq!(wrapped[0].spans[1].style, style(AMBER));
        assert!(
            wrapped
                .last()
                .unwrap()
                .spans
                .iter()
                .all(|span| span.style == style(AMBER))
        );
    }

    #[test]
    fn inline_formatting_distinguishes_code_bold_and_literal_unmatched_markers() {
        let spans = inline_spans(
            "Use `cargo check` for **validation**, keep `unfinished",
            TEXT,
        );
        let line = Line::from(spans);
        assert_eq!(
            text(std::slice::from_ref(&line)),
            "Use cargo check for validation, keep `unfinished"
        );
        let code = line
            .spans
            .iter()
            .find(|span| span.content == "cargo check")
            .unwrap();
        assert_eq!(code.style.fg, Some(AMBER));
        assert_eq!(code.style.bg, Some(RAISED));
        let emphasis = line
            .spans
            .iter()
            .find(|span| span.content == "validation")
            .unwrap();
        assert!(emphasis.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn long_tokens_combining_marks_and_empty_lines_survive_wrapping() {
        let lines = wrap_lines(
            vec![
                Line::from("a\u{301}bcdef"),
                Line::default(),
                Line::from("猫"),
            ],
            2,
        );
        assert_eq!(text(&lines), "a\u{301}bcdef猫");
        assert_eq!(text(&lines[..1]), "a\u{301}b");
        assert!(lines.iter().all(|line| line.width() <= 2));
        assert!(lines[3].spans.is_empty());
        assert_eq!(clipped("猫猫", 3), "猫…");
        assert_eq!(clipped("word", 0), "");
    }
}
