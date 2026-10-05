use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    process::Command,
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use kuru_memory::test_budgets::OPERATION_TIMEOUT;
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use rustix::process::{Pid as RustixPid, WaitId, WaitIdOptions, waitid};

const TICK: Duration = Duration::from_millis(20);
/// The longest a due frame trails the event that makes it due: the idle
/// ambient interval (250 ms, `View::advance_animation` in src/ui.rs; "capped
/// at 4 FPS" in docs/interface.md) plus the idle animation wake (100 ms, the
/// `Wake::Animation` arm of the UI loop). Input, completion and activity wakes
/// mark the view dirty and draw on the next loop pass.
pub const FRAME_ALLOWANCE: Duration = Duration::from_millis(250 + 100);
/// Default bound for a frame or input wait. The PTY child opens memory on the
/// managed Remote backend (src/cli.rs `open_memory`), so the frame after a
/// memory command (`/memory-candidates`, compact, abandon, export, fork) or
/// after the first-run notice record (`notice.record()` in the UI loop) waits
/// on one Remote reply, bounded by the client's reply deadline
/// `OPERATION_TIMEOUT` (kuru-memory src/service/rpc.rs:38, applied by
/// `ReplyBudget::deadline` and `exchange_attached_with_id`). That reply is the
/// longest product step a frame wait encloses, and its frame trails it by at
/// most `FRAME_ALLOWANCE`. Every use is event-driven, so a passing run is
/// unchanged and only a real hang takes longer to report. `startup_timeout`
/// adds this bound, and trust.rs follows through this module.
pub const READY_TIMEOUT: Duration = OPERATION_TIMEOUT.saturating_add(FRAME_ALLOWANCE);
const _: () = assert!(READY_TIMEOUT.as_nanos() > OPERATION_TIMEOUT.as_nanos());
/// kuru-connectors `IO_TIMEOUT` (60 s, `pub(crate)` at src/lib.rs:74): the
/// per-request bound of `http::client()` (src/http.rs:9) and of the MCP host's
/// shutdown join (`McpHosts::shutdown`, src/mcp.rs:1203). Documented in
/// docs/protocols.md (model catalog requests "bounded to 60 seconds", MCP calls
/// "bounded to 60 seconds"). Restated once here because it is not public.
pub const IO_TIMEOUT: Duration = Duration::from_secs(60);
type RestorationCheck = dyn Fn(&dyn MasterPty) -> Result<bool>;
const OUTPUT_QUEUE: usize = 8;
// Escaped PTY output shown whole in an unexpected-end report, and the bound on
// draining output still queued at an observed exit until the reader closes.
const REPORT_OUTPUT_LIMIT: usize = 16 * 1024;
const LATE_OUTPUT_WINDOW: Duration = Duration::from_millis(500);
type OutputReceiver = mpsc::Receiver<std::io::Result<Vec<u8>>>;

fn drain_exit_output(
    receive: &OutputReceiver,
    parser: &mut vt100::Parser,
    output: &mut Vec<u8>,
    deadline: Instant,
) -> Result<()> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "PTY output drain deadline expired");
        let message = receive.recv_timeout(remaining.min(TICK));
        // A receive can finish after its requested wait if this thread was
        // descheduled. Bytes and EOF/EIO must obey the same deadline as a
        // timeout; the next iteration also bounds processing of queued data.
        ensure!(
            Instant::now() < deadline,
            "PTY output drain deadline expired"
        );
        match message {
            Ok(Ok(bytes)) => {
                parser.process(&bytes);
                output.extend(bytes);
            }
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

pub fn exit_drain_deadline_probe() -> Result<()> {
    for eio in [false, true] {
        let queued = || -> Result<OutputReceiver> {
            let (send, receive) = mpsc::sync_channel(OUTPUT_QUEUE);
            send.send(Ok(b"QUEUED\r\n".to_vec()))?;
            send.send(Ok(b"FINAL".to_vec()))?;
            if eio {
                send.send(Err(std::io::Error::from_raw_os_error(nix::libc::EIO)))?;
            }
            // With no EIO message, dropping the sender is the EOF/disconnect.
            drop(send);
            Ok(receive)
        };
        let receive = queued()?;
        let mut parser = vt100::Parser::new(2, 20, 0);
        let mut output = Vec::new();
        // The complete queued sequence exists before the already-expired
        // deadline is supplied. No scheduler delay or positive sleep decides
        // whether bytes/completion can wrongly win over expiry.
        let error = drain_exit_output(&receive, &mut parser, &mut output, Instant::now())
            .expect_err("expired drain accepted queued output and completion");
        ensure!(error.to_string().contains("deadline expired"), "{error}");
        ensure!(output.is_empty(), "expired drain consumed queued output");

        let receive = queued()?;
        drain_exit_output(
            &receive,
            &mut parser,
            &mut output,
            Instant::now() + READY_TIMEOUT,
        )?;
        ensure!(
            output == b"QUEUED\r\nFINAL",
            "timely drain lost byte ordering"
        );
        ensure!(
            parser.screen().contents().contains("FINAL"),
            "timely drain did not update the terminal screen"
        );
    }
    Ok(())
}

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
    // Subsequent frame/input waits retain the shorter READY_TIMEOUT; a
    // submitted dream's settle reuses this budget (`wait_dream_settled`).
    (memory_startup + Duration::from_secs(2)) * 2 + Duration::from_secs(8 + 3 + 2) + READY_TIMEOUT
}

fn escaped(bytes: &[u8], limit: usize) -> String {
    let shown: Vec<u8> = bytes[..bytes.len().min(limit)]
        .iter()
        .flat_map(|byte| std::ascii::escape_default(*byte))
        .collect();
    let mut text = String::from_utf8_lossy(&shown).into_owned();
    if bytes.len() > limit {
        text.push_str(&format!(" ... {} more bytes omitted", bytes.len() - limit));
    }
    text
}

pub struct Terminal {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    // Program, arguments, directory and coverage-profile presence at spawn, for
    // reports. Environment values are omitted: they can carry synthetic keys.
    launch: String,
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
        // The child's own detached spawns name this test in a coverage
        // partition's spawn rows.
        for (name, value) in kuru_memory::test_support::spawn_ledger::forwarded() {
            builder.env(name, value);
        }
        let launch = format!(
            "program={:?} arguments={:?} directory={:?} LLVM_PROFILE_FILE={}",
            command.get_program(),
            command.get_args().collect::<Vec<_>>(),
            command.get_current_dir(),
            if builder.get_env("LLVM_PROFILE_FILE").is_some() {
                "present"
            } else {
                "absent"
            }
        );
        // portable-pty owns the audited setsid/TIOCSCTTY boundary. The child
        // must use this slave as both stdio and its controlling terminal so
        // Crossterm cannot read dimensions from the invoking test runner.
        let child = pair
            .slave
            .spawn_command(builder)
            .context("spawn terminal child")?;
        // The child leads its own session, beyond the coverage runner's
        // group cleanup: record it so a partition names this test if the
        // child's profile appears after the tests.
        if let Some(pid) = child.process_id() {
            kuru_memory::test_support::spawn_ledger::record(
                pid,
                std::path::Path::new(command.get_program()),
                kuru_memory::test_support::spawn_ledger::TERMINAL_CHILD,
            );
        }
        drop(pair.slave);
        // Start the pump only after spawn succeeds. A bounded queue applies
        // backpressure without allowing an unbounded collection of owned Vecs.
        let (receive, reader_done, output) = spawn_reader(reader);
        Ok(Self {
            child,
            launch,
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

    fn receive_output(
        &self,
        timeout: Duration,
    ) -> Result<std::result::Result<std::io::Result<Vec<u8>>, mpsc::RecvTimeoutError>> {
        Ok(self
            .receive
            .as_ref()
            .context("terminal output is closed")?
            .recv_timeout(timeout))
    }

    pub fn read_for(&mut self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match self.receive_output(remaining.min(TICK))? {
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

    #[allow(dead_code)] // Used by cli.rs; each integration target compiles this support module alone.
    pub fn completed_frame_after(&self, previous_output_len: usize) -> bool {
        let Some(output) = self.output.get(previous_output_len..) else {
            return false;
        };
        let show = b"\x1b[?25h";
        output
            .windows(show.len())
            .enumerate()
            .any(|(index, window)| {
                if window != show {
                    return false;
                }
                let Some(cursor) = output.get(index + show.len()..) else {
                    return false;
                };
                if !cursor.starts_with(b"\x1b[") {
                    return false;
                }
                let cursor = &cursor[2..];
                let Some(end) = cursor.iter().position(|byte| *byte == b'H') else {
                    return false;
                };
                let position = &cursor[..end];
                let Some(separator) = position.iter().position(|byte| *byte == b';') else {
                    return false;
                };
                !position[..separator].is_empty()
                    && !position[separator + 1..].is_empty()
                    && position
                        .iter()
                        .enumerate()
                        .all(|(index, byte)| index == separator || byte.is_ascii_digit())
            })
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

    /// What is known about an unexpected end: launch, child state, the complete
    /// escaped PTY output and a bounded process-tree snapshot. Failure paths
    /// only; a snapshot failure is text and never replaces the original error.
    pub fn report(&mut self) -> String {
        let state = match self.child.try_wait() {
            Ok(Some(status)) => format!("exited {status:?}"),
            Ok(None) => "running".to_owned(),
            Err(error) => format!("unknown ({error})"),
        };
        let tree = match self.child_id() {
            Ok(pid) => kuru_platform::unix::snapshot::describe(pid),
            Err(error) => format!("snapshot unavailable: {error}"),
        };
        format!(
            "launch: {}; child {state}; complete PTY output ({} bytes): {}; {tree}",
            self.launch,
            self.output.len(),
            escaped(&self.output, REPORT_OUTPUT_LIMIT)
        )
    }

    // Output still queued when an exit was observed: what the child wrote just
    // before exiting. Process exit closes the last slave descriptor, so drain
    // into `output` and the parser until the reader reports that close (EIO or
    // EOF) or `LATE_OUTPUT_WINDOW` ends. Returns the report section.
    fn drain_after_exit(&mut self) -> Result<String> {
        let deadline = Instant::now() + LATE_OUTPUT_WINDOW;
        let mut late = Vec::new();
        let mut eof = false;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match self.receive_output(remaining.min(TICK))? {
                Ok(Ok(bytes)) => {
                    self.parser.process(&bytes);
                    self.output.extend_from_slice(&bytes);
                    late.extend(bytes);
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    eof = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        Ok(format!(
            "output not yet read when the exit was seen: {} bytes (EOF reached={eof} \
             within {LATE_OUTPUT_WINDOW:?}): {}",
            late.len(),
            escaped(&late, REPORT_OUTPUT_LIMIT)
        ))
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
                // Diagnostics show what was read when the exit was seen; the
                // drain then completes `output` before the predicate decides.
                let observed = self.diagnostics();
                let late = self.drain_after_exit()?;
                if predicate(self)? {
                    return Ok(());
                }
                bail!(
                    "{description}: process exited {status:?}\n{observed}\n{late}\n{}",
                    self.report()
                );
            }
            ensure!(
                Instant::now() < deadline,
                "{description}: timed out after {timeout:?}; process {} is still running\n{}\n{}",
                self.child_id()?,
                self.diagnostics(),
                self.report()
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
        self.submit(text)?;
        if let Some(picker) = picker {
            self.wait_text(&[picker, "Esc back"], &[])
        } else {
            self.wait_text(&["What shall we explore or build?", "enter send"], &[])
        }
    }

    /// Enters `text` at an idle composer and presses Enter, without waiting
    /// for the dispatched operation to settle.
    pub fn submit(&mut self, text: &str) -> Result<()> {
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
        self.send(b"\r")
    }

    /// Waits for a dream submitted by `submit` to finish, bounded by the
    /// caller's stated budget (the sandbox startup budget: a dream is memory
    /// work on the same store, unlike the frame and input waits that keep
    /// READY_TIMEOUT). The welcome placeholder and `enter send` return only
    /// when the operation clears `busy`; the activity panel keeps its
    /// `dream · …` entry and a later event can replace the busy status label,
    /// so the status row above the separator must also no longer lead with
    /// `dream · `. Expiry reports the captured screen.
    #[allow(dead_code)] // Used by terminal.rs; each integration test compiles this support module alone.
    pub fn wait_dream_settled(&mut self, timeout: Duration) -> Result<()> {
        self.wait("the dream settles", timeout, |terminal| {
            let screen = terminal.screen();
            let rows = screen.lines().collect::<Vec<_>>();
            let dreaming = rows
                .iter()
                .rposition(|row| row.trim_start().starts_with(".    ."))
                .and_then(|separator| separator.checked_sub(1))
                .and_then(|status| rows.get(status))
                .is_some_and(|row| {
                    // The busy row is `{spinner} {status}  ·  {seconds}s`.
                    row.trim_start()
                        .split_once(' ')
                        .is_some_and(|(_, label)| label.starts_with("dream · "))
                });
            Ok(!dreaming
                && screen.contains("What shall we explore or build?")
                && screen.contains("enter send"))
        })
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

    /// Waits until the child has exited without reaping it or reading its
    /// output, so a following wait still observes the exit itself.
    pub fn wait_exited(&self, timeout: Duration) -> Result<()> {
        let raw_pid = self.child_id()?.try_into()?;
        let pid =
            RustixPid::from_raw(raw_pid).context("terminal child has an invalid process ID")?;
        let deadline = Instant::now() + timeout;
        loop {
            if waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )?
            .is_some()
            {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "terminal child {pid} did not exit within {timeout:?}\n{}",
                self.diagnostics()
            );
            std::thread::sleep(TICK);
        }
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> Result<()> {
        // The final redraw can exceed the PTY buffer. Keep reading until exit.
        let deadline = Instant::now() + timeout;
        let status = loop {
            self.read_for(TICK)?;
            // Read the clock after the observation: an exit seen past the
            // deadline fails here as a late exit, so the drain below starts
            // only after a timely exit, with part of the budget left.
            let status = self.child.try_wait()?;
            ensure!(
                Instant::now() < deadline,
                "process exits: timed out after {timeout:?}; process {} {}\n{}\n{}",
                self.child_id()?,
                match &status {
                    Some(status) => format!(
                        "did not exit within {timeout:?}: its exit ({status:?}) was seen after the deadline"
                    ),
                    None => "is still running".to_owned(),
                },
                self.diagnostics(),
                self.report()
            );
            if let Some(status) = status {
                break status;
            }
        };
        // Process exit closes the final slave descriptor. Drain until the
        // reader observes that close so no queued final frame or large payload
        // is lost merely because waitpid won the race. No product wait sits
        // inside this drain: the kernel closes an exiting process's
        // descriptors before its exit can be waited for, and no descendant of
        // the child inherits the slave (the memory owner and the browser
        // opener get null stdio, kuru-memory `spawn_service` and
        // src/authentication.rs `open_browser`; tool shells, hooks and MCP
        // stdio servers get pipes). EOF is therefore already due when the exit
        // is seen, and only this process's reader thread stands between them.
        // The drain spends what remains of the caller's `timeout`, not a flat
        // window a descheduled reader could miss. The loop above admits only
        // an exit seen before the deadline, so reaching it here reports output
        // still open after a timely exit: a leaked holder.
        drain_exit_output(
            self.receive.as_ref().context("terminal output is closed")?,
            &mut self.parser,
            &mut self.output,
            deadline,
        )
        .with_context(|| {
            format!(
                "PTY output did not close after process exit within the {timeout:?} wait\n{}",
                self.diagnostics()
            )
        })?;
        ensure!(
            status.success(),
            "child failed: {status:?}\n{}\n{}",
            self.diagnostics(),
            self.report()
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
