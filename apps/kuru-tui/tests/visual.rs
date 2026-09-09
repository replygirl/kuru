use std::{collections::BTreeSet, fmt::Write as _, sync::Arc};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kuru::ui::{View, draw};
use kuru_connectors::DemoProvider;
use kuru_core::{Config, MemoryStore, Mode, ModelInfo, RelationshipKind};
use kuru_runtime::{Event, Harness, PeerMessage};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};
use unicode_width::UnicodeWidthStr;

fn fixture(mode: Mode) -> (tempfile::TempDir, Harness, View) {
    let project = tempfile::tempdir().unwrap();
    let harness = Harness::new(
        Config {
            provider: "demo".into(),
            model: "demo".into(),
            mode,
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        project.path(),
        MemoryStore::in_memory().unwrap(),
        Arc::new(DemoProvider),
        None,
    )
    .unwrap();
    let view = View::new(
        &harness,
        vec![ModelInfo {
            id: "demo".into(),
            name: "Offline demo".into(),
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("low".into()),
        }],
    )
    .unwrap();
    (project, harness, view)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn render(view: &View, width: u16, height: u16, name: &str) -> (Buffer, (u16, u16)) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(frame, view)).unwrap();
    let position = terminal.get_cursor_position().unwrap();
    let buffer = terminal.backend().buffer().clone();
    if let Some(directory) = std::env::var_os("KURU_VISUAL_ARTIFACTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{name}.html")), html(&buffer)).unwrap();
        std::fs::write(directory.join(format!("{name}.txt")), text(&buffer)).unwrap();
    }
    (buffer, (position.x, position.y))
}

fn text(buffer: &Buffer) -> String {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// Optional artifacts capture the actual Ratatui cells, including colors, rather
// than a separate mockup. KURU_VISUAL_ARTIFACTS=/tmp/kuru-visual enables export.
fn html(buffer: &Buffer) -> String {
    fn color(color: Color) -> String {
        match color {
            Color::Rgb(r, g, b) => format!("rgb({r},{g},{b})"),
            Color::Reset => "inherit".into(),
            Color::Black => "#000".into(),
            Color::White => "#fff".into(),
            Color::Gray => "#aaa".into(),
            Color::DarkGray => "#555".into(),
            Color::Red | Color::LightRed => "#f77".into(),
            Color::Green | Color::LightGreen => "#8e9".into(),
            Color::Blue | Color::LightBlue => "#89f".into(),
            Color::Yellow | Color::LightYellow => "#ed9".into(),
            Color::Magenta | Color::LightMagenta => "#e8d".into(),
            Color::Cyan | Color::LightCyan => "#8de".into(),
            Color::Indexed(value) => format!("rgb({value},{value},{value})"),
        }
    }
    let mut output = String::from(
        "<!doctype html><meta charset=utf-8><title>Kuru terminal render</title>\
         <style>body{background:#0c111a;color:#dce7f2;padding:24px}pre{font:14px/1.25 \"SF Mono\",Menlo,monospace;white-space:pre}</style><pre>",
    );
    for row in buffer.content.chunks(usize::from(buffer.area.width)) {
        let mut continuation = 0;
        for cell in row {
            if continuation > 0 {
                continuation -= 1;
                continue;
            }
            continuation = cell.symbol().width().saturating_sub(1);
            let symbol = cell
                .symbol()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let foreground = if cell.modifier.contains(Modifier::REVERSED) {
                cell.bg
            } else {
                cell.fg
            };
            let background = if cell.modifier.contains(Modifier::REVERSED) {
                cell.fg
            } else {
                cell.bg
            };
            let mut modifiers = String::new();
            for (flag, css) in [
                (Modifier::BOLD, "font-weight:bold;"),
                (Modifier::ITALIC, "font-style:italic;"),
                (Modifier::UNDERLINED, "text-decoration:underline;"),
                (Modifier::DIM, "opacity:0.65;"),
            ] {
                if cell.modifier.contains(flag) {
                    modifiers.push_str(css);
                }
            }
            write!(
                output,
                "<span style=\"color:{};background:{};{modifiers}\">{symbol}</span>",
                color(foreground),
                color(background)
            )
            .unwrap();
        }
        output.push('\n');
    }
    output.push_str("</pre>");
    output
}

#[tokio::test]
async fn welcome_shows_actual_framework_members_and_color_at_wide_and_compact_sizes() {
    for mode in [Mode::Ifs, Mode::Polyvagal, Mode::Freudian, Mode::Jungian] {
        let (_project, harness, mut view) = fixture(mode);
        view.motion = false;
        let (wide, _) = render(&view, 140, 50, &format!("welcome-{mode}"));
        let screen = text(&wide);
        for label in ["KURU", "demo", "F2", "F3", "F4"] {
            assert!(screen.contains(label), "missing {label}:\n{screen}");
        }
        assert!(screen.to_ascii_lowercase().contains(&mode.to_string()));
        for part in &harness.topology.parts {
            assert!(screen.contains(&part.name), "missing {}", part.name);
        }
        let accents = wide
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .map(|cell| format!("{:?}", cell.fg))
            .collect::<BTreeSet<_>>();
        assert!(
            accents.len() >= 4,
            "identity and hierarchy need distinct color"
        );
        view.input = "A compact message".into();
        view.cursor = view.input.len();
        let (compact, cursor) = render(&view, 70, 25, &format!("compact-{mode}"));
        let screen = text(&compact);
        for label in ["KURU", "demo", "A compact message"] {
            assert!(screen.contains(label), "missing {label}:\n{screen}");
        }
        assert!(cursor.0 < 70 && cursor.1 < 25);
    }
}

#[tokio::test]
async fn real_turn_events_keep_speaking_group_members_and_peer_routes_visible() {
    let (_project, mut harness, _) = fixture(Mode::Freudian);
    let members = harness.topology.parts[..2]
        .iter()
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    let relation = harness
        .relate(RelationshipKind::Alliance, members.clone())
        .unwrap();
    let names = harness.topology.parts[..2]
        .iter()
        .map(|part| part.name.clone())
        .collect::<Vec<_>>();
    let member_names = relation
        .members
        .iter()
        .map(|id| {
            harness
                .topology
                .parts
                .iter()
                .find(|part| &part.id == id)
                .unwrap()
                .name
                .as_str()
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let mut view = View::new(&harness, vec![]).unwrap();
    view.motion = false;
    view.transcript
        .push(("user".into(), "Shared design".into()));
    let outcome = harness.run("Shared design").await.unwrap();
    assert_eq!(outcome.speaker, relation.id);
    for event in outcome.events {
        view.event(event);
    }
    let route = PeerMessage::new(
        &members[0],
        &members[1],
        &harness.session.id,
        "PRIVATE_PEER_TEXT_MUST_STAY_INTERNAL",
    )
    .unwrap();
    view.event(Event {
        kind: "peer".into(),
        actor: members[0].clone(),
        detail: route.rpc().to_string(),
    });
    let (buffer, _) = render(&view, 120, 45, "conversation-alliance");
    let screen = text(&buffer);
    for label in [
        "Shared design",
        "alliance",
        "Relationships",
        "Peer exchange",
    ] {
        assert!(screen.contains(label), "missing {label}:\n{screen}");
    }
    assert!(screen.contains(&member_names));
    assert!(screen.contains(&format!("{} → {}", names[0], names[1])));
    assert!(!screen.contains("PRIVATE_PEER_TEXT_MUST_STAY_INTERNAL"));
    assert_eq!(view.speaker_id, relation.id);
    assert_eq!(
        view.routes.last(),
        Some(&(members[0].clone(), members[1].clone()))
    );
    assert_eq!(view.transcript.last().unwrap().1, outcome.text);
    assert!(view.part_activity.values().any(|value| value == "idle"));
}

#[tokio::test]
async fn motion_changes_decoration_without_changing_text_and_respects_static_override() {
    let (_project, harness, mut view) = fixture(Mode::Ifs);
    view.busy = true;
    view.transcript
        .push(("user".into(), "Keep this content readable".into()));
    view.input = "Draft remains editable".into();
    view.cursor = view.input.len();
    view.motion = true;
    let members = &harness.topology.parts[..2];
    for part in members {
        view.event(Event {
            kind: "active".into(),
            actor: part.id.clone(),
            detail: "peer round 1".into(),
        });
    }
    let route = PeerMessage::new(
        &members[0].id,
        &members[1].id,
        &view.session,
        "Check the draft",
    )
    .unwrap();
    view.event(Event {
        kind: "peer".into(),
        actor: members[0].id.clone(),
        detail: route.rpc().to_string(),
    });
    let (first, cursor) = render(&view, 120, 45, "busy-frame-zero");
    view.frame = 17;
    let (moving, next_cursor) = render(&view, 120, 45, "busy-frame-later");
    assert_ne!(first, moving, "busy activity should visibly animate");
    assert!(
        (5..14).any(|y| (88..118).any(|x| first[(x, y)] != moving[(x, y)])),
        "the routed peer graph itself should animate, not only the header"
    );
    assert_eq!(cursor, next_cursor, "animation must not move the editor");
    for buffer in [&first, &moving] {
        let screen = text(buffer);
        assert!(screen.contains("Keep this content readable"));
        assert!(screen.contains("Draft remains editable"));
        for part in &harness.topology.parts {
            assert!(
                screen.contains(&part.name),
                "graph legend lost {}",
                part.name
            );
        }
    }
    view.motion = false;
    let (still, _) = render(&view, 120, 45, "busy-reduced-motion");
    view.frame = 91;
    let (later, _) = render(&view, 120, 45, "busy-reduced-motion-later");
    assert_eq!(
        still, later,
        "reduced motion must suppress every changing cell"
    );
    assert!(view.busy);
    view.key(key(KeyCode::Char('!')));
    assert_eq!(view.input, "Draft remains editable!");
}

#[tokio::test]
async fn long_model_catalog_scrolls_selection_into_view_and_preserves_draft() {
    let (_project, _harness, mut view) = fixture(Mode::Freudian);
    view.input = "An unsent draft".into();
    view.cursor = view.input.len();
    view.models = (0..30)
        .map(|index| ModelInfo {
            id: format!("model-{index:02}"),
            name: format!("Model {index}"),
            efforts: vec![],
            default_effort: None,
        })
        .collect();
    view.key(key(KeyCode::F(2)));
    for _ in 0..24 {
        view.key(key(KeyCode::Down));
    }
    let (buffer, _) = render(&view, 70, 18, "model-selector");
    let screen = text(&buffer);
    for label in ["Models", "Enter selects", "model-24"] {
        assert!(screen.contains(label), "missing {label}:\n{screen}");
    }
    assert!(
        !screen.contains("model-00"),
        "offscreen choices should scroll away"
    );
    assert_eq!(
        view.key(key(KeyCode::Enter)).as_deref(),
        Some("/model model-24")
    );
    assert_eq!(view.input, "An unsent draft");
    view.key(key(KeyCode::F(4)));
    view.key(key(KeyCode::Esc));
    assert!(view.picker.is_none());
    assert_eq!(
        view.key(key(KeyCode::Enter)).as_deref(),
        Some("An unsent draft")
    );
}

#[tokio::test]
async fn unicode_editor_and_cursor_survive_resize_even_below_supported_layout_size() {
    let (_project, _harness, mut view) = fixture(Mode::Jungian);
    for ch in "long draft 猫 🌿 ".repeat(6).chars() {
        view.key(key(KeyCode::Char(ch)));
    }
    view.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    for ch in "final line".chars() {
        view.key(key(KeyCode::Char(ch)));
    }
    let draft = view.input.clone();
    for (width, height) in [
        (120, 35),
        (40, 15),
        (38, 7),
        (20, 8),
        (14, 7),
        (10, 5),
        (1, 1),
    ] {
        let (buffer, cursor) = render(&view, width, height, &format!("editor-{width}x{height}"));
        assert!(cursor.0 < width && cursor.1 < height);
        if width >= 40 {
            assert!(text(&buffer).contains("final line"));
        }
        assert_eq!(view.input, draft);
    }
    assert_eq!(view.key(key(KeyCode::Enter)).unwrap(), draft);
}

#[tokio::test]
async fn rich_answer_preserves_prose_code_and_list_content_when_terminal_wraps() {
    let (_project, _harness, mut view) = fixture(Mode::Ifs);
    view.motion = false;
    view.transcript = vec![
        ("user".into(), "Show me the next step".into()),
        (
            "Self · self".into(),
            "# The next step\nKeep **important words** and `inline_code` readable.\n> A useful observation\n- First item\n* Second item\n```rust\nlet greeting = \"hello 猫\";\n```\nAn unmatched ` stays visible.\nFinal marker: green".into(),
        ),
    ];
    for (width, height) in [(120, 40), (54, 32)] {
        let (buffer, _) = render(&view, width, height, &format!("rich-answer-{width}"));
        let screen = text(&buffer);
        for label in [
            "The next step",
            "important words",
            "inline_code",
            "A useful observation",
            "First item",
            "Second item",
            "let greeting",
            "An unmatched ` stays visible.",
            "Final marker: green",
        ] {
            assert!(screen.contains(label), "missing {label}:\n{screen}");
        }
        assert!(!screen.contains("**important words**"));
        assert!(!screen.contains("```"));
        assert!(
            buffer
                .content
                .windows("important words".len())
                .any(|cells| {
                    cells.iter().map(|cell| cell.symbol()).collect::<String>() == "important words"
                        && cells
                            .iter()
                            .all(|cell| cell.modifier.contains(Modifier::BOLD))
                }),
            "the answer's emphasized words must retain bold styling after wrapping"
        );
    }
}

#[tokio::test]
async fn composer_combines_values_with_hints_and_status_prioritizes_errors() {
    let (_project, _harness, mut view) = fixture(Mode::Freudian);
    view.model = "gpt-current-long-model-name".into();
    view.effort = "high".into();
    view.motion = false;
    for (width, height) in [(140, 50), (72, 25), (70, 25), (40, 20)] {
        let (buffer, _) = render(&view, width, height, &format!("controls-{width}"));
        let screen = text(&buffer);
        let rows: Vec<_> = screen.lines().collect();
        assert!(!rows[0].contains("gpt-"));
        assert!(!screen.contains("Message"));
        assert!(!screen.contains("Motion"));
        for (value, hint) in [("gpt-", "F2"), ("high", "F3"), ("Freudian", "F4")] {
            let row = rows.iter().rposition(|line| line.contains(hint)).unwrap();
            assert!(
                row >= usize::from(height) - 5,
                "{hint} is away from composer"
            );
            assert!(
                rows[row].contains(value),
                "value separated from {hint}: {}",
                rows[row]
            );
        }
    }
    view.model = "very-long-future-model-name-".repeat(5);
    view.effort = "future-effort-level-".repeat(5);
    for width in [38, 68, 72, 140] {
        let (buffer, _) = render(&view, width, 25, &format!("future-controls-{width}"));
        let screen = text(&buffer);
        for key in ["F2", "F3", "F4"] {
            assert!(
                screen.contains(key),
                "{key} lost for unknown long values at {width}"
            );
        }
    }
    view.model = "gpt-current-long-model-name".into();
    view.effort = "high".into();
    view.speaker = "Reality · ego".into();
    view.show_scene = false;
    view.status = "Cancelled".into();
    let (buffer, _) = render(&view, 70, 25, "compact-status");
    assert!(text(&buffer).contains("Cancelled"));
    view.status = "Failed · details in conversation".into();
    view.transcript.push((
        "error".into(),
        "Provider unavailable. Try another model.".into(),
    ));
    let (buffer, _) = render(&view, 70, 25, "provider-error");
    assert!(text(&buffer).contains("Provider unavailable"));
    assert!(text(&buffer).contains("Failed"));
    view.status = "Complete".into();
    let (buffer, _) = render(&view, 70, 25, "completed-status");
    assert!(text(&buffer).contains("Reality · ego"));
}

#[tokio::test]
async fn mode_picker_previews_selection_and_filtering_never_changes_the_live_pool() {
    let (_project, _harness, mut view) = fixture(Mode::Ifs);
    view.key(key(KeyCode::F(4)));
    let original = view.parts.clone();
    for mode in ["ifs", "polyvagal", "freudian", "jungian"] {
        view.query = mode.into();
        view.selected = 0;
        let (buffer, _) = render(&view, 140, 50, &format!("picker-{mode}"));
        let screen = text(&buffer);
        assert!(screen.contains(if mode == "ifs" {
            "Current parts"
        } else {
            "Framework preview"
        }));
        assert_eq!(view.parts, original);
        assert_eq!(view.mode, "ifs");
    }
    view.query = "nonexistent".into();
    let (buffer, _) = render(&view, 70, 22, "picker-no-matches");
    assert!(text(&buffer).contains("No matches"));
    assert!(view.key(key(KeyCode::Enter)).is_none());
    assert!(view.picker.is_some());
    view.key(key(KeyCode::Esc));
    view.key(key(KeyCode::F(3)));
    assert!(view.options().contains(&"default".into()));
    assert_eq!(
        view.key(key(KeyCode::Enter)).as_deref(),
        Some("/effort default")
    );
}

#[tokio::test]
async fn ambient_time_preserves_labels_draft_caret_and_static_override() {
    let (_project, harness, mut view) = fixture(Mode::Jungian);
    view.motion = true;
    let (first, _) = render(&view, 140, 50, "ambient-zero");
    view.advance_animation(std::time::Duration::from_secs(12));
    let (ambient, _) = render(&view, 140, 50, "ambient-later");
    assert_ne!(first, ambient, "ambient motion continues beyond welcome");
    for c in "A thought".chars() {
        view.key(key(KeyCode::Char(c)));
    }
    let (typed, cursor) = render(&view, 140, 50, "ambient-draft-start");
    view.advance_animation(std::time::Duration::from_millis(12500));
    let (later_frame, after) = render(&view, 140, 50, "ambient-draft-later");
    assert_ne!(typed, later_frame);
    assert_eq!(cursor, after);
    for part in &harness.topology.parts {
        let locate = |buffer: &Buffer| text(buffer).find(&part.name).unwrap();
        assert_eq!(locate(&typed), locate(&later_frame));
    }
    assert!(text(&later_frame).contains("A thought"));
    view.motion = false;
    let (still, _) = render(&view, 140, 50, "ambient-static");
    view.frame += 97;
    let (later, _) = render(&view, 140, 50, "ambient-static-later");
    assert_eq!(still, later);
}

// Explicit profiling fixture, excluded from normal assertions because timings
// depend on the machine. Exercises actual cells, not a separate scene mockup.
#[tokio::test]
#[ignore = "run explicitly with --ignored --nocapture to measure frame cost"]
async fn frame_cost_profile() {
    let (_project, _harness, mut view) = fixture(Mode::Ifs);
    let mut terminal = Terminal::new(TestBackend::new(140, 50)).unwrap();
    for state in ["welcome", "long conversation", "picker"] {
        if state == "long conversation" {
            view.transcript = (0..500)
                .map(|i| {
                    (
                        "Self".into(),
                        format!("Turn {i}: {}", "useful content ".repeat(40)),
                    )
                })
                .collect();
            view.show_scene = false;
        } else if state == "picker" {
            view.key(key(KeyCode::F(4)));
        }
        terminal.draw(|f| draw(f, &view)).unwrap();
        let start = std::time::Instant::now();
        for frame in 0..200 {
            view.frame = frame;
            terminal.draw(|f| draw(f, &view)).unwrap();
        }
        eprintln!(
            "{state}: {:.3} ms/frame (200 frames, 140x50)",
            start.elapsed().as_secs_f64() * 5.0
        );
    }
}

#[tokio::test]
async fn quiet_typing_leaves_portrait_and_composer_decoration_untouched() {
    for mode in Mode::ALL {
        let (_project, _harness, mut view) = fixture(mode);
        view.motion = true;
        let (baseline, _) = render(&view, 140, 50, &format!("quiet-before-{mode}"));
        // Rows 0..45 include the complete scene and composer separator; the
        // editable draft starts on row 45. The clock stays fixed throughout.
        for c in "A thought".chars() {
            view.key(key(KeyCode::Char(c)));
            let (typed, _) = render(&view, 140, 50, &format!("quiet-typed-{mode}"));
            assert!(
                baseline.content[..140 * 45] == typed.content[..140 * 45],
                "typing disturbed the {mode} portrait or composer decoration"
            );
        }
        view.paste(" and a pasted continuation");
        view.key(key(KeyCode::Backspace));
        view.key(key(KeyCode::Home));
        view.key(key(KeyCode::Delete));
        let (edited, _) = render(&view, 140, 50, &format!("quiet-edited-{mode}"));
        assert!(
            baseline.content[..140 * 45] == edited.content[..140 * 45],
            "paste/edit disturbed the {mode} scene"
        );
        assert_eq!(view.input, " thought and a pasted continuatio");
    }
}

#[tokio::test]
async fn quiet_ambient_keeps_every_glyph_fixed_and_changes_color_gradually() {
    for mode in Mode::ALL {
        let (_project, _harness, mut view) = fixture(mode);
        view.motion = true;
        let (baseline, _) = render(&view, 140, 50, &format!("quiet-ambient-zero-{mode}"));
        let mut colored = false;
        for frame in [75, 150, 225, 300] {
            view.frame = frame;
            let (later, _) = render(&view, 140, 50, &format!("quiet-ambient-{frame}-{mode}"));
            assert!(
                text(&baseline) == text(&later),
                "ambient animation changed glyphs or positions in {mode}"
            );
            colored |= baseline != later;
        }
        assert!(
            colored,
            "{mode} should still have time-driven ambient color"
        );
        view.frame = 0;
        let (first, _) = render(&view, 140, 50, "quiet-smooth-before");
        view.advance_animation(std::time::Duration::from_millis(250));
        let (next, _) = render(&view, 140, 50, "quiet-smooth-after");
        for (a, b) in first.content.iter().zip(&next.content) {
            if let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a.fg, b.fg) {
                assert!(
                    ar.abs_diff(br).max(ag.abs_diff(bg)).max(ab.abs_diff(bb)) <= 2,
                    "abrupt brightness step in {mode}"
                );
            }
        }
    }
}
