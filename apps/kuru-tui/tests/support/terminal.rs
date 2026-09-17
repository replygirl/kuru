use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    process::Command,
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use rustix::process::{Pid as RustixPid, WaitId, WaitIdOptions, waitid};

const TICK: Duration = Duration::from_millis(20);
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);
type RestorationCheck = dyn Fn(&dyn MasterPty) -> Result<bool>;
const OUTPUT_QUEUE: usize = 8;
type OutputReceiver = mpsc::Receiver<std::io::Result<Vec<u8>>>;

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
) -> (OutputReceiver, mpsc::Receiver<()>, JoinHandle<()>) {
    let (send, receive) = mpsc::sync_channel(OUTPUT_QUEUE);
    let (done, reader_done) = mpsc::channel();
    let output = std::thread::spawn(move || {
        loop {
            let mut buffer = [0; 65_536];
            let message = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => Ok(buffer[..count].to_vec()),
                Err(error) => Err(error),
            };
            let failed = message.is_err();
            if send.send(message).is_err() || failed {
                break;
            }
        }
        let _ = done.send(());
    });
    (receive, reader_done, output)
}

fn close_reader(
    receive: &mut Option<OutputReceiver>,
    reader_done: &mut Option<mpsc::Receiver<()>>,
    reader: &mut Option<JoinHandle<()>>,
    timeout: Duration,
) -> Result<()> {
    drop(receive.take());
    let finished = reader_done.as_ref().is_none_or(|done| {
        matches!(
            done.recv_timeout(timeout),
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected)
        )
    });
    ensure!(
        finished,
        "terminal reader cleanup remains unproven after {timeout:?}"
    );
    drop(reader_done.take());
    if let Some(handle) = reader.take() {
        ensure!(handle.join().is_ok(), "terminal reader thread panicked");
    }
    Ok(())
}

pub fn bounded_reader_cleanup_probe(timeout: Duration) -> Result<Duration> {
    let (reader, held_writer) = UnixStream::pair()?;
    let (receive, reader_done, reader) = spawn_reader(Box::new(reader));
    let mut receive = Some(receive);
    let mut reader_done = Some(reader_done);
    let mut reader = Some(reader);
    let started = Instant::now();
    let error = close_reader(&mut receive, &mut reader_done, &mut reader, timeout).unwrap_err();
    let elapsed = started.elapsed();
    ensure!(
        error
            .to_string()
            .contains("terminal reader cleanup remains unproven"),
        "{error}"
    );
    let error = close_reader(&mut receive, &mut reader_done, &mut reader, timeout).unwrap_err();
    ensure!(
        error
            .to_string()
            .contains("terminal reader cleanup remains unproven"),
        "{error}"
    );
    drop(held_writer);
    close_reader(
        &mut receive,
        &mut reader_done,
        &mut reader,
        Duration::from_secs(1),
    )?;
    Ok(elapsed)
}

pub fn startup_timeout(memory_startup: Duration) -> Duration {
    // A fresh store starts a staging server, stops/reaps it, then starts the
    // activated server before the first frame. Each startup allows two seconds
    // for its supervisor handshake; staged shutdown allows 8s graceful + 3s
    // forced reaping + 2s parent acknowledgment (kuru-memory's finish_owner).
    // Subsequent frame/input waits retain the shorter READY_TIMEOUT.
    (memory_startup + Duration::from_secs(2)) * 2 + Duration::from_secs(8 + 3 + 2) + READY_TIMEOUT
}

pub struct Terminal {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    receive: Option<OutputReceiver>,
    reader: Option<JoinHandle<()>>,
    reader_done: Option<mpsc::Receiver<()>>,
    restored: Box<RestorationCheck>,
    parser: vt100::Parser,
    pub output: Vec<u8>,
}

impl Terminal {
    pub fn spawn(command: Command, rows: u16, cols: u16) -> Result<Self> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let before = pair
            .master
            .get_termios()
            .context("terminal has no native attributes")?;
        let restored = Box::new(move |master: &dyn MasterPty| {
            Ok(master
                .get_termios()
                .context("terminal has no native attributes")?
                == before)
        });

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let mut builder = CommandBuilder::new(command.get_program());
        builder.args(command.get_args());
        if let Some(directory) = command.get_current_dir() {
            builder.cwd(directory);
        }
        for (name, value) in command.get_envs() {
            if let Some(value) = value {
                builder.env(name, value);
            } else {
                builder.env_remove(name);
            }
        }
        // portable-pty owns the audited setsid/TIOCSCTTY boundary. The child
        // must use this slave as both stdio and its controlling terminal so
        // Crossterm cannot read dimensions from the invoking test runner.
        let child = pair
            .slave
            .spawn_command(builder)
            .context("spawn terminal child")?;
        drop(pair.slave);
        // Start the pump only after spawn succeeds. A bounded queue applies
        // backpressure without allowing an unbounded collection of owned Vecs.
        let (receive, reader_done, output) = spawn_reader(reader);
        Ok(Self {
            child,
            master: Some(pair.master),
            writer: Some(writer),
            receive: Some(receive),
            reader: Some(output),
            reader_done: Some(reader_done),
            restored,
            parser: vt100::Parser::new(rows, cols, 0),
            output: Vec::new(),
        })
    }

    pub fn read_for(&mut self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match self
                .receive
                .as_ref()
                .context("terminal output is closed")?
                .recv_timeout(remaining.min(TICK))
            {
                Ok(Ok(bytes)) => {
                    self.parser.process(&bytes);
                    self.output.extend(bytes);
                }
                Ok(Err(_error)) if self.child.try_wait()?.is_some() => break,
                Ok(Err(error)) => return Err(error.into()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    }

    pub fn screen(&self) -> String {
        self.parser.screen().contents()
    }

    fn diagnostics(&self) -> String {
        let tail = &self.output[self.output.len().saturating_sub(512)..];
        let escaped: Vec<u8> = tail
            .iter()
            .flat_map(|byte| std::ascii::escape_default(*byte))
            .collect();
        format!(
            "{} PTY bytes; raw tail: {}\n{}",
            self.output.len(),
            String::from_utf8_lossy(&escaped),
            self.screen()
        )
    }

    fn child_id(&self) -> Result<u32> {
        self.child
            .process_id()
            .context("terminal child has no process identifier")
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
                bail!(
                    "{description}: process exited {status:?}\n{}",
                    self.diagnostics()
                );
            }
            ensure!(
                Instant::now() < deadline,
                "{description}: timed out after {timeout:?}; process {} is still running\n{}",
                self.child_id()?,
                self.diagnostics()
            );
        }
    }

    pub fn wait_text(&mut self, present: &[&str], absent: &[&str]) -> Result<()> {
        self.wait_text_with_timeout(present, absent, READY_TIMEOUT)
    }

    pub fn wait_text_with_timeout(
        &mut self,
        present: &[&str],
        absent: &[&str],
        timeout: Duration,
    ) -> Result<()> {
        self.wait(
            &format!("screen contains {present:?}, excludes {absent:?}"),
            timeout,
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
        let writer = self.writer.as_mut().context("terminal input is closed")?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    #[allow(dead_code)] // Used by trust.rs; each integration test compiles this support module alone.
    pub fn interrupt(&mut self) -> Result<()> {
        let pid = Pid::from_raw(self.child_id()?.try_into()?);
        kill(pid, Signal::SIGINT)?;
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
                let rows = screen.lines().collect::<Vec<_>>();
                // The dock grows as controls are added. Find the rendered
                // separator instead of assuming a fixed number of bottom rows.
                let separator = rows
                    .iter()
                    .rposition(|row| row.trim_start().starts_with(".    ."));
                Ok(separator.is_some_and(|row| {
                    rows.get(row + 1..rows.len().saturating_sub(1))
                        .is_some_and(|composer| composer.join("\n").contains(text))
                }))
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
        self.master
            .as_ref()
            .context("terminal is closed")?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        self.parser.screen_mut().set_size(rows, cols);
        Ok(())
    }

    pub fn pause_for(&self, duration: Duration) -> Result<Resume> {
        let pid = Pid::from_raw(self.child_id()?.try_into()?);
        kill(pid, Signal::SIGSTOP)?;
        Ok(Resume(Some(std::thread::spawn(move || {
            std::thread::sleep(duration);
            let _ = kill(pid, Signal::SIGCONT);
        }))))
    }

    pub fn resume(&self) -> Result<()> {
        let pid = Pid::from_raw(self.child_id()?.try_into()?);
        kill(pid, Signal::SIGCONT)?;
        Ok(())
    }

    pub fn wait_stopped(&self, timeout: Duration) -> Result<()> {
        let raw_pid = self.child_id()?.try_into()?;
        let pid =
            RustixPid::from_raw(raw_pid).context("terminal child has an invalid process ID")?;
        let deadline = Instant::now() + timeout;
        loop {
            match waitid(
                WaitId::Pid(pid),
                WaitIdOptions::STOPPED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )? {
                Some(status) if status.stopped() => return Ok(()),
                Some(status) => bail!("terminal child {pid} entered unexpected state {status:?}"),
                None => {}
            }
            ensure!(
                Instant::now() < deadline,
                "terminal child {pid} did not enter stopped state within {timeout:?}"
            );
            std::thread::sleep(TICK);
        }
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> Result<()> {
        // The final redraw can exceed the PTY buffer. Keep reading until exit.
        let deadline = Instant::now() + timeout;
        let status = loop {
            self.read_for(TICK)?;
            if let Some(status) = self.child.try_wait()? {
                break status;
            }
            ensure!(
                Instant::now() < deadline,
                "process exits: timed out after {timeout:?}; process {} is still running\n{}",
                self.child_id()?,
                self.diagnostics()
            );
        };
        // Process exit closes the final slave descriptor. Drain until the
        // reader observes that close so no queued final frame or large payload
        // is lost merely because waitpid won the race.
        let drain_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match self
                .receive
                .as_ref()
                .context("terminal output is closed")?
                .recv_timeout(TICK)
            {
                Ok(Ok(bytes)) => {
                    self.parser.process(&bytes);
                    self.output.extend(bytes);
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    ensure!(
                        Instant::now() < drain_deadline,
                        "PTY output did not close after process exit\n{}",
                        self.diagnostics()
                    );
                }
            }
        }
        ensure!(
            status.success(),
            "child failed: {status:?}\n{}",
            self.diagnostics()
        );
        Ok(())
    }

    pub fn assert_restored(&self) -> Result<()> {
        let master = self.master.as_deref().context("terminal is closed")?;
        ensure!((self.restored)(master)?, "terminal attributes not restored");
        Ok(())
    }

    pub fn close(&mut self, timeout: Duration) -> Result<()> {
        let mut issue = None;
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(error) = self.child.kill() {
                    issue = Some(format!("failed to stop terminal child: {error}"));
                }
                if let Err(error) = self.child.wait() {
                    issue.get_or_insert_with(|| format!("failed to reap terminal child: {error}"));
                }
            }
            Err(error) => {
                issue = Some(format!("failed to inspect terminal child: {error}"));
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }

        // Disconnect a sender blocked on the bounded queue before waiting. A
        // reader blocked in the kernel retains its own handle until the last
        // inherited slave closes.
        let reader = close_reader(
            &mut self.receive,
            &mut self.reader_done,
            &mut self.reader,
            timeout,
        );
        drop(self.writer.take());
        drop(self.master.take());
        if let Err(error) = reader {
            issue.get_or_insert_with(|| error.to_string());
        }
        if let Some(issue) = issue {
            bail!(issue);
        }
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Err(error) = self.close(Duration::from_secs(1)) {
            eprintln!("{error:#}");
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
