//! One resolved presentation palette per interactive invocation.

use kuru_core::{UiConfig, UiThemeName};
use ratatui::style::{Color, Style};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Depth {
    None,
    Ansi16,
    Ansi256,
    #[default]
    Rgb,
}

impl Depth {
    pub(super) fn from_environment() -> Self {
        if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
            return Self::None;
        }
        Self::detect(
            std::env::var("TERM").ok().as_deref(),
            std::env::var("COLORTERM").ok().as_deref(),
            None,
        )
    }
    pub(super) fn detect(
        term: Option<&str>,
        colorterm: Option<&str>,
        no_color: Option<&str>,
    ) -> Self {
        if no_color.is_some_and(|value| !value.is_empty()) {
            return Self::None;
        }
        if cfg!(windows) && term.is_none_or(|term| term.is_empty()) {
            // TerminalSession has already enabled native virtual-terminal output.
            return Self::Rgb;
        }
        let Some(term) =
            term.filter(|term| !term.is_empty() && *term != "dumb" && !term.starts_with("vt100"))
        else {
            return Self::None;
        };
        if !["xterm", "screen", "tmux", "ansi", "linux"]
            .iter()
            .any(|prefix| term.starts_with(prefix))
        {
            return Self::None;
        }
        if colorterm.is_some_and(|value| matches!(value, "truecolor" | "24bit")) {
            Self::Rgb
        } else if term.contains("256color") {
            Self::Ansi256
        } else if ["xterm", "screen", "tmux", "ansi", "linux"]
            .iter()
            .any(|prefix| term.starts_with(prefix))
        {
            Self::Ansi16
        } else {
            Self::None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum Role {
    Background,
    Surface,
    Raised,
    Border,
    Text,
    Muted,
    Accent,
    Secondary,
    Warning,
    Info,
    Error,
    SceneGhost,
    SceneTrace,
}

const ROLE_NAMES: [&str; 13] = [
    "background",
    "surface",
    "raised",
    "border",
    "text",
    "muted",
    "accent",
    "secondary",
    "warning",
    "info",
    "error",
    "scene_ghost",
    "scene_trace",
];

#[derive(Debug, Clone)]
pub(super) struct Theme {
    depth: Depth,
    colors: [Option<Color>; ROLE_NAMES.len()],
}

impl Theme {
    pub(super) fn new(config: &UiConfig, depth: Depth) -> Self {
        let rgb = match config.theme {
            UiThemeName::Dark => DARK,
            UiThemeName::Light => LIGHT,
        };
        let mut colors = match depth {
            Depth::None => [None; ROLE_NAMES.len()],
            Depth::Rgb => rgb.map(|[r, g, b]| Some(Color::Rgb(r, g, b))),
            Depth::Ansi256 => match config.theme {
                UiThemeName::Dark => DARK_256.map(|index| Some(Color::Indexed(index))),
                UiThemeName::Light => LIGHT_256.map(|index| Some(Color::Indexed(index))),
            },
            Depth::Ansi16 => match config.theme {
                UiThemeName::Dark => DARK_16.map(Some),
                UiThemeName::Light => LIGHT_16.map(Some),
            },
        };
        for (name, value) in &config.palette {
            let Some(index) = ROLE_NAMES.iter().position(|role| role == name) else {
                continue; // Config validation rejects this before presentation starts.
            };
            let Some(rgb) = parse_rgb(value) else {
                continue;
            };
            colors[index] = match depth {
                Depth::None => None,
                Depth::Rgb => Some(Color::Rgb(rgb[0], rgb[1], rgb[2])),
                Depth::Ansi256 => Some(Color::Indexed(nearest_256(rgb))),
                Depth::Ansi16 => Some(nearest_16(rgb)),
            };
        }
        Self { depth, colors }
    }

    pub(super) fn depth(&self) -> Depth {
        self.depth
    }

    pub(super) fn color(&self, role: Role) -> Option<Color> {
        self.colors[role as usize]
    }

    pub(super) fn foreground(&self, style: Style, role: Role) -> Style {
        self.color(role).map_or(style, |color| style.fg(color))
    }

    pub(super) fn background(&self, style: Style, role: Role) -> Style {
        self.color(role).map_or(style, |color| style.bg(color))
    }
}

/// Style a human CLI status only for its own terminal stream. Structured
/// output never calls this function and keeps its exact serialized bytes.
pub(crate) fn human_status(text: &str, config: &UiConfig, role: Role, terminal: bool) -> String {
    let depth = Depth::from_environment();
    format_human_status(text, config, role, depth, terminal)
}

fn format_human_status(
    text: &str,
    config: &UiConfig,
    role: Role,
    depth: Depth,
    terminal: bool,
) -> String {
    if !terminal {
        return text.to_owned();
    }
    let Some(color) = Theme::new(config, depth).color(role) else {
        return text.to_owned();
    };
    let prefix = match color {
        Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
        Color::Indexed(index) => format!("\x1b[38;5;{index}m"),
        Color::Black => "\x1b[30m".into(),
        Color::Red => "\x1b[31m".into(),
        Color::Green => "\x1b[32m".into(),
        Color::Yellow => "\x1b[33m".into(),
        Color::Blue => "\x1b[34m".into(),
        Color::Magenta => "\x1b[35m".into(),
        Color::Cyan => "\x1b[36m".into(),
        Color::Gray => "\x1b[37m".into(),
        Color::DarkGray => "\x1b[90m".into(),
        Color::LightRed => "\x1b[91m".into(),
        Color::LightGreen => "\x1b[92m".into(),
        Color::LightYellow => "\x1b[93m".into(),
        Color::LightBlue => "\x1b[94m".into(),
        Color::LightMagenta => "\x1b[95m".into(),
        Color::LightCyan => "\x1b[96m".into(),
        Color::White => "\x1b[97m".into(),
        _ => return text.to_owned(),
    };
    format!("{prefix}{text}\x1b[39m")
}

const DARK: [[u8; 3]; ROLE_NAMES.len()] = [
    [15, 19, 30],
    [21, 27, 42],
    [30, 38, 57],
    [57, 69, 92],
    [222, 231, 244],
    [145, 161, 184],
    [131, 231, 199],
    [193, 166, 247],
    [241, 200, 129],
    [135, 191, 250],
    [242, 149, 173],
    [43, 57, 76],
    [62, 81, 104],
];
const LIGHT: [[u8; 3]; ROLE_NAMES.len()] = [
    [248, 250, 252],
    [255, 255, 255],
    [233, 238, 245],
    [195, 204, 216],
    [21, 32, 51],
    [71, 85, 105],
    [15, 118, 110],
    [109, 40, 217],
    [146, 64, 14],
    [29, 78, 216],
    [185, 28, 28],
    [203, 213, 225],
    [148, 163, 184],
];
const DARK_256: [u8; ROLE_NAMES.len()] = [
    233, 234, 236, 239, 255, 246, 121, 183, 222, 117, 211, 238, 241,
];
const LIGHT_256: [u8; ROLE_NAMES.len()] =
    [255, 15, 254, 250, 233, 241, 29, 92, 94, 25, 124, 251, 247];
const DARK_16: [Color; ROLE_NAMES.len()] = [
    Color::Black,
    Color::Black,
    Color::DarkGray,
    Color::Gray,
    Color::White,
    Color::Gray,
    Color::LightGreen,
    Color::LightMagenta,
    Color::Yellow,
    Color::LightBlue,
    Color::LightRed,
    Color::DarkGray,
    Color::Gray,
];
const LIGHT_16: [Color; ROLE_NAMES.len()] = [
    Color::White,
    Color::White,
    Color::Gray,
    Color::DarkGray,
    Color::Black,
    Color::DarkGray,
    Color::Green,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Red,
    Color::Gray,
    Color::DarkGray,
];

fn parse_rgb(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#')?;
    Some([
        u8::from_str_radix(hex.get(0..2)?, 16).ok()?,
        u8::from_str_radix(hex.get(2..4)?, 16).ok()?,
        u8::from_str_radix(hex.get(4..6)?, 16).ok()?,
    ])
}

fn nearest_256(rgb: [u8; 3]) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    (16..=255u8)
        .min_by_key(|&index| {
            let candidate = if index < 232 {
                let cube = usize::from(index - 16);
                [LEVELS[cube / 36], LEVELS[cube / 6 % 6], LEVELS[cube % 6]]
            } else {
                [8 + 10 * (index - 232); 3]
            };
            candidate
                .into_iter()
                .zip(rgb)
                .map(|(a, b)| u32::from(a.abs_diff(b)).pow(2))
                .sum::<u32>()
        })
        .expect("fixed nonempty xterm palette")
}

fn nearest_16(rgb: [u8; 3]) -> Color {
    const CANDIDATES: [([u8; 3], Color); 16] = [
        ([0, 0, 0], Color::Black),
        ([128, 0, 0], Color::Red),
        ([0, 128, 0], Color::Green),
        ([128, 128, 0], Color::Yellow),
        ([0, 0, 128], Color::Blue),
        ([128, 0, 128], Color::Magenta),
        ([0, 128, 128], Color::Cyan),
        ([192, 192, 192], Color::Gray),
        ([128, 128, 128], Color::DarkGray),
        ([255, 0, 0], Color::LightRed),
        ([0, 255, 0], Color::LightGreen),
        ([255, 255, 0], Color::LightYellow),
        ([0, 0, 255], Color::LightBlue),
        ([255, 0, 255], Color::LightMagenta),
        ([0, 255, 255], Color::LightCyan),
        ([255, 255, 255], Color::White),
    ];
    CANDIDATES
        .into_iter()
        .min_by_key(|(candidate, _)| {
            candidate
                .iter()
                .copied()
                .zip(rgb)
                .map(|(a, b)| u32::from(a.abs_diff(b)).pow(2))
                .sum::<u32>()
        })
        .expect("fixed nonempty ANSI palette")
        .1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonempty_no_color_precedes_theme_and_terminal_depth() {
        assert_eq!(
            Depth::detect(Some("xterm-256color"), Some("truecolor"), Some("1")),
            Depth::None
        );
        assert_eq!(
            Depth::detect(Some("xterm-256color"), Some("truecolor"), Some("")),
            Depth::Rgb
        );
        assert_eq!(
            Depth::detect(Some("xterm-256color"), None, None),
            Depth::Ansi256
        );
        assert_eq!(Depth::detect(Some("xterm"), None, None), Depth::Ansi16);
        assert_eq!(
            Depth::detect(Some("dumb"), Some("truecolor"), None),
            Depth::None
        );
        assert_eq!(Depth::detect(Some("vt100"), None, None), Depth::None);
        assert_eq!(
            Depth::detect(Some("unknown-terminal"), Some("truecolor"), None),
            Depth::None
        );
        assert_eq!(
            Depth::detect(Some("vt100"), Some("truecolor"), None),
            Depth::None
        );
    }

    #[test]
    fn semantic_palettes_keep_shipped_status_distinctions_at_each_depth() {
        for name in [UiThemeName::Dark, UiThemeName::Light] {
            let config = UiConfig {
                theme: name,
                ..UiConfig::default()
            };
            for depth in [Depth::Rgb, Depth::Ansi256, Depth::Ansi16] {
                let palette = Theme::new(&config, depth);
                let ordinary = palette.color(Role::Text).unwrap();
                let background = palette.color(Role::Background).unwrap();
                let status = [Role::Accent, Role::Warning, Role::Error, Role::Info]
                    .map(|role| palette.color(role).unwrap());
                assert_ne!(ordinary, background, "{name:?} {depth:?}");
                assert!(status.iter().all(|color| *color != background));
                assert_ne!(status[1], status[2], "warning and error collapsed");
            }
        }
        let none = Theme::new(&UiConfig::default(), Depth::None);
        assert_eq!(
            none.foreground(Style::default(), Role::Error),
            Style::default()
        );
        assert_eq!(
            none.background(Style::default(), Role::Raised),
            Style::default()
        );
    }

    #[test]
    fn validated_override_changes_only_its_named_role() {
        let mut config = UiConfig::default();
        config.palette.insert("error".into(), "#123456".into());
        for depth in [Depth::Rgb, Depth::Ansi256, Depth::Ansi16] {
            let baseline = Theme::new(&UiConfig::default(), depth);
            let changed = Theme::new(&config, depth);
            assert_ne!(changed.color(Role::Error), baseline.color(Role::Error));
            assert_eq!(changed.color(Role::Text), baseline.color(Role::Text));
            assert_eq!(changed.color(Role::Warning), baseline.color(Role::Warning));
        }
        assert_eq!(
            Theme::new(&config, Depth::Rgb).color(Role::Error),
            Some(Color::Rgb(0x12, 0x34, 0x56))
        );
    }

    #[test]
    fn human_status_styles_only_a_capable_human_stream() {
        let config = UiConfig::default();
        let plain = "MCP local: unavailable";
        assert_eq!(
            format_human_status(plain, &config, Role::Warning, Depth::Rgb, false),
            plain
        );
        assert_eq!(
            format_human_status(plain, &config, Role::Warning, Depth::None, true),
            plain
        );
        assert_eq!(
            format_human_status(plain, &config, Role::Warning, Depth::Ansi16, true),
            format!("\x1b[33m{plain}\x1b[39m")
        );
        assert!(
            format_human_status(plain, &config, Role::Warning, Depth::Ansi256, true)
                .starts_with("\x1b[38;5;")
        );
    }
}
