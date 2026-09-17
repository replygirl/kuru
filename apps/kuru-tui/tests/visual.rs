use std::{collections::BTreeSet, fmt::Write as _};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kuru::ui::{InitialViewData, RuntimeSnapshot, View, draw};
use kuru_core::{ContextBudget, ContextEstimate, Framework, Mode, ModelInfo, Sourced, UsagePhase};
use kuru_runtime::{Event, FacingProgress, PeerMessage, RequestContext};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};
use unicode_width::UnicodeWidthStr;

fn fixture(mode: Mode) -> View {
    let framework = Framework::builtin(mode);
    View::from_initial(
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
        vec![ModelInfo {
            id: "demo".into(),
            name: "Offline demo".into(),
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("low".into()),
            metadata: Default::default(),
        }],
    )
}

#[test]
fn provisional_tail_and_thinking_remain_separate_from_transcript_at_practical_sizes() {
    let mut view = fixture(Mode::Ifs);
    view.show_scene = false;
    view.busy = true;
    view.input = "draft for next turn".into();
    view.preview = Some(FacingProgress {
        turn_id: "turn".into(),
        request_round: 1,
        seq: 3,
        text_tail: "earlier\nsecond\nthird\nVISIBLE_PARTIAL_TAIL".into(),
        text_truncated: false,
        summary_tail: "VISIBLE_THINKING_SUMMARY\ncontinued".into(),
        summary_truncated: false,
        activity: "selected voice composing".into(),
        activity_truncated: false,
    });
    for (width, height) in [(80, 24), (120, 40), (40, 18)] {
        let (buffer, cursor) = render(&view, width, height, &format!("provisional-{width}"));
        let screen = text(&buffer);
        assert!(
            screen.contains("VISIBLE_PARTIAL_TAIL"),
            "missing preview at {width}x{height}: {screen}"
        );
        assert!(
            screen.contains("thinking"),
            "missing summary label at {width}x{height}: {screen}"
        );
        assert!(
            screen.contains("activity"),
            "missing activity at {width}x{height}: {screen}"
        );
        assert!(
            screen.contains("earlier text omitted"),
            "hidden draft lacks truncation cue at {width}x{height}: {screen}"
        );
        assert!(
            screen.contains("draft for next turn"),
            "composer lost at {width}x{height}: {screen}"
        );
        assert!(cursor.0 < width && cursor.1 < height);
        assert!(view.transcript.is_empty());
    }
}

#[test]
fn prepared_context_status_is_bounded_and_keeps_the_composer_at_practical_sizes() {
    let mut view = fixture(Mode::Ifs);
    view.show_scene = false;
    view.busy = true;
    view.status = "Listening to the parts".into();
    view.input = "unsent draft".into();
    view.active_operation_id = Some("turn".into());
    view.request_context = Some(RequestContext {
        operation_id: "turn".into(),
        actor_id: view.parts[0].0.clone(),
        phase: UsagePhase::Speak,
        estimate: ContextEstimate::for_final_body(
            ContextBudget::resolve(Sourced::advertised(128_000), None, None).unwrap(),
            1_024,
            false,
            vec![],
        ),
        runtime_sources: vec![],
        omitted_public_rows: 2,
        omitted_private_rows: 1,
        omitted_note_rows: 0,
    });
    for (width, height) in [(40, 18), (80, 24), (120, 35)] {
        let (buffer, cursor) = render(&view, width, height, "context-status");
        let screen = text(&buffer);
        assert!(screen.contains("est"), "{width}x{height}: {screen}");
        assert!(
            screen.contains("512+8192/128000"),
            "{width}x{height}: {screen}"
        );
        assert!(
            screen.contains("unsent draft"),
            "{width}x{height}: {screen}"
        );
        if width == 40 {
            assert!(screen.contains("adv"), "{screen}");
        }
        if width == 120 {
            assert!(screen.contains("advertised"), "{screen}");
        }
        assert!(cursor.0 < width && cursor.1 < height);
    }
    view.request_context
        .as_mut()
        .unwrap()
        .estimate
        .budget
        .window = Sourced::built_in(128_000);
    let (buffer, _) = render(&view, 40, 18, "assumed-context-status");
    assert!(text(&buffer).contains("assumed"));
    view.active_operation_id = Some("other".into());
    let (buffer, _) = render(&view, 120, 35, "foreign-context-status");
    assert!(!text(&buffer).contains("prepared est"));
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

#[test]
fn welcome_shows_actual_framework_members_and_color_at_wide_and_compact_sizes() {
    for mode in [Mode::Ifs, Mode::Polyvagal, Mode::Freudian, Mode::Jungian] {
        let mut view = fixture(mode);
        view.motion = false;
        let (wide, _) = render(&view, 140, 50, &format!("welcome-{mode}"));
        let screen = text(&wide);
        for label in ["KURU", "demo", "F2", "F3", "F4"] {
            assert!(screen.contains(label), "missing {label}:\n{screen}");
        }
        assert!(screen.to_ascii_lowercase().contains(&mode.to_string()));
        for (_, label) in &view.parts {
            let name = label.split_once(" · ").unwrap().0;
            assert!(screen.contains(name), "missing {name}");
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

#[test]
fn motion_changes_decoration_without_changing_text_and_respects_static_override() {
    let mut view = fixture(Mode::Ifs);
    view.busy = true;
    view.transcript
        .push(("user".into(), "Keep this content readable".into()));
    view.input = "Draft remains editable".into();
    view.cursor = view.input.len();
    view.motion = true;
    let members = view.parts[..2].to_vec();
    for (id, _) in &members {
        view.event(Event::Active {
            actor: id.clone(),
            detail: "peer round 1".into(),
        });
    }
    let route = PeerMessage::new(
        &members[0].0,
        &members[1].0,
        &view.session,
        "Check the draft",
    )
    .unwrap();
    view.event(Event::Peer {
        actor: members[0].0.clone(),
        envelope: route.rpc(),
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
        for (_, label) in &view.parts {
            let name = label.split_once(" · ").unwrap().0;
            assert!(screen.contains(name), "graph legend lost {name}",);
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

#[test]
fn long_model_catalog_scrolls_selection_into_view_and_preserves_draft() {
    let mut view = fixture(Mode::Freudian);
    view.input = "An unsent draft".into();
    view.cursor = view.input.len();
    view.models = (0..30)
        .map(|index| ModelInfo {
            id: format!("model-{index:02}"),
            name: format!("Model {index}"),
            efforts: vec![],
            default_effort: None,
            metadata: Default::default(),
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

#[test]
fn unicode_editor_and_cursor_survive_resize_even_below_supported_layout_size() {
    let mut view = fixture(Mode::Jungian);
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

#[test]
fn rich_answer_preserves_prose_code_and_list_content_when_terminal_wraps() {
    let mut view = fixture(Mode::Ifs);
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

#[test]
fn composer_combines_values_with_hints_and_status_prioritizes_errors() {
    let mut view = fixture(Mode::Freudian);
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

#[test]
fn mode_picker_previews_selection_and_filtering_never_changes_the_live_pool() {
    let mut view = fixture(Mode::Ifs);
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

#[test]
fn ambient_time_preserves_labels_draft_caret_and_static_override() {
    let mut view = fixture(Mode::Jungian);
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
    for (_, label) in &view.parts {
        let name = label.split_once(" · ").unwrap().0;
        let locate = |buffer: &Buffer| text(buffer).find(name).unwrap();
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
#[test]
#[ignore = "run explicitly with --ignored --nocapture to measure frame cost"]
fn frame_cost_profile() {
    let mut view = fixture(Mode::Ifs);
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

#[test]
fn quiet_typing_leaves_portrait_and_composer_decoration_untouched() {
    for mode in Mode::ALL {
        let mut view = fixture(mode);
        view.motion = true;
        let (baseline, _) = render(&view, 140, 50, &format!("quiet-before-{mode}"));
        // The dock can gain control rows. Locate its separator so the fixed
        // scene and decoration are compared without including editable input.
        let separator = text(&baseline)
            .lines()
            .enumerate()
            .filter(|(_, row)| row.trim_start().starts_with(".    ."))
            .map(|(index, _)| index)
            .last()
            .expect("composer separator");
        let fixed_cells = 140 * (separator + 1);
        for c in "A thought".chars() {
            view.key(key(KeyCode::Char(c)));
            let (typed, _) = render(&view, 140, 50, &format!("quiet-typed-{mode}"));
            assert!(
                baseline.content[..fixed_cells] == typed.content[..fixed_cells],
                "typing disturbed the {mode} portrait or composer decoration"
            );
        }
        view.paste(" and a pasted continuation");
        view.key(key(KeyCode::Backspace));
        view.key(key(KeyCode::Home));
        view.key(key(KeyCode::Delete));
        let (edited, _) = render(&view, 140, 50, &format!("quiet-edited-{mode}"));
        assert!(
            baseline.content[..fixed_cells] == edited.content[..fixed_cells],
            "paste/edit disturbed the {mode} scene"
        );
        assert_eq!(view.input, " thought and a pasted continuatio");
    }
}

#[test]
fn quiet_ambient_keeps_every_glyph_fixed_and_changes_color_gradually() {
    for mode in Mode::ALL {
        let mut view = fixture(mode);
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
