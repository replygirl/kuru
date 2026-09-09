use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsFd,
    process::{Child, Command, Stdio},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use nix::{
    fcntl::{FcntlArg, FdFlag, OFlag, fcntl},
    poll::{PollFd, PollFlags, poll},
    pty::{Winsize, openpty},
    sys::{
        signal::{Signal, kill},
        termios::{Termios, tcgetattr},
    },
    unistd::Pid,
};

const TICK: Duration = Duration::from_millis(20);
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Terminal {
    child: Child,
    master: File,
    slave: File,
    before: Termios,
    parser: vt100::Parser,
    pub output: Vec<u8>,
}

impl Terminal {
    pub fn spawn(mut command: Command, rows: u16, cols: u16) -> Result<Self> {
        let pair = openpty(
            Some(&Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )?;
        // Only the explicit stdio duplicates belong in the child. Inheriting
        // master/slave originals can keep descriptors alive after shutdown.
        fcntl(&pair.master, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
        fcntl(&pair.slave, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
        let master = File::from(pair.master);
        let slave = File::from(pair.slave);
        let flags = OFlag::from_bits_truncate(fcntl(&master, FcntlArg::F_GETFL)?);
        fcntl(&master, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
        let before = tcgetattr(&slave)?;
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave.try_clone()?));
        let child = command.spawn().context("spawn terminal child")?;
        Ok(Self {
            child,
            master,
            slave,
            before,
            parser: vt100::Parser::new(rows, cols, 0),
            output: Vec::new(),
        })
    }

    pub fn read_for(&mut self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            let mut pollfds = [PollFd::new(self.master.as_fd(), PollFlags::POLLIN)];
            let timeout = remaining.min(TICK).as_millis().max(1) as u16;
            match poll(&mut pollfds, timeout) {
                Ok(0) => continue,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
            let mut buffer = [0; 65536];
            match self.master.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    self.parser.process(&buffer[..count]);
                    self.output.extend_from_slice(&buffer[..count]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    if self.child.try_wait()?.is_some() {
                        break;
                    }
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }

    pub fn screen(&self) -> String {
        self.parser.screen().contents()
    }

    pub fn wait(
        &mut self,
        description: &str,
        timeout: Duration,
        mut predicate: impl FnMut(&mut Self) -> Result<bool>,
    ) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.read_for(TICK)?;
            if predicate(self)? {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait()? {
                bail!("{description}: process exited {status}\n{}", self.screen());
            }
            ensure!(
                Instant::now() < deadline,
                "{description}: timed out\n{}",
                self.screen()
            );
        }
    }

    pub fn wait_text(&mut self, present: &[&str], absent: &[&str]) -> Result<()> {
        self.wait(
            &format!("screen contains {present:?}, excludes {absent:?}"),
            READY_TIMEOUT,
            |terminal| {
                let screen = terminal.screen();
                Ok(present.iter().all(|value| screen.contains(value))
                    && !absent.iter().any(|value| screen.contains(value)))
            },
        )
    }

    pub fn wait_idle(&mut self) -> Result<()> {
        self.wait_text(&["enter send"], &[])
    }

    pub fn wait_composer_frame(&mut self, present: &[&str], timeout: Duration) -> Result<()> {
        self.wait(
            &format!("completed composer frame contains {present:?}"),
            timeout,
            |terminal| {
                let screen = terminal.screen();
                // Ratatui's Crossterm backend flushes the cell diff and Show
                // before separately flushing MoveTo. Visible draft text (or
                // even the resulting cursor position) can therefore precede
                // the final bytes. Observe the complete wire trailer instead.
                let (row, col) = terminal.parser.screen().cursor_position();
                let trailer = format!(
                    "\x1b[?25h\x1b[{};{}H",
                    u32::from(row) + 1,
                    u32::from(col) + 1
                );
                Ok(present.iter().all(|value| screen.contains(value))
                    && terminal.output.ends_with(trailer.as_bytes()))
            },
        )
    }

    pub fn assert_no_output_since(&self, settled: usize, description: &str) -> Result<()> {
        ensure!(
            self.output.len() == settled,
            "{description}: {} new bytes after completed frame\n{}",
            self.output.len() - settled,
            self.screen()
        );
        Ok(())
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.master.write_all(bytes)?;
        Ok(())
    }

    pub fn command(&mut self, text: &str, picker: Option<&str>) -> Result<()> {
        self.wait_idle()?;
        self.send(text.as_bytes())?;
        // A stale idle frame cannot acknowledge a command: first observe the
        // entered draft in the composer, then observe it clearing after Enter.
        self.wait(
            &format!("draft contains {text:?}"),
            READY_TIMEOUT,
            |terminal| {
                let screen = terminal.screen();
                let composer: String = screen.lines().rev().take(5).collect();
                Ok(composer.contains(text))
            },
        )?;
        self.send(b"\r")?;
        if let Some(picker) = picker {
            self.wait_text(&[picker, "Esc back"], &[])
        } else {
            self.wait_text(&["What shall we explore or build?", "enter send"], &[])
        }
    }

    pub fn close_picker(&mut self, keys: &[u8]) -> Result<()> {
        self.send(keys)?;
        self.wait_text(&["enter send"], &["Esc back"])
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        rustix::termios::tcsetwinsize(
            &self.slave,
            rustix::termios::Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        self.parser.screen_mut().set_size(rows, cols);
        Ok(())
    }

    pub fn pause_for(&self, duration: Duration) -> Result<Resume> {
        let pid = Pid::from_raw(self.child.id().try_into()?);
        kill(pid, Signal::SIGSTOP)?;
        Ok(Resume(Some(std::thread::spawn(move || {
            std::thread::sleep(duration);
            let _ = kill(pid, Signal::SIGCONT);
        }))))
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> Result<()> {
        // The final redraw can exceed the PTY buffer. Keep reading until exit.
        self.wait("process exits", timeout, |terminal| {
            Ok(terminal.child.try_wait()?.is_some())
        })?;
        self.read_for(TICK)?;
        let status = self.child.try_wait()?.context("child has not exited")?;
        ensure!(
            status.success(),
            "child failed: {status}\n{}",
            self.screen()
        );
        Ok(())
    }

    pub fn assert_restored(&self) -> Result<()> {
        ensure!(
            tcgetattr(&self.slave)? == self.before,
            "terminal attributes not restored"
        );
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub struct Resume(Option<JoinHandle<()>>);

impl Drop for Resume {
    fn drop(&mut self) {
        if let Some(thread) = self.0.take() {
            let _ = thread.join();
        }
    }
}
