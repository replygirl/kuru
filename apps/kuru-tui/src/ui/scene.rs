//! Framework portraits: ornamental contours and factual peer activity are separate layers.

use std::f64::consts::{FRAC_PI_2, TAU};

use kuru_core::RelationshipKind;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::View;

const INK: Color = Color::Rgb(15, 19, 30);
const MINT: Color = Color::Rgb(131, 231, 199);
const BLUE: Color = Color::Rgb(135, 191, 250);
const AMBER: Color = Color::Rgb(241, 200, 129);
const LILAC: Color = Color::Rgb(193, 166, 247);
const ROSE: Color = Color::Rgb(242, 149, 173);
const GHOST: Color = Color::Rgb(43, 57, 76);
const TRACE: Color = Color::Rgb(62, 81, 104);

pub(super) struct Identity {
    pub title: &'static str,
    pub description: &'static str,
    pub symbol: &'static str,
    pub accent: Color,
}

pub(super) fn identity(mode: &str) -> Identity {
    match mode {
        "polyvagal" => Identity {
            title: "Polyvagal",
            description: "Connection, action, conservation",
            symbol: "~",
            accent: BLUE,
        },
        "freudian" => Identity {
            title: "Freudian",
            description: "Impulse, reality, standards",
            symbol: "△",
            accent: AMBER,
        },
        "jungian" => Identity {
            title: "Jungian",
            description: "Patterns, shadow, shared memory",
            symbol: "✧",
            accent: LILAC,
        },
        _ => Identity {
            title: "IFS",
            description: "Parts in relationship",
            symbol: "◌",
            accent: MINT,
        },
    }
}

#[derive(Clone, Copy, Debug)]
struct Point {
    x: i32,
    y: i32,
}

struct Canvas {
    width: u16,
    height: u16,
    cells: Vec<Option<(char, Color)>>,
}

impl Canvas {
    fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            cells: vec![None; usize::from(width) * usize::from(height)],
        }
    }

    fn point(&self, x: f64, y: f64) -> Point {
        Point {
            x: (x * f64::from(self.width.saturating_sub(1))).round() as i32,
            y: (y * f64::from(self.height.saturating_sub(1))).round() as i32,
        }
    }

    fn put(&mut self, point: Point, glyph: char, color: Color) {
        if point.x >= 0
            && point.y >= 0
            && point.x < i32::from(self.width)
            && point.y < i32::from(self.height)
        {
            self.cells[point.y as usize * usize::from(self.width) + point.x as usize] =
                Some((glyph, color));
        }
    }

    fn line(&mut self, start: Point, end: Point, glyph: char, color: Color) {
        let steps = (end.x - start.x).abs().max((end.y - start.y).abs());
        for step in 0..=steps {
            let t = f64::from(step) / f64::from(steps.max(1));
            self.put(interpolate(start, end, t), glyph, color);
        }
    }

    fn paint(&self, frame: &mut Frame<'_>, area: Rect) {
        for (index, value) in self.cells.iter().enumerate() {
            if let Some((glyph, color)) = value {
                let x = area.x + (index % usize::from(self.width)) as u16;
                let y = area.y + (index / usize::from(self.width)) as u16;
                frame.buffer_mut()[(x, y)]
                    .set_char(*glyph)
                    .set_fg(*color)
                    .set_bg(INK);
            }
        }
    }
}

fn interpolate(a: Point, b: Point, t: f64) -> Point {
    Point {
        x: (f64::from(a.x) + f64::from(b.x - a.x) * t).round() as i32,
        y: (f64::from(a.y) + f64::from(b.y - a.y) * t).round() as i32,
    }
}

fn mix(a: Color, b: Color, amount: f32) -> Color {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return a;
    };
    let channel = |left: u8, right: u8| {
        (f32::from(left) + (f32::from(right) - f32::from(left)) * amount.clamp(0.0, 1.0)).round()
            as u8
    };
    Color::Rgb(channel(ar, br), channel(ag, bg), channel(ab, bb))
}

fn role(label: &str) -> &str {
    label.rsplit(" · ").next().unwrap_or(label)
}

fn name(label: &str) -> &str {
    label.rsplit_once(" · ").map_or(label, |(name, _)| name)
}

fn color(label: &str) -> Color {
    match role(label) {
        "self" | "ego" | "ventral_vagal" => MINT,
        "manager" | "persona" | "superego" => BLUE,
        "firefighter" | "id" | "sympathetic" => AMBER,
        "exile" | "shadow" | "dorsal_vagal" => LILAC,
        _ => ROSE,
    }
}

fn relationship_color(kind: RelationshipKind) -> Color {
    match kind {
        RelationshipKind::Protection => BLUE,
        RelationshipKind::Polarization => AMBER,
        RelationshipKind::Alliance => LILAC,
    }
}

/// Render a portrait without querying time or inventing actor activity.
pub(super) fn draw(frame: &mut Frame<'_>, view: &View, area: Rect) {
    let area = area.intersection(frame.area());
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Block::default().style(Style::default().bg(INK)), area);
    let compact = area.width < 40 || area.height < 10;
    let mut canvas = Canvas::new(area.width, area.height);
    let phase = if view.motion {
        view.frame as f64 / 12.5
    } else {
        0.0
    };
    let accent = identity(&view.mode).accent;
    match view.mode.as_str() {
        "polyvagal" => flowing_traces(&mut canvas, phase, accent),
        "freudian" => triangle(&mut canvas, phase, accent),
        "jungian" => rosette(&mut canvas, phase, accent),
        _ => orbit(&mut canvas, phase, accent),
    }
    let positions = positions(view, &canvas);
    draw_relationships(&mut canvas, view, &positions, phase);
    canvas.paint(frame, area);
    draw_peers(frame, view, area, &positions, compact);
}

// Time changes only dim color, never contour cells or glyphs. A full pass
// takes 24 seconds, independent of typing and the busy-indicator frame rate.
fn ambient_color(base: Color, phase: f64, offset: f64, accent: Color) -> Color {
    let light = ((phase * TAU / 24.0 - offset).sin() + 1.0) as f32 / 2.0;
    mix(base, mix(TRACE, accent, 0.12), light)
}

fn ornament(canvas: &mut Canvas, point: Point, index: usize, phase: f64, accent: Color) {
    canvas.put(
        point,
        '.',
        ambient_color(GHOST, phase, index as f64 * 0.025, accent),
    );
}

fn orbit(canvas: &mut Canvas, phase: f64, accent: Color) {
    // Four interrupted orbits suggest regions without inventing network edges.
    for region in 0..4 {
        let start = region as f64 * FRAC_PI_2 + 0.16;
        for step in 0..48 {
            let angle = start + step as f64 / 47.0 * 1.06;
            let point = canvas.point(0.5 + 0.45 * angle.cos(), 0.48 + 0.43 * angle.sin());
            ornament(canvas, point, region * 48 + step, phase, accent);
        }
    }
    for step in 0..76 {
        let angle = step as f64 / 75.0 * TAU;
        if angle.sin().abs() > 0.88 {
            continue;
        }
        let point = canvas.point(0.5 + 0.24 * angle.cos(), 0.48 + 0.24 * angle.sin());
        ornament(canvas, point, step + 90, phase, accent);
    }
}

fn flowing_traces(canvas: &mut Canvas, phase: f64, accent: Color) {
    for band in 0..3 {
        let baseline = 0.19 + band as f64 * 0.30;
        for x in 0..canvas.width {
            let t = f64::from(x) / f64::from(canvas.width.saturating_sub(1).max(1));
            let wave = (t * TAU * 1.2 + band as f64 * 0.8).sin() * 0.065;
            let point = canvas.point(t, baseline + wave);
            canvas.put(
                point,
                '-',
                ambient_color(GHOST, phase, t * TAU + band as f64, accent),
            );
            if x % 4 == band {
                let echo = canvas.point(t, baseline + wave + 0.08);
                ornament(
                    canvas,
                    echo,
                    usize::from(x) + usize::from(band) * 19,
                    phase,
                    accent,
                );
            }
        }
    }
}

fn triangle(canvas: &mut Canvas, phase: f64, accent: Color) {
    for layer in 0..2 {
        let inset = layer as f64 * 0.085;
        let vertices = [
            canvas.point(0.5, 0.06 + inset),
            canvas.point(0.08 + inset, 0.86 - inset),
            canvas.point(0.92 - inset, 0.86 - inset),
        ];
        for side in 0..3 {
            let start = vertices[side];
            let end = vertices[(side + 1) % 3];
            let samples = usize::from(canvas.height).saturating_mul(2).clamp(12, 72);
            for step in 0..samples {
                let t = step as f64 / (samples - 1) as f64;
                if (0.44..0.59).contains(&t) {
                    continue;
                }
                let point = interpolate(start, end, t);
                let glyph = if side == 1 {
                    '-'
                } else if side == 0 {
                    '/'
                } else {
                    '\\'
                };
                canvas.put(
                    point,
                    glyph,
                    ambient_color(
                        if layer == 0 { TRACE } else { GHOST },
                        phase,
                        t * TAU + side as f64,
                        accent,
                    ),
                );
            }
        }
    }
}

fn rosette(canvas: &mut Canvas, phase: f64, accent: Color) {
    for ring in 0..2 {
        for step in 0..128 {
            let angle = step as f64 / 127.0 * TAU;
            let radius = 0.26 + ring as f64 * 0.11 + 0.075 * (angle * 4.0).cos();
            let point = canvas.point(
                0.5 + radius * angle.cos(),
                0.47 + radius * 0.91 * angle.sin(),
            );
            ornament(canvas, point, step + ring * 23, phase, accent);
        }
    }
    // A broken outer field keeps the rosette open rather than enclosing the peers.
    for step in 0..90 {
        if step % 5 > 2 {
            continue;
        }
        let angle = step as f64 / 89.0 * TAU;
        let point = canvas.point(0.5 + 0.47 * angle.cos(), 0.47 + 0.45 * angle.sin());
        canvas.put(point, '.', GHOST);
    }
}

fn positions(view: &View, canvas: &Canvas) -> Vec<Point> {
    let count = view.parts.len();
    let mut occurrences = std::collections::BTreeMap::<&str, usize>::new();
    view.parts
        .iter()
        .enumerate()
        .map(|(index, (_, label))| {
            let role = role(label);
            let entry = occurrences.entry(role).or_default();
            let order = *entry;
            *entry += 1;
            let same_role = view
                .parts
                .iter()
                .filter(|(_, other)| self::role(other) == role)
                .count();
            let spread = if same_role > 1 {
                order as f64 / (same_role - 1) as f64 - 0.5
            } else {
                0.0
            };
            let (x, y) = match view.mode.as_str() {
                "ifs" => match role {
                    "self" => (0.5 + spread * 0.2, 0.10),
                    "manager" => (0.79, 0.46 + spread * 0.34),
                    "firefighter" => (0.5 - spread * 0.30, 0.82),
                    "exile" => (0.21, 0.46 - spread * 0.34),
                    _ => (0.5, 0.5),
                },
                "polyvagal" => {
                    let band = match role {
                        "ventral_vagal" => 0,
                        "sympathetic" => 1,
                        _ => 2,
                    };
                    (
                        0.20 + band as f64 * 0.30 + spread * 0.17,
                        0.17 + band as f64 * 0.30,
                    )
                }
                "freudian" => match role {
                    "ego" => (0.5 + spread * 0.20, 0.12),
                    "id" => (0.20 + spread * 0.14, 0.78),
                    _ => (0.80 + spread * 0.14, 0.78),
                },
                _ => {
                    let angle = index as f64 / count.max(1) as f64 * TAU - FRAC_PI_2;
                    (0.5 + 0.35 * angle.cos(), 0.45 + 0.34 * angle.sin())
                }
            };
            canvas.point(x, y)
        })
        .collect()
}

fn draw_relationships(canvas: &mut Canvas, view: &View, positions: &[Point], phase: f64) {
    let position = |id: &str| {
        view.parts
            .iter()
            .position(|(part, _)| part == id)
            .map(|index| positions[index])
    };
    for relationship in &view.relationships {
        for (index, member) in relationship.members.iter().enumerate() {
            for other in &relationship.members[index + 1..] {
                if let (Some(start), Some(end)) = (position(member), position(other)) {
                    canvas.line(
                        start,
                        end,
                        '.',
                        mix(TRACE, relationship_color(relationship.kind), 0.38),
                    );
                }
            }
        }
    }
    for (sender, recipient) in &view.routes {
        if let (Some(start), Some(end)) = (position(sender), position(recipient)) {
            canvas.line(start, end, ':', mix(TRACE, MINT, 0.48));
            if view.busy && view.motion {
                canvas.put(
                    interpolate(start, end, 0.15 + (phase * 0.4).fract() * 0.70),
                    '*',
                    MINT,
                );
            }
        }
    }
}

fn draw_peers(frame: &mut Frame<'_>, view: &View, area: Rect, positions: &[Point], compact: bool) {
    for (index, ((id, label), point)) in view.parts.iter().zip(positions).enumerate() {
        let active = view.busy
            && view
                .part_activity
                .get(id)
                .is_some_and(|state| matches!(state.as_str(), "active" | "tool"));
        let state = view.part_activity.get(id).map(String::as_str);
        let glyph = if state == Some("error") {
            "!"
        } else if view.busy && id == &view.speaker_id {
            "◆"
        } else if active && view.motion {
            ["|", "/", "-", "\\"][view.frame as usize % 4]
        } else {
            "○"
        };
        let selected = Style::default()
            .fg(if state == Some("error") {
                ROSE
            } else {
                color(label)
            })
            .bg(INK)
            .add_modifier(Modifier::BOLD);
        let x = area.x + (point.x as u16).min(area.width - 1);
        let y = area.y + (point.y as u16).min(area.height - 1);
        if compact {
            let marker = format!("{glyph}{}", index + 1);
            let width = marker.width() as u16;
            let x = x.min(area.right().saturating_sub(width.min(area.width)));
            frame.render_widget(
                Paragraph::new(marker).style(selected),
                Rect::new(x, y, width.min(area.width), 1),
            );
        } else {
            frame.render_widget(Paragraph::new(glyph).style(selected), Rect::new(x, y, 1, 1));
            let label = format!(" {} ", name(label));
            let width = (label.width() as u16).min(area.width);
            let left = x
                .saturating_sub(width / 2)
                .max(area.x)
                .min(area.right().saturating_sub(width));
            let row = (y + 1).min(area.bottom() - 1);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    label,
                    selected.remove_modifier(Modifier::BOLD),
                ))),
                Rect::new(left, row, width, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use kuru_core::{Framework, Mode, Relationship};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use super::*;
    use crate::ui::{InitialViewData, RuntimeSnapshot};

    fn view(mode: Mode) -> View {
        let framework = Framework::builtin(mode);
        let mut view = View::from_initial(
            InitialViewData {
                transcript: vec![],
                session: "plain-session".into(),
                project: "plain-project".into(),
                motion: true,
                runtime: RuntimeSnapshot {
                    turns: 0,
                    mode: mode.to_string(),
                    model: "demo".into(),
                    effort: "default".into(),
                    parts: framework
                        .parts
                        .into_iter()
                        .map(|part| (part.id, format!("{} · {}", part.name, part.role)))
                        .collect(),
                    relationships: vec![],
                    focus: None,
                },
                usage: None,
            },
            vec![],
        );
        view.motion = true;
        view.focused = true;
        view
    }

    fn render(view: &View, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, view, frame.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn text(buffer: &Buffer) -> String {
        buffer.content.iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn framework_silhouettes_are_distinct_and_keep_real_peer_names() {
        let mut silhouettes = BTreeSet::new();
        for mode in Mode::ALL {
            let view = view(mode);
            let buffer = render(&view, 65, 14);
            let screen = text(&buffer);
            if let Some(directory) = std::env::var_os("KURU_VISUAL_ARTIFACTS") {
                std::fs::create_dir_all(&directory).unwrap();
                let rows = buffer
                    .content
                    .chunks(65)
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                std::fs::write(
                    std::path::Path::new(&directory).join(format!("scene-{mode}.txt")),
                    rows,
                )
                .unwrap();
            }
            for (_, label) in &view.parts {
                assert!(screen.contains(name(label)), "{mode} lost {}", name(label));
            }
            let contour = buffer
                .content
                .iter()
                .map(|cell| {
                    if matches!(cell.symbol(), "." | ":" | "-" | "/" | "\\") {
                        cell.symbol()
                    } else {
                        " "
                    }
                })
                .collect::<String>();
            silhouettes.insert(contour);
        }
        assert_eq!(silhouettes.len(), 4);
    }

    #[test]
    fn reduced_motion_freezes_contour_color_and_keeps_peer_names() {
        let mut view = view(Mode::Ifs);
        view.motion = false;
        let still = render(&view, 65, 14);
        for (_, label) in &view.parts {
            assert!(text(&still).contains(name(label)));
        }
        view.frame = 900;
        assert_eq!(still, render(&view, 65, 14));
    }

    #[test]
    fn real_relationships_and_routes_change_the_portrait_unknown_endpoints_do_not() {
        let mut view = view(Mode::Freudian);
        let baseline = render(&view, 65, 14);
        view.routes.push(("unknown".into(), "missing".into()));
        assert_eq!(baseline, render(&view, 65, 14));
        let first = view.parts[0].0.clone();
        let second = view.parts[1].0.clone();
        view.relationships.push(
            Relationship::new(
                RelationshipKind::Protection,
                vec![first.clone(), second.clone()],
            )
            .unwrap(),
        );
        assert_ne!(baseline, render(&view, 65, 14));
        view.routes.push((first, second));
        view.busy = true;
        let route = render(&view, 65, 14);
        view.frame = 13;
        assert_ne!(route, render(&view, 65, 14));
        assert!(text(&route).contains('*'));
    }

    #[test]
    fn compact_portraits_number_peers_and_survive_tiny_clipped_areas() {
        for mode in Mode::ALL {
            let view = view(mode);
            let compact = text(&render(&view, 31, 9));
            for index in 1..=view.parts.len() {
                assert!(compact.contains(&format!("○{index}")));
            }
            for (width, height) in [(1, 1), (2, 3), (8, 5), (16, 6), (40, 2)] {
                render(&view, width, height);
            }
            let mut terminal = Terminal::new(TestBackend::new(8, 4)).unwrap();
            terminal
                .draw(|frame| draw(frame, &view, Rect::new(6, 2, 20, 20)))
                .unwrap();
        }
    }
}
