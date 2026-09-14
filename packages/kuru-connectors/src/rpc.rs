use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::Directory;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

use crate::{MAX_BYTES, http::rpc_result, mcp::Admission, redaction::Scanner};

#[cfg(unix)]
use kuru_platform::unix::{GroupPresence, OwnedProcessGroup, Reap, RootState, Termination};
#[cfg(unix)]
type Input = tokio::process::ChildStdin;
#[cfg(unix)]
type Output = tokio::process::ChildStdout;
#[cfg(unix)]
type ErrorPipe = tokio::process::ChildStderr;
#[cfg(unix)]
type Owner = OwnedProcessGroup;

#[cfg(windows)]
use kuru_platform::windows::{
    pipe::Pipe,
    process::{NativeChild, Stdio as NativeStdio},
};
#[cfg(windows)]
type Input = Pipe;
#[cfg(windows)]
type Output = Pipe;
#[cfg(windows)]
type ErrorPipe = Pipe;
#[cfg(windows)]
type Owner = NativeChild;

const STDERR_LIMIT: usize = 8 * 1024;
const CLEANUP: Duration = Duration::from_secs(5);
const GRACE: Duration = Duration::from_millis(300);
const STDERR_DRAIN: Duration = Duration::from_secs(1);

enum Command {
    Send(Value, oneshot::Sender<Result<()>>),
    #[cfg(test)]
    Read(oneshot::Sender<Result<Value>>),
    Request(String, Value, Duration, oneshot::Sender<Result<Value>>),
    Close(oneshot::Sender<Result<()>>),
}

/// Handle for one serial session whose worker owns the process and pipes.
pub(crate) struct Rpc {
    commands: mpsc::UnboundedSender<Command>,
    stderr: Arc<StdMutex<StderrTail>>,
    completion: Arc<Completion>,
    started: Option<oneshot::Receiver<Result<()>>>,
}

impl Rpc {
    #[cfg(test)]
    pub(crate) fn unconfirmed_for_test() -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        drop(receiver);
        Self {
            commands,
            stderr: Arc::new(StdMutex::new(StderrTail::default())),
            completion: Arc::new(Completion::default()),
            started: None,
        }
    }

    pub fn spawn(
        program: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
        root_guard: Arc<Directory>,
        admission: Arc<Admission>,
    ) -> Result<Self> {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (ready, started) = oneshot::channel();
        let stderr = Arc::new(StdMutex::new(StderrTail::default()));
        let completion = Arc::new(Completion::default());
        let worker_stderr = stderr.clone();
        let worker_completion = completion.clone();
        let (program, args, env, cwd) = (
            program.to_owned(),
            args.to_vec(),
            env.clone(),
            cwd.to_path_buf(),
        );
        std::thread::Builder::new()
            .name("kuru-mcp-stdio".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    worker_completion.confirmed.store(true, Ordering::Release);
                    worker_completion.finished.store(true, Ordering::Release);
                    let _ = ready.send(Err(anyhow::anyhow!("MCP worker runtime failed")));
                    return;
                };
                runtime.block_on(worker(
                    receiver,
                    ready,
                    program,
                    args,
                    env,
                    cwd,
                    root_guard,
                    admission,
                    worker_stderr,
                    worker_completion,
                ));
            })
            .context("MCP worker thread failed")?;
        Ok(Self {
            commands,
            stderr,
            completion,
            started: Some(started),
        })
    }

    pub async fn ready(&mut self) -> Result<()> {
        let Some(started) = self.started.take() else {
            return Ok(());
        };
        started.await.context("MCP worker stopped during startup")?
    }

    pub async fn send(&mut self, message: Value) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Send(message, tx))
            .map_err(|_| anyhow::anyhow!("JSON-RPC session is closed"))?;
        rx.await.context("JSON-RPC send was cancelled")?
    }

    #[cfg(test)]
    pub async fn read(&mut self) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Read(tx))
            .map_err(|_| anyhow::anyhow!("JSON-RPC session is closed"))?;
        rx.await.context("JSON-RPC read was cancelled")?
    }

    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        duration: Duration,
    ) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Request(method.into(), params, duration, tx))
            .map_err(|_| anyhow::anyhow!("JSON-RPC session is closed"))?;
        rx.await.context("JSON-RPC request was cancelled")?
    }

    pub(crate) fn stderr_diagnostic(&self) -> Option<String> {
        lock(&self.stderr).diagnostic()
    }

    pub async fn close(&mut self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        if self.commands.send(Command::Close(tx)).is_err() {
            return self.completion.result();
        }
        match tokio::time::timeout(CLEANUP + GRACE + STDERR_DRAIN + Duration::from_secs(1), rx)
            .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => self.completion.result(),
            Err(_) => self.completion.result(),
        }
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        let (tx, _) = oneshot::channel();
        let _ = self.commands.send(Command::Close(tx));
    }
}

struct Session {
    owner: Owner,
    input: Option<Input>,
    output: Option<BufReader<Output>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
    completion: Arc<Completion>,
    _root_guard: Arc<Directory>,
    next_id: u64,
    events: VecDeque<Value>,
}

#[allow(clippy::too_many_arguments)]
async fn worker(
    mut commands: mpsc::UnboundedReceiver<Command>,
    ready: oneshot::Sender<Result<()>>,
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: PathBuf,
    root_guard: Arc<Directory>,
    admission: Arc<Admission>,
    stderr: Arc<StdMutex<StderrTail>>,
    completion: Arc<Completion>,
) {
    let _finished = FinishOnDrop(completion.clone());
    let mut session = match Session::spawn(
        &program,
        &args,
        &env,
        &cwd,
        root_guard,
        admission,
        stderr,
        completion.clone(),
    )
    .await
    {
        Startup::Ready(session) => session,
        Startup::Rejected => {
            completion.confirmed.store(true, Ordering::Release);
            let _ = ready.send(Err(anyhow::anyhow!(
                "configured MCP process failed to start"
            )));
            return;
        }
        Startup::Retaining(mut owner) => {
            let _ = ready.send(Err(anyhow::anyhow!(
                "configured MCP process failed to start"
            )));
            owner.retain().await;
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        session.finish(false).await;
        return;
    }
    while let Some(command) = commands.recv().await {
        let keep_running = match command {
            Command::Send(value, mut reply) => {
                let result = session.dispatch(value, &mut reply).await;
                settle(&mut session, reply, result).await
            }
            #[cfg(test)]
            Command::Read(mut reply) => {
                let result = session.receive(&mut reply).await;
                settle(&mut session, reply, result).await
            }
            Command::Request(method, params, duration, mut reply) => {
                let result = session.request(&method, params, duration, &mut reply).await;
                settle(&mut session, reply, result).await
            }
            Command::Close(reply) => {
                commands.close();
                let confirmed = session.cleanup(true).await;
                let result = confirmed
                    .then_some(())
                    .ok_or_else(|| anyhow::anyhow!("MCP cleanup remains unconfirmed"));
                let _ = reply.send(result);
                if !confirmed {
                    session.retain().await;
                }
                false
            }
        };
        if !keep_running {
            return;
        }
    }
    session.finish(false).await;
}

enum Operation<T> {
    Complete(Result<T>),
    Stop(Result<T>),
    CallerLost,
}

enum Startup {
    Ready(Session),
    Rejected,
    Retaining(StartupOwner),
}

struct StartupOwner {
    owner: Owner,
    completion: Arc<Completion>,
    _root_guard: Arc<Directory>,
}

impl StartupOwner {
    async fn retain(&mut self) {
        while !cleanup_owner(&mut self.owner, false, Instant::now() + CLEANUP).await {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.completion.confirmed.store(true, Ordering::Release);
    }
}

async fn settle<T>(
    session: &mut Session,
    reply: oneshot::Sender<Result<T>>,
    operation: Operation<T>,
) -> bool {
    match operation {
        Operation::Complete(result) => {
            if reply.send(result).is_ok() {
                true
            } else {
                session.finish(false).await;
                false
            }
        }
        Operation::CallerLost => {
            session.finish(false).await;
            false
        }
        Operation::Stop(result) => {
            let confirmed = session.cleanup(false).await;
            let _ = reply.send(result);
            if !confirmed {
                session.retain().await;
            }
            false
        }
    }
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    async fn spawn(
        program: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
        root_guard: Arc<Directory>,
        admission: Arc<Admission>,
        stderr: Arc<StdMutex<StderrTail>>,
        completion: Arc<Completion>,
    ) -> Startup {
        #[cfg(unix)]
        let (owner, input, output, error) = {
            let mut command = std::process::Command::new(program);
            command
                .args(args)
                .envs(env)
                .current_dir(cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if root_guard.revalidate().is_err() {
                return Startup::Rejected;
            }
            if admission.enter().is_err() {
                return Startup::Rejected;
            }
            let mut owner = match OwnedProcessGroup::spawn(command) {
                Ok(owner) => owner,
                Err(_) => return Startup::Rejected,
            };
            #[cfg(test)]
            admission.after_launch().await;
            #[cfg(test)]
            if admission.fail_setup() {
                return Startup::Retaining(StartupOwner {
                    owner,
                    completion,
                    _root_guard: root_guard,
                });
            }
            let pipes: Result<_> = (|| {
                let input = Input::from_std(owner.take_stdin()?)?;
                let output = Output::from_std(owner.take_stdout()?)?;
                let error = ErrorPipe::from_std(owner.take_stderr()?)?;
                Ok((input, output, error))
            })();
            let (input, output, error) = match pipes {
                Ok(pipes) => pipes,
                Err(_) => {
                    return Startup::Retaining(StartupOwner {
                        owner,
                        completion,
                        _root_guard: root_guard,
                    });
                }
            };
            (owner, input, output, error)
        };
        #[cfg(windows)]
        let (owner, input, output, error) = {
            let mut spec = match crate::process::configured(program, args, env, cwd) {
                Ok(spec) => spec,
                Err(_) => return Startup::Rejected,
            };
            spec.stdin = NativeStdio::Pipe;
            spec.stdout = NativeStdio::Pipe;
            spec.stderr = NativeStdio::Pipe;
            if root_guard.revalidate().is_err() {
                return Startup::Rejected;
            }
            if admission.enter().is_err() {
                return Startup::Rejected;
            }
            let mut owner = match spec.spawn().await {
                Ok(owner) => owner,
                Err(_) => return Startup::Rejected,
            };
            #[cfg(test)]
            admission.after_launch().await;
            #[cfg(test)]
            if admission.fail_setup() {
                return Startup::Retaining(StartupOwner {
                    owner,
                    completion,
                    _root_guard: root_guard,
                });
            }
            let pipes: Result<_> = (|| {
                let input = owner.take_stdin().context("missing MCP stdin")?;
                let output = owner.take_stdout().context("missing MCP stdout")?;
                let error = owner.take_stderr().context("missing MCP stderr")?;
                Ok((input, output, error))
            })();
            let (input, output, error) = match pipes {
                Ok(pipes) => pipes,
                Err(_) => {
                    return Startup::Retaining(StartupOwner {
                        owner,
                        completion,
                        _root_guard: root_guard,
                    });
                }
            };
            (owner, input, output, error)
        };
        Startup::Ready(Self {
            owner,
            input: Some(input),
            output: Some(BufReader::new(output)),
            stderr_task: Some(tokio::spawn(drain(error, stderr))),
            completion,
            _root_guard: root_guard,
            next_id: 0,
            events: VecDeque::new(),
        })
    }

    async fn dispatch(
        &mut self,
        message: Value,
        reply: &mut oneshot::Sender<Result<()>>,
    ) -> Operation<()> {
        let operation = self.send_wire(message);
        tokio::pin!(operation);
        tokio::select! {
            biased;
            _ = reply.closed() => Operation::CallerLost,
            result = &mut operation => match result {
                Ok(()) => Operation::Complete(Ok(())),
                Err(_) => Operation::Stop(Err(anyhow::anyhow!("JSON-RPC dispatch failed"))),
            },
        }
    }

    #[cfg(test)]
    async fn receive(&mut self, reply: &mut oneshot::Sender<Result<Value>>) -> Operation<Value> {
        if let Some(event) = self.events.pop_front() {
            return Operation::Complete(Ok(event));
        }
        let operation = self.read_wire();
        tokio::pin!(operation);
        tokio::select! {
            biased;
            _ = reply.closed() => Operation::CallerLost,
            result = &mut operation => match result {
                Ok(value) => Operation::Complete(Ok(value)),
                Err(_) => Operation::Stop(Err(anyhow::anyhow!("JSON-RPC protocol failure"))),
            },
        }
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        duration: Duration,
        reply: &mut oneshot::Sender<Result<Value>>,
    ) -> Operation<Value> {
        self.next_id = self.next_id.saturating_add(1);
        let operation = self.request_wire(json!(self.next_id), method, params);
        tokio::pin!(operation);
        tokio::select! {
            biased;
            _ = reply.closed() => Operation::CallerLost,
            _ = tokio::time::sleep(duration) => Operation::Stop(
                Err(anyhow::anyhow!("JSON-RPC request timed out"))
            ),
            result = &mut operation => match result {
                Ok(value) => Operation::Complete(Ok(value)),
                Err(_) => Operation::Stop(Err(anyhow::anyhow!("JSON-RPC protocol failure"))),
            },
        }
    }

    async fn request_wire(&mut self, id: Value, method: &str, params: Value) -> Result<Value> {
        self.send_wire(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        loop {
            let message = self.read_wire().await?;
            if self.reject_request(&message).await? {
                continue;
            }
            if message.get("id") == Some(&id) {
                return rpc_result(message, &id);
            }
            ensure!(message.get("id").is_none(), "unexpected response ID");
            ensure!(self.events.len() < 1024, "notification queue limit");
            self.events.push_back(message);
        }
    }

    async fn send_wire(&mut self, message: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&message)?;
        ensure!(bytes.len() <= MAX_BYTES, "request size limit");
        bytes.push(b'\n');
        let input = self.input.as_mut().context("connection closed")?;
        input.write_all(&bytes).await?;
        input.flush().await?;
        Ok(())
    }

    async fn read_wire(&mut self) -> Result<Value> {
        let output = self.output.as_mut().context("connection closed")?;
        let mut bytes = Vec::new();
        let count = output
            .take((MAX_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await?;
        ensure!(count > 0, "closed output");
        ensure!(count <= MAX_BYTES, "response size limit");
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn reject_request(&mut self, message: &Value) -> Result<bool> {
        if message.get("method").is_some() && message.get("id").is_some() {
            let response = if message["method"] == "ping" {
                json!({"jsonrpc":"2.0","id":message["id"],"result":{}})
            } else {
                json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32601,"message":"Kuru does not authorize server-initiated tools or approvals"}})
            };
            self.send_wire(response).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn finish(&mut self, graceful: bool) {
        if !self.cleanup(graceful).await {
            self.retain().await;
        }
    }

    async fn retain(&mut self) {
        while !self.cleanup(false).await {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn cleanup(&mut self, graceful: bool) -> bool {
        self.input.take();
        self.output.take();
        let deadline = Instant::now() + CLEANUP;
        let confirmed = cleanup_owner(&mut self.owner, graceful, deadline).await;
        if confirmed {
            self.finish_stderr(Instant::now() + STDERR_DRAIN).await;
            self.completion.confirmed.store(true, Ordering::Release);
        }
        confirmed
    }

    async fn finish_stderr(&mut self, deadline: Instant) {
        let Some(mut task) = self.stderr_task.take() else {
            return;
        };
        if tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            &mut task,
        )
        .await
        .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}

#[derive(Default)]
struct Completion {
    confirmed: AtomicBool,
    finished: AtomicBool,
}

impl Completion {
    fn result(&self) -> Result<()> {
        if self.confirmed.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("MCP cleanup remains unconfirmed"))
        }
    }
}

struct FinishOnDrop(Arc<Completion>);

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
    }
}

async fn drain(mut stderr: ErrorPipe, tail: Arc<StdMutex<StderrTail>>) {
    let mut scanner = Scanner::new();
    let mut input = [0_u8; 512];
    let mut projected = Vec::new();
    loop {
        match stderr.read(&mut input).await {
            Ok(0) => {
                if scanner.finish(&mut projected).is_err() {
                    lock(&tail).withhold();
                } else {
                    lock(&tail).push(&projected);
                }
                return;
            }
            Ok(count) => {
                lock(&tail).observe(count);
                if scanner.push(&input[..count], &mut projected).is_err() {
                    lock(&tail).withhold();
                    while let Ok(count) = stderr.read(&mut input).await {
                        if count == 0 {
                            return;
                        }
                        lock(&tail).observe(count);
                    }
                    return;
                }
                lock(&tail).push(&projected);
                projected.clear();
            }
            Err(_) => {
                lock(&tail).withhold();
                return;
            }
        }
    }
}

fn lock<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Default)]
struct StderrTail {
    raw: u64,
    retained: usize,
    truncated: bool,
    withheld: bool,
    chunks: VecDeque<Vec<u8>>,
}

impl StderrTail {
    fn observe(&mut self, count: usize) {
        self.raw = self.raw.saturating_add(count as u64);
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.withheld {
            return;
        }
        if bytes.len() > STDERR_LIMIT {
            self.withhold();
            return;
        }
        while self.retained.saturating_add(bytes.len()) > STDERR_LIMIT {
            let Some(removed) = self.chunks.pop_front() else {
                break;
            };
            self.retained = self.retained.saturating_sub(removed.len());
            self.truncated = true;
        }
        self.retained += bytes.len();
        self.chunks.push_back(bytes.to_vec());
    }

    fn withhold(&mut self) {
        self.withheld = true;
        self.truncated = true;
        self.retained = 0;
        self.chunks.clear();
    }

    fn diagnostic(&self) -> Option<String> {
        if self.raw == 0 {
            return None;
        }
        if self.withheld {
            return Some(format!(
                "MCP stderr: {} bytes observed; diagnostic withheld",
                self.raw
            ));
        }
        let mut bytes = Vec::with_capacity(self.retained);
        self.chunks
            .iter()
            .for_each(|chunk| bytes.extend_from_slice(chunk));
        let label = if self.truncated {
            " bytes observed; showing filtered tail: "
        } else {
            " bytes observed; filtered: "
        };
        Some(format!(
            "MCP stderr: {}{}{}",
            self.raw,
            label,
            escape_terminal(&bytes)
        ))
    }
}

fn escape_terminal(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    for &byte in bytes {
        match byte {
            b'\n' => escaped.push('\n'),
            b' '..=b'~' => escaped.push(char::from(byte)),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\x{byte:02x}");
            }
        }
    }
    escaped
}

#[cfg(unix)]
async fn cleanup_owner(owner: &mut Owner, graceful: bool, deadline: Instant) -> bool {
    if graceful {
        let grace = Instant::now() + GRACE;
        while Instant::now() < grace {
            if matches!(owner.root_state(), RootState::Exited | RootState::Reaped(_)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    loop {
        match owner.terminate_before_reap() {
            Termination::Signalled(_) | Termination::InvalidPhase => break,
            Termination::Interrupted => {}
            Termination::Disarmed(_) => return false,
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    loop {
        match owner.reap_if_exited() {
            Reap::Reaped(_) => break,
            Reap::NotExited | Reap::Interrupted => {}
            Reap::Disarmed(_) | Reap::InvalidPhase => return false,
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    loop {
        match owner.presence_after_reap() {
            GroupPresence::Absent => return true,
            GroupPresence::Present | GroupPresence::PermissionDenied => {}
            GroupPresence::ObservationError(_) | GroupPresence::InvalidPhase => return false,
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(windows)]
async fn cleanup_owner(owner: &mut Owner, graceful: bool, deadline: Instant) -> bool {
    if graceful && owner.wait(GRACE).await.is_ok() {
        return true;
    }
    owner.terminate().is_ok()
        && owner
            .wait(deadline.saturating_duration_since(Instant::now()))
            .await
            .is_ok()
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        task::{Context, Poll, Wake, Waker},
    };

    use super::*;
    use crate::{
        IO_TIMEOUT,
        test_support::{StdioFixture, Step},
    };
    use kuru_platform::fs::{NameRetention, Privacy};

    fn root_guard() -> Arc<Directory> {
        let root = Path::new(".").canonicalize().unwrap();
        Arc::new(Directory::open(&root, Privacy::Inherited, NameRetention::Pinned).unwrap())
    }

    struct CloseReplyWake {
        commands: mpsc::UnboundedSender<Command>,
        completion: Arc<Completion>,
        observation: StdMutex<Option<(bool, bool)>>,
        notified: tokio::sync::Notify,
    }

    impl Wake for CloseReplyWake {
        fn wake(self: Arc<Self>) {
            let observation = (
                self.commands.is_closed(),
                self.completion.confirmed.load(Ordering::Acquire),
            );
            assert!(
                lock(&self.observation).replace(observation).is_none(),
                "close reply receiver woke more than once"
            );
            self.notified.notify_one();
        }
    }

    #[tokio::test]
    async fn close_closes_admission_before_publishing_confirmed_completion() {
        let ready = json!({"ready":true});
        let script = StdioFixture::new([Step::Write(ready.clone()), Step::Eof]);
        let mut rpc = Rpc::spawn(
            script.command(),
            &[],
            &BTreeMap::new(),
            Path::new("."),
            root_guard(),
            Arc::new(Admission::new()),
        )
        .unwrap();
        rpc.ready().await.unwrap();
        assert_eq!(rpc.read().await.unwrap(), ready);

        let (reply, result) = oneshot::channel();
        let mut result = Box::pin(result);
        let wake = Arc::new(CloseReplyWake {
            commands: rpc.commands.clone(),
            completion: rpc.completion.clone(),
            observation: StdMutex::new(None),
            notified: tokio::sync::Notify::new(),
        });
        let waker = Waker::from(wake.clone());
        let mut context = Context::from_waker(&waker);
        assert!(matches!(result.as_mut().poll(&mut context), Poll::Pending));

        rpc.commands.send(Command::Close(reply)).unwrap();
        tokio::time::timeout(
            CLEANUP + GRACE + STDERR_DRAIN + Duration::from_secs(1),
            wake.notified.notified(),
        )
        .await
        .unwrap();
        assert_eq!(*lock(&wake.observation), Some((true, true)));
        result.await.unwrap().unwrap();
        rpc.close().await.unwrap();
        script.assert_completed(1);
    }

    #[tokio::test]
    async fn rpc_handles_pings_actions_and_notifications() {
        let script = StdioFixture::new([
            Step::StderrRepeat(20_000),
            Step::StderrRepeat(473),
            Step::Stderr("api_key=synthetic-secret \u{1b}[31m"),
            Step::StderrInvalid,
            Step::Read,
            Step::Write(json!({"id":900,"method":"ping"})),
            Step::Read,
            Step::Write(json!({"id":900,"method":"dangerous"})),
            Step::Read,
            Step::Write(json!({"method":"notice"})),
            Step::Write(json!({"id":1,"result":{"answer":42}})),
            Step::Eof,
        ]);
        let mut rpc = Rpc::spawn(
            script.command(),
            &[],
            &BTreeMap::new(),
            Path::new("."),
            root_guard(),
            Arc::new(Admission::new()),
        )
        .unwrap();
        rpc.ready().await.unwrap();
        assert_eq!(
            rpc.request("test", json!({}), IO_TIMEOUT).await.unwrap()["answer"],
            42
        );
        assert_eq!(rpc.read().await.unwrap()["method"], "notice");
        rpc.close().await.unwrap();
        rpc.close().await.unwrap();
        let diagnostic = rpc.stderr_diagnostic().unwrap();
        assert!(diagnostic.contains("[REDACTED:recognized-secret]"));
        assert!(diagnostic.contains("\\x1b"));
        assert!(diagnostic.contains("\\xff"));
        assert!(!diagnostic.contains("synthetic-secret"));
        assert!(diagnostic.contains("showing filtered tail"));
        script.assert_completed(1);
        let requests = script.conversations().remove(0);
        assert_eq!(requests[0]["method"], "test");
        assert_eq!(requests[1], json!({"jsonrpc":"2.0","id":900,"result":{}}));
        assert_eq!(requests[2]["id"], 900);
        assert_eq!(requests[2]["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn rpc_bounds_protocol_failures_timeout_and_oversized_dispatch() {
        let cases = [
            (vec![Step::Read], IO_TIMEOUT, "protocol failure"),
            (
                vec![Step::Read, Step::Raw("not JSON")],
                IO_TIMEOUT,
                "protocol failure",
            ),
            (
                vec![Step::Read, Step::Repeat(MAX_BYTES + 1)],
                IO_TIMEOUT,
                "protocol failure",
            ),
            (
                vec![Step::Read, Step::Sleep(5_000)],
                Duration::from_millis(100),
                "timed out",
            ),
        ];
        for (steps, deadline, expected) in cases {
            let script = StdioFixture::new(steps);
            let mut rpc = Rpc::spawn(
                script.command(),
                &[],
                &BTreeMap::new(),
                Path::new("."),
                root_guard(),
                Arc::new(Admission::new()),
            )
            .unwrap();
            rpc.ready().await.unwrap();
            let error = rpc.request("test", json!({}), deadline).await.unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            rpc.close().await.unwrap();
        }

        let ready = json!({"jsonrpc":"2.0","method":"fixture/ready","params":{}});
        let script = StdioFixture::new([Step::Write(ready.clone()), Step::Read]);
        let mut rpc = Rpc::spawn(
            script.command(),
            &[],
            &BTreeMap::new(),
            Path::new("."),
            root_guard(),
            Arc::new(Admission::new()),
        )
        .unwrap();
        rpc.ready().await.unwrap();
        assert_eq!(rpc.read().await.unwrap(), ready);
        let error = rpc.send(json!({"text":"x".repeat(MAX_BYTES)})).await;
        assert!(error.unwrap_err().to_string().contains("dispatch failed"));
        rpc.close().await.unwrap();
        assert_eq!(script.conversations(), vec![Vec::<Value>::new()]);
    }

    #[tokio::test]
    async fn worker_retains_cleanup_after_parent_runtime_loss() {
        let script = StdioFixture::new([Step::Read, Step::HoldStderr(5_000), Step::Sleep(5_000)]);
        let command = script.command().to_owned();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let runtime = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let mut session = Rpc::spawn(
                    &command,
                    &[],
                    &BTreeMap::new(),
                    Path::new("."),
                    root_guard(),
                    Arc::new(Admission::new()),
                )
                .unwrap();
                session.ready().await.unwrap();
                let rpc = Arc::new(tokio::sync::Mutex::new(session));
                let completion = rpc.lock().await.completion.clone();
                let call = rpc.clone();
                tokio::spawn(async move {
                    let _ = call
                        .lock()
                        .await
                        .request("mutate", json!({}), IO_TIMEOUT)
                        .await;
                });
                assert!(ready_tx.send(completion).is_ok());
                let _ = stop_rx.await;
            });
        });
        let completion = ready_rx.await.unwrap();
        script.wait_for_requests(1).await;
        stop_tx.send(()).unwrap();
        runtime.join().unwrap();
        tokio::time::timeout(CLEANUP + Duration::from_secs(2), async {
            while !completion.finished.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(completion.confirmed.load(Ordering::Acquire));
        assert_eq!(script.conversations()[0].len(), 1);
    }

    #[test]
    fn stderr_tail_is_bounded_and_terminal_safe() {
        let mut tail = StderrTail::default();
        tail.observe(9_000);
        for _ in 0..1_000 {
            tail.push(b"safe\x1b[31m\n");
        }
        let diagnostic = tail.diagnostic().unwrap();
        assert!(diagnostic.contains("9000 bytes observed"));
        assert!(diagnostic.contains("\\x1b"));
        assert!(!diagnostic.contains('\x1b'));
        assert!(tail.retained <= STDERR_LIMIT);
    }
}
