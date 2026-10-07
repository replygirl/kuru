//! Fixed terminal metadata, independent of prompts, answers and actor activity.

use std::io::{self, Write};

#[derive(Default)]
pub(super) struct SavedTitle {
    saved: bool,
}

impl SavedTitle {
    pub(super) fn save(&mut self, output: &mut impl Write, supported: bool) -> io::Result<()> {
        if supported {
            output.write_all(b"\x1b[22;2t")?;
            // Complete escape accepted by the writer: a failed flush must still
            // leave restoration owned. A partial escape never changes a title.
            self.saved = true;
            output.flush()?;
        }
        Ok(())
    }

    pub(super) fn supported(&self) -> bool {
        self.saved
    }

    pub(super) fn restore(&mut self, output: &mut impl Write) -> io::Result<()> {
        if self.saved {
            output.write_all(b"\x1b[23;2t")?;
            // Do not pop a second title if another guard restoration fails.
            self.saved = false;
            output.flush()?;
        }
        Ok(())
    }
}

pub(super) fn title_stack_supported(term: Option<&str>) -> bool {
    cfg!(unix) && matches!(term, Some("xterm" | "xterm-256color"))
}

#[derive(Default)]
pub(super) struct Signals {
    pub(super) titles: bool,
    pub(super) completion: bool,
    title: String,
    pub(super) color_depth: super::theme::Depth,
}

impl Signals {
    pub(super) fn new(titles: bool, completion: bool) -> Self {
        Self {
            titles,
            completion,
            ..Self::default()
        }
    }

    pub(super) fn with_color_depth(mut self, depth: super::theme::Depth) -> Self {
        self.color_depth = depth;
        self
    }

    pub(super) fn present(
        &mut self,
        output: &mut impl Write,
        session: &str,
        completion: bool,
    ) -> io::Result<()> {
        if !self.titles && !self.completion {
            return Ok(());
        }
        if self.titles {
            let suffix: String = session
                .chars()
                .filter(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                })
                .take(32)
                .collect();
            let title = format!("Kuru | {suffix}");
            if self.title != title {
                write!(output, "\x1b]2;{title}\x1b\\")?;
                self.title = title;
            }
        }
        if self.completion && completion {
            output.write_all(b"\x07")?;
        }
        output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_metadata_pairs_title_stack_and_never_contains_content_or_escape_suffixes() {
        let mut bytes = Vec::new();
        let mut saved = SavedTitle::default();
        saved.save(&mut bytes, true).unwrap();
        let mut signals = Signals::new(true, true);
        signals
            .present(&mut bytes, "session\u{1b}]2;foreign\u{7}秘密", false)
            .unwrap();
        signals
            .present(&mut bytes, "session\u{1b}]2;foreign\u{7}秘密", true)
            .unwrap();
        saved.restore(&mut bytes).unwrap();
        saved.restore(&mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(
            text,
            "\u{1b}[22;2t\u{1b}]2;Kuru | session2foreign\u{1b}\\\u{7}\u{1b}[23;2t"
        );
        assert!(!saved.supported());
        let mut unsupported = Vec::new();
        saved.save(&mut unsupported, false).unwrap();
        Signals::default()
            .present(&mut unsupported, "private answer", true)
            .unwrap();
        saved.restore(&mut unsupported).unwrap();
        assert!(unsupported.is_empty());
        assert!(!title_stack_supported(Some("unknown-terminal")));
    }
}
