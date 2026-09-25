use anyhow::{Context, Result, bail, ensure};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, SlavePty};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub const READY: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(20);
const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

pub fn composer_frame_ready(screen: &vt100::Screen, draft: &str) -> bool {
    let cursor = screen.cursor_position();
    // contents() joins autowrapped rows into logical lines. ConPTY emits those
    // wraps while painting its grid, so cursor coordinates require real rows.
    screen
        .rows(0, screen.size().1)
        .enumerate()
        .any(|(row, line)| {
            line.find(draft).is_some_and(|byte| {
                let col: usize = line[..byte + draft.len()]
                    .chars()
                    .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                    .sum();
                cursor == (row as u16, col as u16) && !screen.hide_cursor()
            })
        })
}

pub struct Terminal {
    child: Box<dyn Child + Send + Sync>,
    console: Console,
    receive: mpsc::Receiver<std::io::Result<Vec<u8>>>,
    parser: vt100::Parser,
    pub output: Vec<u8>,
    directory: PathBuf,
    sequence: usize,
}

// Also guards construction errors before a Terminal/child exists. No remaining
// console owner may reach a synchronous native destructor on the test thread.
struct Console {
    master: Option<Box<dyn MasterPty + Send>>,
    slave: Option<Box<dyn SlavePty + Send>>,
    writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    reader: Option<JoinHandle<()>>,
    closing: Vec<JoinHandle<()>>,
    directory: PathBuf,
}

impl Console {
    fn close(&mut self, timeout: Duration) -> Result<()> {
        let writer = self.writer.take();
        let master = self.master.take();
        let slave = self.slave.take();
        if writer.is_some() || master.is_some() || slave.is_some() {
            // The output pump owns its reader and DSR writer independently.
            // Keep it alive throughout ClosePseudoConsole, including on errors.
            self.closing.push(std::thread::spawn(move || {
                drop(writer);
                drop(slave);
                drop(master);
            }));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let reader_done = self.reader.as_ref().is_none_or(JoinHandle::is_finished);
            let close_done = self.closing.iter().all(JoinHandle::is_finished);
            if reader_done && close_done {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "ConPTY cleanup remains unproven at {}: reader_done={reader_done}, close_done={close_done}",
                self.directory.display()
            );
            std::thread::sleep(TICK);
        }
        for closing in self.closing.drain(..) {
            closing
                .join()
                .map_err(|_| anyhow::anyhow!("ConPTY close thread panicked"))?;
        }
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| anyhow::anyhow!("ConPTY reader thread panicked"))?;
        }
        Ok(())
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        if let Err(error) = self.close(Duration::from_secs(5)) {
            // Workers retain the actual resources until native close completes.
            // Dropping their JoinHandles neither blocks nor proves completion.
            eprintln!("{error:#}");
        }
    }
}

impl Terminal {
    pub fn spawn(directory: &Path, plan: serde_json::Value, rows: u16, cols: u16) -> Result<Self> {
        std::fs::create_dir_all(directory)?;
        std::fs::write(directory.join("plan.json"), serde_json::to_vec(&plan)?)?;
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut console = Console {
            master: Some(pair.master),
            slave: Some(pair.slave),
            writer: None,
            reader: None,
            closing: Vec::new(),
            directory: directory.into(),
        };
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_kuru-terminal-windows-fixture"));
        command.arg(directory);
        command.cwd(directory);
        command.env_clear();
        for key in ["SystemRoot", "LLVM_PROFILE_FILE"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env("TERM", "xterm-256color");
        let master = console.master.as_ref().context("console master")?;
        let mut input = master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(master.take_writer()?));
        console.writer = Some(writer.clone());
        let replies = writer.clone();
        let (send, receive) = mpsc::channel();
        console.reader = Some(std::thread::spawn(move || {
            let mut total = 0usize;
            let mut tail = Vec::new();
            loop {
                let mut buffer = [0; 65536];
                match input.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        // portable-pty 0.9.0 requests inherited cursor state.
                        // Service ConPTY's DSR even during CreateProcess/close;
                        // this newly created test terminal begins at row1,col1.
                        tail.extend_from_slice(&buffer[..count]);
                        for _ in 0..tail.windows(4).filter(|part| *part == b"\x1b[6n").count() {
                            let result = replies
                                .lock()
                                .map_err(|_| std::io::Error::other("console reply lock poisoned"))
                                .and_then(|mut writer| {
                                    writer.write_all(b"\x1b[1;1R")?;
                                    writer.flush()
                                });
                            if let Err(error) = result {
                                let _ = send.send(Err(error));
                            }
                        }
                        if tail.len() > 3 {
                            tail.drain(..tail.len() - 3);
                        }
                        total += count;
                        if total <= OUTPUT_LIMIT {
                            let _ = send.send(Ok(buffer[..count].to_vec()));
                        } else if total - count <= OUTPUT_LIMIT {
                            let _ = send.send(Err(std::io::Error::other(
                                "ConPTY output exceeds fixture limit",
                            )));
                        }
                        // Keep draining after a limit failure so console
                        // shutdown cannot deadlock on a full output pipe.
                    }
                    Err(error) => {
                        let _ = send.send(Err(error));
                        break;
                    }
                }
            }
        }));
        // Every test in this binary holds its async SERIAL lock for the whole
        // scenario; no ordinary spawn overlaps this one ConPTY creation. The
        // output pump is already live for the inherited-cursor handshake.
        let child = console
            .slave
            .as_ref()
            .context("console slave")?
            .spawn_command(command)?;
        drop(console.slave.take());
        Ok(Self {
            child,
            console,
            receive,
            parser: vt100::Parser::new(rows, cols, 0),
            output: Vec::new(),
            directory: directory.into(),
            sequence: 0,
        })
    }

    pub fn screen(&self) -> String {
        self.parser.screen().contents()
    }

    /// Observe the real master destructor without replacing native ConPTY I/O.
    pub fn wrap_master(
        &mut self,
        wrap: impl FnOnce(Box<dyn MasterPty + Send>) -> Box<dyn MasterPty + Send>,
    ) -> Result<()> {
        let master = self
            .console
            .master
            .take()
            .context("console already closed")?;
        self.console.master = Some(wrap(master));
        Ok(())
    }

    pub fn read_for(&mut self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match self.receive.recv_timeout(remaining.min(TICK)) {
                Ok(Ok(bytes)) => {
                    self.parser.process(&bytes);
                    self.output.extend(bytes);
                }
                Ok(Err(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
                    ) && self.child.try_wait()?.is_some() =>
                {
                    break;
                }
                Ok(Err(error)) => return Err(error.into()),
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    }

    pub fn wait(
        &mut self,
        description: &str,
        timeout: Duration,
        mut ready: impl FnMut(&Self) -> bool,
    ) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.read_for(TICK)?;
            if ready(self) {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait()? {
                bail!("{description}: child exited {status:?}\n{}", self.screen());
            }
            ensure!(
                Instant::now() < deadline,
                "{description}: timed out after {timeout:?}\n{}",
                self.screen()
            );
        }
    }

    pub fn text(&mut self, values: &[&str], timeout: Duration) -> Result<()> {
        self.wait(
            &format!("screen contains {values:?}"),
            timeout,
            |terminal| values.iter().all(|value| terminal.screen().contains(value)),
        )
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        let mut writer = self
            .console
            .writer
            .as_ref()
            .context("terminal input is closed")?
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal writer lock poisoned"))?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    pub fn focus(&mut self, focused: bool) -> Result<()> {
        let path = self
            .directory
            .join(format!("control-{}.json", self.sequence));
        let temporary = path.with_extension("pending");
        std::fs::write(
            &temporary,
            serde_json::to_vec(&serde_json::json!({"focus":focused}))?,
        )?;
        std::fs::rename(temporary, path)?;
        let ack = self.directory.join(format!("ack-{}.json", self.sequence));
        self.wait("native focus record acknowledged", READY, |_| ack.is_file())?;
        self.sequence += 1;
        Ok(())
    }

    pub fn composer(&mut self, draft: &str) -> Result<()> {
        self.wait(
            &format!("completed visible composer cursor after {draft:?}"),
            READY,
            |terminal| composer_frame_ready(terminal.parser.screen(), draft),
        )
        .with_context(|| {
            format!(
                "native cursor {:?}, hidden={}",
                self.parser.screen().cursor_position(),
                self.parser.screen().hide_cursor()
            )
        })
    }

    /// Assert the entire observation interval, without first waiting for silence.
    pub fn quiet(&mut self, description: &str, duration: Duration) -> Result<()> {
        let settled = self.output.len();
        let cursor = self.parser.screen().cursor_position();
        let hidden = self.parser.screen().hide_cursor();
        let before = self.screen();
        self.read_for(duration)?;
        let appended = &self.output[settled..];
        ensure!(
            appended.is_empty(),
            "{description}: {} additional bytes over {duration:?}; \
             preceding 512 bytes: {}; first 4096 new bytes: {}; \
             cursor {cursor:?}/hidden={hidden} -> {:?}/hidden={}; \
             screen changed={}; screen prefix: {}",
            appended.len(),
            self.output[settled.saturating_sub(512)..settled].escape_ascii(),
            appended[..appended.len().min(4096)].escape_ascii(),
            self.parser.screen().cursor_position(),
            self.parser.screen().hide_cursor(),
            before != self.screen(),
            self.screen()
                .chars()
                .take(2048)
                .collect::<String>()
                .escape_debug(),
        );
        Ok(())
    }

    /// A completed ConPTY frame may be followed by Ratatui's one inert
    /// empty-diff style reset. Keep every actual paint or repeated draw visible.
    pub fn quiet_with_optional_style_reset(
        &mut self,
        description: &str,
        duration: Duration,
    ) -> Result<()> {
        let settled = self.output.len();
        let before = self.parser.screen().clone();
        self.read_for(duration)?;
        let appended = &self.output[settled..];
        let after = self.parser.screen();
        let (rows, columns) = before.size();
        let cells_unchanged = before.size() == after.size()
            && (0..rows).all(|row| {
                before.row_wrapped(row) == after.row_wrapped(row)
                    && (0..columns)
                        .all(|column| before.cell(row, column) == after.cell(row, column))
            });
        ensure!(
            (appended.is_empty() || appended == b"\x1b[m")
                && cells_unchanged
                && before.cursor_position() == after.cursor_position()
                && before.hide_cursor() == after.hide_cursor()
                && before.alternate_screen() == after.alternate_screen()
                && before.input_mode_formatted() == after.input_mode_formatted(),
            "{description}: {} additional bytes over {duration:?}; first 4096 new bytes: {}; \
             cells/styles unchanged={cells_unchanged}; cursor {:?}/hidden={} -> {:?}/hidden={}",
            appended.len(),
            appended[..appended.len().min(4096)].escape_ascii(),
            before.cursor_position(),
            before.hide_cursor(),
            after.cursor_position(),
            after.hide_cursor(),
        );
        Ok(())
    }

    pub fn command(&mut self, value: &str) -> Result<()> {
        self.text(&["enter send"], READY)?;
        self.send(value.as_bytes())?;
        self.composer(value)?;
        self.send(b"\r")?;
        self.wait("command completed", READY, |terminal| {
            terminal.screen().contains("enter send")
                && terminal.screen().contains("What shall we explore")
        })
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.console
            .master
            .as_ref()
            .context("terminal closed")?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        self.parser.screen_mut().set_size(rows, cols);
        Ok(())
    }

    pub fn finish(&mut self, timeout: Duration) -> Result<serde_json::Value> {
        let deadline = Instant::now() + timeout;
        loop {
            self.read_for(TICK)?;
            if let Some(status) = self.child.try_wait()? {
                let report: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(self.directory.join("report.json")).with_context(|| {
                        format!(
                            "missing console observer report: {status:?}\n{}",
                            self.screen()
                        )
                    })?,
                )?;
                ensure!(
                    status.success(),
                    "console observer failed: {report}\n{}",
                    self.screen()
                );
                ensure!(
                    report["before"] == report["after"],
                    "native console modes differ: {report}"
                );
                self.close_output(READY)?;
                return Ok(report);
            }
            ensure!(
                Instant::now() < deadline,
                "console fixture exit timed out\n{}",
                self.screen()
            );
        }
    }

    fn close_output(&mut self, timeout: Duration) -> Result<()> {
        self.console.close(timeout)?;
        self.read_for(TICK)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(TICK);
            }
            if self.child.try_wait().ok().flatten().is_none() {
                eprintln!(
                    "ConPTY child cleanup remains unproven at {}\n{}",
                    self.directory.display(),
                    self.screen()
                );
            }
        }
    }
}
