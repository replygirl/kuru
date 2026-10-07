//! Translate Crossterm's indexed spelling of named colours for a classic ANSI terminal.
//!
//! Crossterm 0.29 emits even its named 16 colours as `38;5;n` / `48;5;n`.
//! A terminal advertising only classic colours need not understand that form.

use std::io::{self, Write};

pub(super) struct Writer<W> {
    inner: W,
    classic: bool,
    pending: Vec<u8>,
    scratch: Vec<u8>,
}

impl<W: Write> Writer<W> {
    pub(super) fn new(inner: W, classic: bool) -> Self {
        Self {
            inner,
            classic,
            pending: Vec::new(),
            scratch: Vec::new(),
        }
    }
}

impl<W: Write> Write for Writer<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.classic {
            return self.inner.write(bytes);
        }
        self.scratch.clear();
        let output = &mut self.scratch;
        for &byte in bytes {
            if self.pending.is_empty() {
                if byte == b'\x1b' {
                    self.pending.push(byte);
                } else {
                    output.push(byte);
                }
                continue;
            }
            self.pending.push(byte);
            if self.pending.len() == 2 {
                if byte != b'[' {
                    output.append(&mut self.pending);
                }
                continue;
            }
            if (0x40..=0x7e).contains(&byte) || self.pending.len() >= 64 {
                if byte == b'm' {
                    if let Some(classic) = classic_sgr(&self.pending) {
                        output.extend(classic);
                    } else {
                        output.extend(self.pending.iter().copied());
                    }
                } else {
                    output.extend(self.pending.iter().copied());
                }
                self.pending.clear();
            }
        }
        self.inner.write_all(output)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.write_all(&self.pending)?;
        self.pending.clear();
        self.inner.flush()
    }
}

fn classic_sgr(sequence: &[u8]) -> Option<Vec<u8>> {
    let parameters =
        std::str::from_utf8(sequence.strip_prefix(b"\x1b[")?.strip_suffix(b"m")?).ok()?;
    let fields = parameters.split(';').collect::<Vec<_>>();
    let mut rewritten = Vec::with_capacity(fields.len());
    let mut changed = false;
    let mut index = 0;
    while index < fields.len() {
        if matches!(fields[index], "38" | "48")
            && fields.get(index + 1) == Some(&"5")
            && let Some(value) = fields
                .get(index + 2)
                .and_then(|value| value.parse::<u8>().ok())
            && value < 16
        {
            let base = match (fields[index], value < 8) {
                ("38", true) => 30,
                ("38", false) => 90,
                ("48", true) => 40,
                _ => 100,
            };
            rewritten.push((base + value % 8).to_string());
            index += 3;
            changed = true;
        } else {
            // Extended colours are indivisible: RGB components can themselves
            // spell `38;5;n`, and must never be interpreted as a second colour.
            let count = if matches!(fields[index], "38" | "48") {
                match fields.get(index + 1) {
                    Some(&"2") => 5,
                    Some(&"5") => 3,
                    _ => 1,
                }
            } else {
                1
            };
            let end = (index + count).min(fields.len());
            rewritten.extend(fields[index..end].iter().map(|field| (*field).to_owned()));
            index = end;
        }
    }
    changed.then(|| format!("\x1b[{}m", rewritten.join(";")).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_combined_colours_become_classic_codes_without_changing_other_bytes() {
        let mut output = Vec::new();
        let mut writer = Writer::new(&mut output, true);
        writer.write_all(b"plain\x1b[38;5;").unwrap();
        writer
            .write_all(b"7;48;5;12mwide\x1b[0m\x1b[38;5;200m")
            .unwrap();
        writer.flush().unwrap();
        assert_eq!(output, b"plain\x1b[37;104mwide\x1b[0m\x1b[38;5;200m");
    }

    #[test]
    fn pinned_backend_named_colours_use_classic_sgr_and_rgb_groups_stay_intact() {
        use crossterm::style::{Color, SetBackgroundColor, SetForegroundColor};
        let mut bytes = Vec::new();
        let mut writer = Writer::new(&mut bytes, true);
        crossterm::queue!(
            writer,
            SetForegroundColor(Color::White),
            SetBackgroundColor(Color::DarkBlue)
        )
        .unwrap();
        writer.write_all(b"\x1b[38;2;38;5;7;48;5;12m").unwrap();
        writer.flush().unwrap();
        // Crossterm caches the runner's NO_COLOR setting. Do not change it:
        // the real PTY fixture separately clears its child environment and
        // proves named backend colours reach the classic wire encoding.
        let suppressed = SetForegroundColor(Color::White).to_string() == "\x1b[m";
        let expected: &[u8] = if suppressed {
            b"\x1b[m\x1b[m\x1b[38;2;38;5;7;104m"
        } else {
            b"\x1b[97m\x1b[44m\x1b[38;2;38;5;7;104m"
        };
        assert_eq!(bytes, expected);
    }

    #[test]
    fn non_classic_mode_preserves_short_writes_and_raw_bytes() {
        struct Short(Vec<u8>);
        impl Write for Short {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.push(bytes[0]);
                Ok(1)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Writer::new(Short(Vec::new()), false);
        assert_eq!(writer.write(b"\x1b[38;5;7m").unwrap(), 1);
        assert_eq!(writer.inner.0, b"\x1b");

        let mut classic = Writer::new(Short(Vec::new()), true);
        classic.write_all(b"\x1b[38;5;7m").unwrap();
        assert_eq!(classic.inner.0, b"\x1b[37m");

        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(
            Writer::new(Broken, true)
                .write(b"\x1b[38;5;7m")
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
