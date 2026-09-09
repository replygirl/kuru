use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

use crate::{MAX_BYTES, http::rpc_result};

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
    pub fn spawn(
        program: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<Self> {
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().with_context(|| {
            format!("cannot start {program}; install it or change the configured command")
        })?;
        let input = child.stdin.take().context("missing subprocess stdin")?;
        let output = BufReader::new(child.stdout.take().context("missing subprocess stdout")?);
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
                bail!("JSON-RPC {method} timed out");
            }
        }
    }

    pub async fn close(&mut self) {
        self.input.take();
        if timeout(Duration::from_millis(300), self.child.wait())
            .await
            .is_err()
        {
            self.terminate();
            let _ = self.child.wait().await;
        }
    }

    fn terminate(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.process_group.take() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let _ = self.child.start_kill();
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
    use crate::{IO_TIMEOUT, test_support::Script};

    #[tokio::test]
    async fn rpc_handles_pings_rejects_unsolicited_actions_and_preserves_notifications() {
        let script = Script::new(
            r#"
import json,sys
q=json.loads(sys.stdin.readline())
for m in ['ping','dangerous']:
 print(json.dumps({'id':900,'method':m}),flush=True)
 r=json.loads(sys.stdin.readline()); assert ('result' in r) if m=='ping' else ('error' in r)
print(json.dumps({'method':'notice','params':{'ready':True}}),flush=True)
print(json.dumps({'id':q['id'],'result':{'answer':42}}),flush=True)
for line in sys.stdin: pass
"#,
        );
        let mut rpc = Rpc::spawn(script.command(), &[], &BTreeMap::new(), Path::new(".")).unwrap();
        assert_eq!(
            rpc.request("test", json!({}), IO_TIMEOUT).await.unwrap()["answer"],
            42
        );
        assert_eq!(rpc.read().await.unwrap()["method"], "notice");
        rpc.close().await;
        assert!(rpc.send(json!({})).await.is_err());
    }

    #[tokio::test]
    async fn rpc_bounds_timeout_closed_peer_malformed_and_oversized_messages() {
        let scripts = [
            ("import time\ntime.sleep(5)", "timed out"),
            ("pass", "closed"),
            ("print('not JSON',flush=True)", "invalid JSON"),
            ("print('x'*2097153,flush=True)", "size limit"),
            (
                "import json\nprint(json.dumps({'id':999,'result':{}}),flush=True)",
                "unexpected",
            ),
        ];
        for (source, expected) in scripts {
            let script = Script::new(source);
            let mut rpc =
                Rpc::spawn(script.command(), &[], &BTreeMap::new(), Path::new(".")).unwrap();
            let error = rpc
                .request("test", json!({}), Duration::from_millis(200))
                .await
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            rpc.close().await;
        }
        let script = Script::new("import time\ntime.sleep(5)");
        let mut rpc = Rpc::spawn(script.command(), &[], &BTreeMap::new(), Path::new(".")).unwrap();
        assert!(
            rpc.send(json!({"text":"x".repeat(MAX_BYTES)}))
                .await
                .unwrap_err()
                .to_string()
                .contains("size limit")
        );
        rpc.close().await;
    }
}
