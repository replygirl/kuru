//! Pure terminal presentation. Runtime state supplies every activity label and edge.

use std::{cell::RefCell, collections::BTreeMap};

use kuru_core::{FactProvenance, RelationshipKind, UsagePhase};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{Picker, View, editor_layout, scene};

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
    completion_metadata: BTreeMap<usize, String>,
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
    if area.height < 7 || area.width < 14 {
        draw_tiny(frame, view, area);
        return;
    }
    // Chip rows, then the standing cost/context meters, then permissions.
    let control_rows = 2 + if area.width >= 72 {
        1
    } else if area.width >= 38 {
        2
    } else {
        3
    };
    let input_rows = editor_layout(
        &view.input,
        view.cursor,
        usize::from(area.width.saturating_sub(6)).max(1),
    )
    .0
    .len()
    .clamp(2, 5) as u16;
    let dock_height = (input_rows + control_rows + 2).min(area.height.saturating_sub(3));
    let preview_height = if view.preview.is_some() {
        6.min(area.height.saturating_sub(dock_height + 5))
    } else {
        0
    };
    let rows = Layout::vertical([
        Constraint::Length(if area.height >= 16 { 2 } else { 1 }),
        Constraint::Min(0),
        Constraint::Length(preview_height),
        Constraint::Length(1),
        Constraint::Length(dock_height),
        Constraint::Length(1),
    ])
    .split(area);
    draw_header(frame, view, rows[0]);
    let welcome = view.show_scene && view.transcript.is_empty();
    let sidebar = !welcome && area.width >= 108 && rows[1].height >= 17;
    if welcome {
        draw_welcome(frame, view, rows[1]);
    } else if sidebar {
        let columns =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(34)]).split(rows[1]);
        draw_conversation(frame, view, columns[0]);
        draw_sidebar(frame, view, columns[1]);
    } else {
        draw_conversation(frame, view, rows[1]);
    }
    draw_preview(frame, view, rows[2]);
    draw_status(frame, view, rows[3]);
    draw_composer(frame, view, rows[4]);
    draw_footer(frame, view, rows[5]);
    if view.picker.is_some() {
        draw_picker(frame, view, area);
    }
    let overlay = Rect::new(
        area.x.saturating_add(1),
        rows[1].y,
        area.width.saturating_sub(2),
        rows[3].bottom().saturating_sub(rows[1].y),
    );
    if view.instruction_prompt.is_some() {
        draw_instruction_prompt(frame, view, overlay);
    } else if view.permission_prompt.is_some() {
        draw_permission_prompt(frame, view, overlay);
    } else if view.permission_rows.is_some() {
        draw_permission_inspector(frame, view, overlay);
    }
}

fn draw_preview(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let Some(preview) = &view.preview else { return };
    if area.width < 4 || area.height < 3 {
        return;
    }
    let inner = inset(area, 2.min(area.width / 4), 1);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut info = Vec::new();
    if !preview.summary_tail.is_empty() {
        let prefix = if preview.summary_truncated {
            "thinking … "
        } else {
            "thinking · "
        };
        info.push(Line::from(Span::styled(
            clipped(
                &format!("{prefix}{}", safe_preview_line(&preview.summary_tail)),
                inner.width as usize,
            ),
            style(LILAC),
        )));
    }
    // A dispatched tool call carries its raw catalog name; while its arguments
    // are still streaming the runtime can only say that a call is in flight.
    // The most recently started facing call still running wins; the view
    // clears its own record (and corrects the cached preview) as each one
    // settles, so an empty record always falls back to a truthful preview.
    let activity = view
        .calling_tool
        .last()
        .map(|(_, _, name)| {
            if view.calling_tool.len() == 1 {
                format!("Calling {name}")
            } else {
                format!("Calling {name} · {} active", view.calling_tool.len())
            }
        })
        .unwrap_or_else(|| preview.activity.clone());
    if !activity.is_empty() {
        let prefix = if view.calling_tool.is_empty() && preview.activity_truncated {
            "activity … "
        } else {
            "activity · "
        };
        info.push(Line::from(Span::styled(
            clipped(
                &format!("{prefix}{}", safe_preview_line(&activity)),
                inner.width as usize,
            ),
            style(MUTED),
        )));
    }
    let info_height = (info.len() as u16).min(inner.height.saturating_sub(1));
    let text_height = inner.height.saturating_sub(info_height);
    let text_area = Rect::new(inner.x, inner.y, inner.width, text_height);
    let text = if preview.text_tail.is_empty() {
        vec![Line::from(Span::styled(
            "Waiting for the selected voice…",
            style(MUTED),
        ))]
    } else {
        preview
            .text_tail
            .lines()
            .map(|line| Line::from(Span::styled(safe_preview(line), style(TEXT))))
            .collect()
    };
    let wrapped = wrap_lines(text, inner.width as usize);
    let title = if preview.text_truncated || wrapped.len() > text_height as usize {
        " draft · earlier text omitted "
    } else {
        " draft · provisional "
    };
    frame.render_widget(panel(title, MINT), area);
    let visible = wrapped
        .into_iter()
        .rev()
        .take(text_height as usize)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(visible.into_iter().rev().collect::<Vec<_>>()),
        text_area,
    );
    if info_height > 0 {
        frame.render_widget(
            Paragraph::new(
                info.into_iter()
                    .take(info_height as usize)
                    .collect::<Vec<_>>(),
            ),
            Rect::new(inner.x, text_area.bottom(), inner.width, info_height),
        );
    }
}

fn safe_preview(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .collect()
}

fn safe_preview_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
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
    let mut spans = vec![Span::raw("  ")];
    spans.extend(
        "KURU"
            .chars()
            .enumerate()
            .map(|(i, c)| Span::styled(c.to_string(), bold(PALETTE[i]))),
    );
    spans.push(Span::styled("  /  ", style(EDGE)));
    spans.push(Span::styled(
        clipped(&view.project, usize::from(area.width.saturating_sub(38))),
        style(MUTED),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if area.width >= 62 {
        let label = format!(
            "{}  ·  {} turns  ",
            view.session.chars().take(8).collect::<String>(),
            view.turns
        );
        let width = label.width() as u16;
        frame.render_widget(
            Paragraph::new(label).style(style(MUTED)),
            Rect::new(area.right().saturating_sub(width), area.y, width, 1),
        );
    }
}

fn draw_conversation(frame: &mut Frame<'_>, view: &View, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let content = inset(area, u16::from(area.width >= 12) * 2, 1);
    TRANSCRIPT.with_borrow_mut(|cache| {
        if cache.width != content.width
            || cache.source != view.transcript
            || cache.completion_metadata != view.completion_metadata
        {
            cache.lines = wrap_lines(conversation_lines(view), usize::from(content.width.max(1)));
            cache.source.clone_from(&view.transcript);
            cache
                .completion_metadata
                .clone_from(&view.completion_metadata);
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
    for (index, (speaker, body)) in view.transcript.iter().enumerate() {
        let color = match speaker.as_str() {
            "user" => BLUE,
            "system" | "help" => AMBER,
            "error" => ROSE,
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
        if let Some(metadata) = view.completion_metadata.get(&index) {
            lines.push(Line::from(Span::styled(
                format!("  {metadata}"),
                style(MUTED),
            )));
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
    if area.width == 0 || area.height == 0 {
        return;
    }
    let identity = scene::identity(&view.mode);
    let tall = area.height >= 23 && area.width >= 46;
    let total = area.height.min(if tall { 31 } else { 17 });
    let body = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(total) / 2,
        area.width,
        total,
    );
    let wordmark = [
        "██  ██  ██    ██  ██████   ██    ██",
        "██ ██   ██    ██  ██   ██  ██    ██",
        "████    ██    ██  ██████   ██    ██",
        "██ ██   ██    ██  ██   ██  ██    ██",
        "██  ██   ██████   ██   ██   ██████ ",
    ];
    let mut lines = vec![];
    if tall {
        for line in wordmark {
            lines.push(
                Line::from(
                    line.chars()
                        .enumerate()
                        .map(|(index, c)| {
                            let color = if index < 7 {
                                MINT
                            } else if index < 17 {
                                BLUE
                            } else if index < 26 {
                                LILAC
                            } else {
                                AMBER
                            };
                            Span::styled(c.to_string(), bold(color))
                        })
                        .collect::<Vec<_>>(),
                )
                .alignment(Alignment::Center),
            );
        }
        lines.push(Line::default());
    }
    lines.push(
        Line::from(Span::styled("Many voices. One conversation.", style(TEXT)))
            .alignment(Alignment::Center),
    );
    lines.push(Line::default());
    lines.push(
        Line::from(vec![
            Span::styled(
                format!("{}  {}", identity.symbol, identity.title),
                bold(identity.accent),
            ),
            Span::styled(format!("  /  {} parts", view.parts.len()), style(MUTED)),
        ])
        .alignment(Alignment::Center),
    );
    if body.width >= 48 {
        lines.push(
            Line::from(Span::styled(identity.description, style(MUTED)))
                .alignment(Alignment::Center),
        );
    }
    let text_height = lines.len() as u16;
    frame.render_widget(
        Paragraph::new(lines),
        Rect::new(body.x, body.y, body.width, text_height.min(body.height)),
    );
    let width = body.width.saturating_sub(4).min(76);
    let graph = Rect::new(
        body.x + body.width.saturating_sub(width) / 2,
        body.y + text_height.min(body.height),
        width,
        body.height.saturating_sub(text_height),
    );
    scene::draw(frame, view, graph);
}

fn draw_sidebar(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(style(EDGE))
        .style(style(TEXT).bg(INK));
    let mut inner = inset(block.inner(area), 2, 0);
    frame.render_widget(block, area);
    let numbered = inner.height >= 26 && !view.parts.is_empty();
    if numbered {
        scene::draw(frame, view, Rect::new(inner.x, inner.y, inner.width, 10));
        inner = Rect::new(inner.x, inner.y + 11, inner.width, inner.height - 11);
    }
    let mut lines = vec![
        Line::from(Span::styled(
            format!("PARTS / {}", view.parts.len()),
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
            lines.push(Line::from(Span::styled("No activity yet.", style(MUTED))));
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

fn dock_controls(view: &View, width: u16) -> Vec<Line<'static>> {
    let identity = scene::identity(&view.mode);
    let chip = |symbol: &str, value: String, key: &str, color: Color| {
        vec![
            Span::styled(format!("{symbol} "), style(color)),
            Span::styled(value, bold(color)),
            Span::styled(format!(" {key}  "), style(MUTED)),
        ]
    };
    let effort_budget = if width >= 68 {
        (width / 4).min(18)
    } else if width >= 34 {
        width.saturating_sub(identity.title.width() as u16 + 14)
    } else {
        width.saturating_sub(7)
    };
    let effort_value = clipped(&view.effort, usize::from(effort_budget));
    let model_budget = if width >= 68 {
        width.saturating_sub(effort_value.width() as u16 + identity.title.width() as u16 + 21)
    } else {
        width.saturating_sub(7)
    };
    let model = chip(
        "◇",
        clipped(&view.model, usize::from(model_budget)),
        "F2",
        BLUE,
    );
    let effort = chip("~", effort_value, "F3", AMBER);
    let mode = chip(
        identity.symbol,
        clipped(identity.title, usize::from(width.saturating_sub(7))),
        "F4",
        identity.accent,
    );
    let mut rows = if width >= 68 {
        vec![Line::from([model, effort, mode].concat())]
    } else if width >= 34 {
        vec![Line::from(model), Line::from([effort, mode].concat())]
    } else {
        vec![Line::from(model), Line::from(effort), Line::from(mode)]
    };
    rows.push(Line::from(Span::styled(
        clipped(&dock_meters(view, width), usize::from(width)),
        style(AMBER),
    )));
    rows.push(Line::from(Span::styled(
        clipped(
            &format!(
                "◈ Permissions · {} session · {} always  F5",
                view.permission_counts.0, view.permission_counts.1
            ),
            usize::from(width),
        ),
        style(MINT),
    )));
    rows
}

fn window_provenance(provenance: &FactProvenance) -> (&'static str, &'static str) {
    match provenance {
        FactProvenance::RouteAdvertisement => ("advertised", "adv"),
        FactProvenance::Pinned { .. } => ("pinned", "pin"),
        FactProvenance::ConfiguredAssumption => ("configured assumption", "cfg"),
        FactProvenance::BuiltInAssumption => ("built-in assumption", "assumed"),
    }
}

fn compact_tokens(value: u64) -> String {
    if value < 10_000 {
        value.to_string()
    } else if value < 10_000_000 {
        format!("{}k", value.div_ceil(1_000))
    } else {
        format!("{}M", value.div_ceil(1_000_000))
    }
}

/// The standing context-use and cost meters. Both stay in the dock for every
/// frame; the one-line status slot above it keeps carrying transient work.
///
/// An unknown price renders as the word `unknown`, never as a zero charge and
/// never as a quota or subscription amount. A known subtotal that the ledger
/// could not finish pricing reads as a lower bound, and both forms stay
/// labelled as an estimate; `/cost` prints the same figure in full.
fn dock_meters(view: &View, width: u16) -> String {
    let wide = width >= 40;
    let context = view.request_context.as_ref().map_or_else(
        || "ctx not estimated yet".to_owned(),
        |context| {
            let budget = &context.estimate.budget;
            let input = context.estimate.estimated_input_tokens;
            let reserve = budget.output_reserve_tokens;
            if wide {
                let (_, provenance) = window_provenance(&budget.window.provenance);
                format!(
                    "ctx ≈{input}+{reserve}/{} {provenance} · {}",
                    budget.window.value,
                    context.estimate.sizing.label(),
                )
            } else {
                let assumed = matches!(
                    budget.window.provenance,
                    FactProvenance::ConfiguredAssumption | FactProvenance::BuiltInAssumption
                );
                format!(
                    "ctx {}/{}{}",
                    compact_tokens(input.saturating_add(reserve)),
                    compact_tokens(budget.window.value),
                    if assumed { "~" } else { "" }
                )
            }
        },
    );
    let cost = view
        .usage
        .as_ref()
        .and_then(|usage| {
            [&usage.api_standard, &usage.api_equivalent]
                .into_iter()
                .find_map(|estimate| {
                    let known = estimate.known_usd.as_ref()?;
                    Some(if estimate.incomplete {
                        format!("≥${known} est")
                    } else {
                        format!("≈${known} est")
                    })
                })
        })
        .unwrap_or_else(|| "cost unknown".to_owned());
    if wide {
        format!("◆ {context}  ·  {cost}")
    } else {
        format!("◆ {context} {cost}")
    }
}

fn draw_permission_prompt(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let Some(prompt) = &view.permission_prompt else {
        return;
    };
    if area.width < 8 || area.height < 5 {
        return;
    }
    frame.render_widget(Clear, area);
    let title = if prompt.display.rememberable {
        "Permission request · ↑↓ scope"
    } else {
        "Permission request · Once only"
    };
    let inner = panel(title, AMBER).inner(area);
    frame.render_widget(panel(title, AMBER), area);
    if inner.height < 3 {
        return;
    }
    let choices_height = if inner.width >= 62 { 1 } else { 2 };
    let reason_height = u16::from(!prompt.display.rememberable);
    let body_height = inner.height.saturating_sub(choices_height + reason_height);
    let body = Rect::new(inner.x, inner.y, inner.width, body_height);
    let scope_kind = if prompt.whole_tool {
        "whole tool"
    } else {
        "literal project file"
    };
    let content = format!(
        "{}\nExact grant scope ({scope_kind}): {}\nPreview: {}",
        prompt.display.label, prompt.display.scope, prompt.display.preview
    );
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .scroll((prompt.scroll, 0)),
        body,
    );
    if !prompt.display.rememberable {
        let reason = prompt
            .display
            .remember_disabled_reason
            .as_deref()
            .unwrap_or("Exact grant scope is unavailable.");
        frame.render_widget(
            Paragraph::new(clipped(reason, usize::from(inner.width))).style(style(ROSE)),
            Rect::new(inner.x, body.bottom(), inner.width, reason_height),
        );
    }
    let choices = if prompt.display.rememberable {
        if inner.width >= 62 {
            "1 once · 2 session · 3 always · 4 deny  (also Alt+digit)"
        } else {
            "1 once · 2 session  (Alt+digit)\n3 always · 4 deny"
        }
    } else if inner.width >= 62 {
        "1 once · 2/3 unavailable · 4 deny  (also Alt+digit)"
    } else {
        "1 once · 4 deny  (Alt+digit)\nSession/Always unavailable"
    };
    frame.render_widget(
        Paragraph::new(choices).style(bold(AMBER)),
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(choices_height),
            inner.width,
            choices_height,
        ),
    );
}

fn draw_instruction_prompt(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let Some(prompt) = &view.instruction_prompt else {
        return;
    };
    if area.width < 8 || area.height < 5 {
        return;
    }
    frame.render_widget(Clear, area);
    let title = "Workspace instruction review · ↑↓ claims";
    let inner = panel(title, AMBER).inner(area);
    frame.render_widget(panel(title, AMBER), area);
    if inner.height < 2 {
        return;
    }
    let choices_height = if inner.width >= 48 { 1 } else { 2 };
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(choices_height),
    );
    frame.render_widget(
        Paragraph::new(prompt.display.as_str())
            .wrap(Wrap { trim: false })
            .scroll((prompt.scroll, 0)),
        body,
    );
    let choices = if !prompt.persistent_allowed && choices_height == 1 {
        "1 once · 2 persistent unavailable · 3 deny"
    } else if !prompt.persistent_allowed {
        "1 continue once · 3 deny\nPersistent approval unavailable"
    } else if choices_height == 1 {
        "1 continue once · 2 approve complete manifest · 3 deny"
    } else {
        "1 continue once · 2 approve complete manifest\n3 deny"
    };
    frame.render_widget(
        Paragraph::new(choices).style(bold(AMBER)),
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(choices_height),
            inner.width,
            choices_height,
        ),
    );
}

fn draw_permission_inspector(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let Some(rows) = &view.permission_rows else {
        return;
    };
    if area.width < 8 || area.height < 5 {
        return;
    }
    frame.render_widget(Clear, area);
    let title = "Permissions · ↑↓ select · PgUp/PgDn detail · Delete revoke · Esc close";
    let inner = panel(title, MINT).inner(area);
    frame.render_widget(panel(title, MINT), area);
    if inner.height == 0 {
        return;
    }
    if rows.is_empty() {
        frame.render_widget(Paragraph::new("No session or always grants."), inner);
        return;
    }
    let detail_height = inner.height.saturating_sub(2).min(8);
    let list_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(detail_height),
    );
    let items = rows
        .iter()
        .map(|row| {
            let lifetime = if row.persistent { "Always" } else { "Session" };
            ListItem::new(clipped(
                &format!("{lifetime} · {}", row.label),
                usize::from(inner.width),
            ))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(view.permission_selected));
    frame.render_stateful_widget(
        List::new(items).highlight_style(bold(AMBER)),
        list_area,
        &mut state,
    );
    if let Some(row) = rows.get(view.permission_selected) {
        let detail = format!(
            "Selected exact scope · PgUp/PgDn to inspect all:\n{}",
            row.label
        );
        frame.render_widget(
            Paragraph::new(detail)
                .wrap(Wrap { trim: false })
                .scroll((view.permission_detail_scroll, 0))
                .style(style(TEXT)),
            Rect::new(inner.x, list_area.bottom(), inner.width, detail_height),
        );
    }
}

fn draw_status(frame: &mut Frame<'_>, view: &View, area: Rect) {
    if area.height == 0 {
        return;
    }
    let identity = scene::identity(&view.mode);
    let error = view.status.starts_with("Failed") || view.status.starts_with("error");
    let cancelled = view.status.starts_with("Cancelled");
    if view.notice.is_none()
        && !view.busy
        && view.show_scene
        && view.transcript.is_empty()
        && !error
        && !cancelled
    {
        return;
    }
    let context_label = view.request_context.as_ref().map(|context| {
        let phase = match context.phase {
            UsagePhase::Deliberate => "deliberate",
            UsagePhase::Speak => "speak",
            UsagePhase::Consult => "consult",
            UsagePhase::Dream => "dream",
            UsagePhase::Compact => "compact",
        };
        let (provenance, short_provenance) =
            window_provenance(&context.estimate.budget.window.provenance);
        let input = context.estimate.estimated_input_tokens;
        let reserve = context.estimate.budget.output_reserve_tokens;
        let window = context.estimate.budget.window.value;
        if area.width < 60 {
            format!("≈{input}+{reserve}/{window} {short_provenance}")
        } else {
            format!(
                "≈{input}+{reserve}/{window} tokens · {} {} · {provenance} · {}",
                view.actor_name(&context.actor_id),
                phase,
                context.estimate.sizing.label(),
            )
        }
    });
    let (glyph, label, color) = if let Some(notice) = &view.notice {
        (
            if view.busy { "i" } else { "✓" },
            notice.clone(),
            identity.accent,
        )
    } else if view.busy {
        if let Some(context) = context_label.as_ref()
            && view.active_operation_id.as_deref().is_some_and(|id| {
                view.request_context
                    .as_ref()
                    .is_some_and(|item| item.operation_id == id)
            })
        {
            (
                spinner(view),
                format!(
                    "{}{context}",
                    if area.width < 60 {
                        "est "
                    } else {
                        "prepared est "
                    }
                ),
                identity.accent,
            )
        } else {
            let thinking = view
                .part_activity
                .values()
                .filter(|s| matches!(s.as_str(), "active" | "tool"))
                .count();
            (
                spinner(view),
                format!(
                    "{}  ·  {}s{}",
                    view.status,
                    view.operation_ms / 1000,
                    if thinking > 0 {
                        format!("  ·  {thinking} active")
                    } else {
                        String::new()
                    }
                ),
                if error { ROSE } else { identity.accent },
            )
        }
    } else if cancelled || error {
        (
            if error { "!" } else { "-" },
            view.status.clone(),
            if error { ROSE } else { AMBER },
        )
    } else if let Some(context) = context_label.as_ref() {
        ("◆", format!("last {context}"), identity.accent)
    } else if view.speaker != "pool" && !view.show_scene {
        (
            "◆",
            format!(
                "{}  ·  {} parts present{}",
                view.speaker,
                view.parts.len(),
                view.usage
                    .as_ref()
                    .map_or_else(String::new, |usage| format!(
                        " · {} recorded calls · /cost",
                        usage.invocation_count
                    ))
            ),
            identity.accent,
        )
    } else {
        (
            identity.symbol,
            format!(
                "{} parts present{}",
                view.parts.len(),
                view.usage
                    .as_ref()
                    .map_or_else(String::new, |usage| format!(
                        " · {} recorded calls · /cost",
                        usage.invocation_count
                    ))
            ),
            MUTED,
        )
    };
    let text = Line::from(vec![
        Span::styled(format!("  {glyph} "), style(color)),
        Span::styled(
            clipped(&label, usize::from(area.width.saturating_sub(6))),
            style(if error { ROSE } else { MUTED }),
        ),
    ]);
    frame.render_widget(Paragraph::new(text), area);
}

fn draw_composer(frame: &mut Frame<'_>, view: &View, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let accent = scene::identity(&view.mode).accent;
    frame.render_widget(Block::default().style(style(TEXT).bg(SURFACE)), area);
    let inner = inset(area, u16::from(area.width >= 8) * 2, 0);
    let controls = dock_controls(view, inner.width);
    let control_height = (controls.len() as u16).min(inner.height.saturating_sub(1));
    let control_area = Rect::new(
        inner.x,
        inner.bottom().saturating_sub(control_height),
        inner.width,
        control_height,
    );
    frame.render_widget(Paragraph::new(controls), control_area);
    let input = Rect::new(
        inner.x.saturating_add(2).min(inner.right()),
        inner.y + u16::from(inner.height > 2),
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(control_height + 2).max(1),
    );
    let line = Line::from(
        (0..area.width)
            .map(|x| Span::styled(if x % 5 == 0 { "." } else { " " }, style(EDGE)))
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if input.width == 0 || input.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("›").style(bold(accent)),
        Rect::new(inner.x, input.y, 1, 1),
    );
    let (lines, x, y) = editor_layout(&view.input, view.cursor, usize::from(input.width));
    let scroll = y.saturating_sub(usize::from(input.height.saturating_sub(1)));
    if view.input.is_empty() {
        frame.render_widget(
            Paragraph::new(if view.busy {
                "Keep your next thought here…"
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
    if view.picker.is_none() {
        frame.set_cursor_position((
            input.x + (x as u16).min(input.width - 1),
            input.y + ((y - scroll) as u16).min(input.height - 1),
        ));
    }
}

fn draw_footer(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let hint = if view.busy {
        "  esc cancel  ·  alt+enter newline"
    } else {
        "  enter send  ·  alt+enter newline"
    };
    let mut spans = vec![Span::styled(hint, style(MUTED))];
    if area.width >= 65 {
        spans.push(Span::styled("  ·  /help", style(MUTED)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if area.width >= 90 {
        let label = if view.input.is_empty() {
            "PgUp/PgDn history  ".into()
        } else {
            format!("{} chars  ", view.input.chars().count())
        };
        let width = label.width() as u16;
        frame.render_widget(
            Paragraph::new(label).style(style(MUTED)),
            Rect::new(area.right().saturating_sub(width), area.y, width, 1),
        );
    }
}

fn draw_picker(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let options = view.options();
    let modes = view.picker == Some(Picker::Modes);
    let width = area.width.saturating_sub(4).clamp(
        1,
        if modes || view.picker == Some(Picker::Sessions) {
            96
        } else {
            72
        },
    );
    let height = (if modes {
        22
    } else {
        options.len().max(4) as u16 + 7
    })
    .min(area.height.saturating_sub(2))
    .max(1);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let (title, color) = match view.picker {
        Some(Picker::Models) => ("Models", BLUE),
        Some(Picker::Efforts) => ("Efforts", AMBER),
        Some(Picker::Modes) => ("Modes", LILAC),
        Some(Picker::SessionBoundaries) => (
            if view.session_boundaries_next.is_some() {
                "Settled boundaries · Enter fork · PageDown older"
            } else {
                "Settled boundaries · Enter fork"
            },
            MINT,
        ),
        Some(Picker::Sessions) => (
            "Sessions · Enter resume · Del remove · Ctrl+R restore · Ctrl+L rename · Ctrl+F fork",
            MINT,
        ),
        None => unreachable!("picker rendering requires an active picker"),
    };
    let block = panel(format!(" {title} "), color)
        .border_style(style(color))
        .style(style(TEXT).bg(RAISED));
    let inner = inset(block.inner(popup), 1, 0);
    frame.render_widget(block, popup);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(u16::from(inner.height >= 5) * 2),
        Constraint::Min(1),
        Constraint::Length(u16::from(inner.height >= 4) * 2),
    ])
    .split(inner);
    let query = if view.query.is_empty() {
        "Type to filter…"
    } else {
        &view.query
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" / ", bold(color)),
            Span::styled(
                clipped(query, usize::from(rows[0].width.saturating_sub(4))),
                style(if view.query.is_empty() { MUTED } else { TEXT }),
            ),
        ])),
        rows[0],
    );
    let columns = if modes && inner.width >= 62 {
        Layout::horizontal([Constraint::Length(21), Constraint::Min(0)])
            .split(rows[1])
            .to_vec()
    } else {
        vec![rows[1]]
    };
    let items = options
        .iter()
        .map(|label| {
            let current = match view.picker {
                Some(Picker::Models) => label == &view.model,
                Some(Picker::Efforts) => label == &view.effort,
                Some(Picker::Modes) => label == &view.mode,
                Some(Picker::SessionBoundaries) => false,
                Some(Picker::Sessions) => label.starts_with(&view.session),
                None => false,
            };
            let value = if modes {
                scene::identity(label).title
            } else {
                label
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{} ", if current { "●" } else { " " })),
                Span::raw(clipped(
                    value,
                    usize::from(columns[0].width.saturating_sub(5)),
                )),
            ]))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(" No matches. Backspace to edit.").style(style(MUTED)),
            columns[0],
        );
    } else {
        let mut selected = ListState::default().with_selected(Some(view.selected));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("› ")
                .highlight_style(bold(INK).bg(color)),
            columns[0],
            &mut selected,
        );
    }
    if let Some(preview_area) = columns.get(1) {
        frame.render_widget(Block::default().style(style(TEXT).bg(INK)), *preview_area);
        if let Some(mode) = options.get(view.selected) {
            let identity = scene::identity(mode);
            let mut preview = view.clone();
            preview.mode.clone_from(mode);
            preview.busy = false;
            preview.part_activity.clear();
            preview.relationships.clear();
            preview.routes.clear();
            preview.speaker_id.clear();
            if mode != &view.mode {
                preview.parts = kuru_core::Framework::builtin(mode.parse().expect("built-in mode"))
                    .parts
                    .into_iter()
                    .map(|part| (part.id, format!("{} · {}", part.name, part.role)))
                    .collect();
            }
            let shape = Rect::new(
                preview_area.x,
                preview_area.y,
                preview_area.width,
                preview_area.height.saturating_sub(3),
            );
            scene::draw(frame, &preview, shape);
            let caption = vec![
                Line::from(Span::styled(
                    format!("{} {}", identity.symbol, identity.title),
                    bold(identity.accent),
                ))
                .alignment(Alignment::Center),
                Line::from(Span::styled(identity.description, style(MUTED)))
                    .alignment(Alignment::Center),
                Line::from(Span::styled(
                    if mode == &view.mode {
                        "Current parts"
                    } else {
                        "Framework preview"
                    },
                    style(MUTED),
                ))
                .alignment(Alignment::Center),
            ];
            frame.render_widget(
                Paragraph::new(caption),
                Rect::new(
                    preview_area.x,
                    shape.bottom(),
                    preview_area.width,
                    preview_area.height.saturating_sub(shape.height),
                ),
            );
        }
    }
    let guide = if rows[2].width >= 54 {
        " ↑↓ move · Enter selects · Esc back · saved for project"
    } else {
        " Enter selects · Esc back"
    };
    frame.render_widget(Paragraph::new(guide).style(style(MUTED)), rows[2]);
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
