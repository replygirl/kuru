use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    time::timeout,
};

use crate::{MAX_BYTES, http::rpc_result};

#[cfg(windows)]
use kuru_platform::windows::{pipe::Pipe, process::NativeChild};
#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
#[cfg(windows)]
type Child = NativeChild;
#[cfg(windows)]
type ChildStdin = Pipe;
#[cfg(windows)]
type ChildStdout = Pipe;

/// One serial JSON-RPC connection. Independent actors use independent Codex
/// connections; MCP sessions serialize their own calls under a mutex.
pub(crate) struct Rpc {
    child: Child,
    #[cfg(unix)]
    process_group: Option<u32>,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    next_id: u64,
    events: VecDeque<Value>,
}

impl Rpc {
    pub async fn spawn(
        program: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<Self> {
        #[cfg(unix)]
        let mut child = {
            let mut command = Command::new(program);
            command
                .args(args)
                .envs(env)
                .current_dir(cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            command.process_group(0);
            command.spawn().with_context(|| {
                format!("cannot start {program}; install it or change the configured command")
            })?
        };
        #[cfg(windows)]
        let mut child = crate::process::piped(program, args, env, cwd, false).await?;
        #[cfg(unix)]
        let input = child.stdin.take().context("missing subprocess stdin")?;
        #[cfg(windows)]
        let input = child.take_stdin().context("missing subprocess stdin")?;
        #[cfg(unix)]
        let output = BufReader::new(child.stdout.take().context("missing subprocess stdout")?);
        #[cfg(windows)]
        let output = BufReader::new(child.take_stdout().context("missing subprocess stdout")?);
        Ok(Self {
            #[cfg(unix)]
            process_group: child.id(),
            child,
            input: Some(input),
            output,
            next_id: 0,
            events: VecDeque::new(),
        })
    }

    pub async fn send(&mut self, message: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&message)?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "JSON-RPC request exceeds size limit"
        );
        bytes.push(b'\n');
        let input = self
            .input
            .as_mut()
            .context("JSON-RPC connection is closed")?;
        input.write_all(&bytes).await?;
        input.flush().await?;
        Ok(())
    }

    pub async fn read(&mut self) -> Result<Value> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        self.read_wire().await
    }

    async fn read_wire(&mut self) -> Result<Value> {
        let mut bytes = Vec::new();
        // take() limits allocation even if a peer never sends a newline.
        let count = (&mut self.output)
            .take((MAX_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await?;
        ensure!(count > 0, "JSON-RPC subprocess closed its output");
        ensure!(count <= MAX_BYTES, "JSON-RPC response exceeds size limit");
        serde_json::from_slice(&bytes).context("invalid JSON-RPC subprocess output")
    }

    pub async fn reject_request(&mut self, message: &Value) -> Result<bool> {
        if message.get("method").is_some() && message.get("id").is_some() {
            let result = if message["method"] == "ping" {
                json!({"jsonrpc":"2.0", "id":message["id"], "result":{}})
            } else {
                json!({"jsonrpc":"2.0", "id":message["id"], "error":{"code":-32601,"message":"Kuru does not authorize server-initiated tools or approvals"}})
            };
            self.send(result).await?;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        duration: Duration,
    ) -> Result<Value> {
        self.next_id += 1;
        let id = json!(self.next_id);
        let operation = async {
            self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
                .await?;
            loop {
                let message = self.read_wire().await?;
                if self.reject_request(&message).await? {
                    continue;
                }
                if message.get("id") == Some(&id) {
                    return rpc_result(message, &id);
                }
                ensure!(
                    message.get("id").is_none(),
                    "unexpected JSON-RPC response ID"
                );
                ensure!(
                    self.events.len() < 1024,
                    "JSON-RPC notification queue limit exceeded"
                );
                self.events.push_back(message);
            }
        };
        match timeout(duration, operation).await {
            Ok(result) => result,
            Err(_) => {
                self.terminate();
                // Observe owned-tree and pending-pipe cleanup before returning
                // the operation timeout. Drop remains the cancellation owner.
                let _ = self.close().await;
                bail!("JSON-RPC {method} timed out");
            }
        }
    }

    pub async fn close(&mut self) -> Result<()> {
        #[cfg(windows)]
        {
            if let Some(mut input) = self.input.take() {
                input.close(Duration::from_secs(5)).await?;
            }
            if self.child.wait(Duration::from_millis(300)).await.is_err() {
                self.child.terminate()?;
            }
            self.output.get_mut().close(Duration::from_secs(5)).await?;
            self.child.wait(Duration::from_secs(5)).await?;
        }
        #[cfg(unix)]
        {
            self.input.take();
            if timeout(Duration::from_millis(300), self.child.wait())
                .await
                .is_err()
            {
                self.terminate();
                timeout(Duration::from_secs(5), self.child.wait())
                    .await
                    .context("JSON-RPC child did not exit after termination")??;
            }
        }
        Ok(())
    }

    fn terminate(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.process_group.take() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        #[cfg(unix)]
        let _ = self.child.start_kill();
        #[cfg(windows)]
        let _ = self.child.terminate();
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IO_TIMEOUT,
        test_support::{StdioFixture, Step},
    };

    #[tokio::test]
    async fn rpc_handles_pings_rejects_unsolicited_actions_and_preserves_notifications() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(json!({"id":900,"method":"ping"})),
            Step::Read,
            Step::Write(json!({"id":900,"method":"dangerous"})),
            Step::Read,
            Step::Write(json!({"method":"notice","params":{"ready":true}})),
            Step::Write(json!({"id":1,"result":{"answer":42}})),
            Step::Eof,
        ]);
        let mut rpc = Rpc::spawn(script.command(), &[], &BTreeMap::new(), Path::new("."))
            .await
            .unwrap();
        assert_eq!(
            rpc.request("test", json!({}), IO_TIMEOUT).await.unwrap()["answer"],
            42
        );
        assert_eq!(rpc.read().await.unwrap()["method"], "notice");
        rpc.close().await.unwrap();
        assert!(rpc.send(json!({})).await.is_err());
        script.assert_completed(1);
        let requests = script.conversations().remove(0);
        assert_eq!(requests[0]["method"], "test");
        assert_eq!(requests[1], json!({"jsonrpc":"2.0","id":900,"result":{}}));
        assert_eq!(requests[2]["id"], 900);
        assert_eq!(requests[2]["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn rpc_bounds_timeout_closed_peer_malformed_and_oversized_messages() {
        let scripts = [
            (vec![Step::Read, Step::Sleep(5000)], "timed out"),
            (vec![Step::Read], "closed"),
            (vec![Step::Read, Step::Raw("not JSON")], "invalid JSON"),
            (vec![Step::Read, Step::Repeat(MAX_BYTES + 1)], "size limit"),
            (
                vec![Step::Read, Step::Write(json!({"id":999,"result":{}}))],
                "unexpected",
            ),
        ];
        for (steps, expected) in scripts {
            let script = StdioFixture::new(
                [Step::Write(json!({"method":"ready"}))]
                    .into_iter()
                    .chain(steps),
            );
            let mut rpc = Rpc::spawn(script.command(), &[], &BTreeMap::new(), Path::new("."))
                .await
                .unwrap();
            assert_eq!(
                tokio::time::timeout(IO_TIMEOUT, rpc.read())
                    .await
                    .unwrap()
                    .unwrap()["method"],
                "ready"
            );
            // Only the deliberately stalled peer tests the short timeout.
            // Malformed/oversized/closed replies test distinct failures and
            // must allow normal process scheduling on loaded CI runners.
            let deadline = if expected == "timed out" {
                Duration::from_millis(200)
            } else {
                IO_TIMEOUT
            };
            let error = rpc.request("test", json!({}), deadline).await.unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            rpc.close().await.unwrap();
        }
        let script = StdioFixture::new([Step::Sleep(5000)]);
        let mut rpc = Rpc::spawn(script.command(), &[], &BTreeMap::new(), Path::new("."))
            .await
            .unwrap();
        assert!(
            rpc.send(json!({"text":"x".repeat(MAX_BYTES)}))
                .await
                .unwrap_err()
                .to_string()
                .contains("size limit")
        );
        rpc.close().await.unwrap();
    }
}
