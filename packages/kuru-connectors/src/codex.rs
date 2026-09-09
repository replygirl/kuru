use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use kuru_core::{Completion, CompletionRequest, ModelInfo, ToolCall};
use serde_json::{Value, json};
use tokio::{process::Command, time::timeout};

use crate::{IO_TIMEOUT, Provider, rpc::Rpc};

/// Uses the installed, authenticated Codex app-server. Each request is a fresh
/// ephemeral thread with restricted read access and no native tool authority.
pub struct CodexProvider {
    command: String,
}

impl CodexProvider {
    pub fn new(command: &str) -> Self {
        Self {
            command: command.into(),
        }
    }

    async fn connect(&self, cwd: &Path) -> Result<Rpc> {
        let mut args = vec!["app-server".into(), "--stdio".into()];
        for (key, value) in isolation_config(cwd)
            .as_object()
            .context("invalid isolation config")?
        {
            // JSON booleans/strings are valid TOML scalar values. Object values
            // are applied through thread/start's config map instead.
            if (value.is_boolean() || value.is_string())
                && !key.starts_with("permissions.")
                && key != "default_permissions"
            {
                args.extend(["-c".into(), format!("{key}={value}")]);
            }
        }
        args.extend(["-c".into(), "mcp_servers={}".into()]);
        let mut rpc = Rpc::spawn(&self.command, &args, &BTreeMap::new(), cwd)?;
        rpc.request("initialize", json!({"clientInfo":{"name":"kuru","title":"Kuru peer harness","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}), IO_TIMEOUT).await?;
        rpc.send(json!({"method":"initialized","params":{}}))
            .await?;
        Ok(rpc)
    }
}

fn isolation_config(cwd: &Path) -> Value {
    json!({
        "sqlite_home":cwd,
        "features.shell_tool":false,"features.unified_exec":false,
        "features.shell_snapshot":false,"features.multi_agent":false,
        "features.multi_agent_v2":false,"agents.enabled":false,
        "features.apps":false,"features.plugins":false,"features.hooks":false,
        "features.memories":false,"features.browser_use":false,
        "features.browser_use_external":false,"features.computer_use":false,
        "features.image_generation":false,"features.code_mode":false,
        "features.code_mode_host":false,"features.code_mode_only":false,
        "features.view_image":false,"tools.view_image":false,
        "features.goals":false,"features.sleep_tool":false,
        "features.tool_suggest":false,"features.skill_search":false,
        "features.skip_host_skill_discovery":true,
        "web_search":"disabled","allow_login_shell":false,
        "default_permissions":"kuru-inference",
        "permissions.kuru-inference.filesystem":{":minimal":"read",cwd.to_string_lossy().as_ref():"read"},
        "permissions.kuru-inference.network.enabled":false
    })
}

fn output_schema() -> Value {
    json!({"type":"object","properties":{"text":{"type":"string"},"calls":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"arguments_json":{"type":"string"}},"required":["name","arguments_json"],"additionalProperties":false}}},"required":["text","calls"],"additionalProperties":false})
}

fn parse_output(text: &str, input_tokens: u64, output_tokens: u64) -> Result<Completion> {
    let value: Value =
        serde_json::from_str(text).context("Codex did not return Kuru's structured completion")?;
    let text = value["text"]
        .as_str()
        .context("Codex structured output lacks text")?
        .to_owned();
    let calls = value["calls"]
        .as_array()
        .context("Codex structured output lacks calls")?
        .iter()
        .map(|call| {
            Ok(ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: call["name"]
                    .as_str()
                    .context("structured call lacks name")?
                    .into(),
                arguments: serde_json::from_str(
                    call["arguments_json"]
                        .as_str()
                        .context("structured call lacks arguments_json")?,
                )
                .context("invalid structured tool arguments")?,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Completion {
        text,
        calls,
        input_tokens,
        output_tokens,
    })
}

#[async_trait]
impl Provider for CodexProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        let temp = tempfile::tempdir()?;
        let mut rpc = self.connect(temp.path()).await?;
        let mut cursor = Value::Null;
        let mut seen = BTreeSet::new();
        let mut models = Vec::new();
        loop {
            let page = rpc
                .request(
                    "model/list",
                    json!({"includeHidden":true,"limit":100,"cursor":cursor}),
                    IO_TIMEOUT,
                )
                .await?;
            for item in page["data"]
                .as_array()
                .context("Codex model list lacks data")?
            {
                let id = item["model"]
                    .as_str()
                    .or_else(|| item["id"].as_str())
                    .context("Codex model lacks ID")?;
                let efforts = item["supportedReasoningEfforts"]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| {
                                value["reasoningEffort"].as_str().map(str::to_owned)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let model = ModelInfo {
                    id: id.into(),
                    name: item["displayName"].as_str().unwrap_or(id).into(),
                    efforts,
                    default_effort: item["defaultReasoningEffort"].as_str().map(str::to_owned),
                };
                if item["isDefault"] == true {
                    models.insert(0, model);
                } else {
                    models.push(model);
                }
            }
            cursor = page["nextCursor"].clone();
            if cursor.is_null() || cursor == "" {
                break;
            }
            ensure!(
                seen.len() < 100 && seen.insert(cursor.to_string()),
                "Codex model pagination did not advance"
            );
        }
        rpc.close().await;
        Ok(models)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let temp = tempfile::tempdir()?;
        let mut rpc = self.connect(temp.path()).await?;
        let mut config = isolation_config(temp.path());
        let effective = rpc
            .request("config/read", json!({"includeLayers":false}), IO_TIMEOUT)
            .await?;
        if let Some(servers) = effective["config"]["mcp_servers"].as_object() {
            for name in servers.keys() {
                config[format!("mcp_servers.{name}.enabled")] = json!(false);
            }
        }
        let instructions = format!(
            "{}\n\nKuru owns all tools and persistent memory. Return ONLY the required JSON object with text and calls. To request an advertised Kuru tool, add {{\"name\":\"tool_name\",\"arguments_json\":\"JSON object encoded as a string\"}} to calls. Never invoke native Codex tools. Do not read files, inspect environment, use MCP, or spawn agents. If text itself needs a structured format, encode that response as the text string.\nAdvertised Kuru tools: {}",
            request.instructions,
            serde_json::to_string(&request.tools)?
        );
        let model = if request.model == "auto" {
            Value::Null
        } else {
            json!(request.model)
        };
        let thread = rpc.request("thread/start", json!({"cwd":temp.path(),"ephemeral":true,"permissions":"kuru-inference","approvalPolicy":"never","model":model,"baseInstructions":instructions,"developerInstructions":"The supplied actor context is complete. Use only it. Native tools are disabled.","environments":[],"dynamicTools":[],"config":config}), IO_TIMEOUT).await?;
        let thread_id = thread["thread"]["id"]
            .as_str()
            .context("Codex thread/start lacks thread ID")?
            .to_owned();
        let input =
            serde_json::to_string(&json!({"actor":request.actor,"messages":request.messages}))?;
        rpc.request("turn/start", json!({"threadId":thread_id,"input":[{"type":"text","text":input}],"effort":request.effort,"outputSchema":output_schema(),"environments":[]}), IO_TIMEOUT).await?;
        let operation = async {
            let mut final_text = String::new();
            let mut tokens = (0, 0);
            loop {
                let event = rpc.read().await?;
                if rpc.reject_request(&event).await? {
                    continue;
                }
                let params = &event["params"];
                if params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id != thread_id)
                {
                    continue;
                }
                match event["method"].as_str() {
                    Some("item/completed") if params["item"]["type"] == "agentMessage" => {
                        if let Some(text) = params["item"]["text"].as_str() {
                            final_text = text.to_owned();
                        }
                    }
                    Some("thread/tokenUsage/updated") => {
                        tokens = (
                            params["tokenUsage"]["total"]["inputTokens"]
                                .as_u64()
                                .unwrap_or(0),
                            params["tokenUsage"]["total"]["outputTokens"]
                                .as_u64()
                                .unwrap_or(0),
                        );
                    }
                    Some("turn/completed") => {
                        let turn = &params["turn"];
                        ensure!(
                            turn["status"] == "completed",
                            "Codex turn {}: {}",
                            turn["status"],
                            turn["error"]["message"]
                                .as_str()
                                .unwrap_or("turn failed or was interrupted")
                        );
                        if final_text.is_empty()
                            && let Some(items) = turn["items"].as_array()
                        {
                            for item in items {
                                if item["type"] == "agentMessage" {
                                    final_text = item["text"].as_str().unwrap_or("").into();
                                }
                            }
                        }
                        return parse_output(&final_text, tokens.0, tokens.1);
                    }
                    Some("error") if params["willRetry"] != true => bail!(
                        "Codex inference error: {}",
                        params["error"]["message"]
                            .as_str()
                            .unwrap_or("unknown error")
                    ),
                    _ => {}
                }
            }
        };
        let result = timeout(Duration::from_secs(600), operation)
            .await
            .context("Codex inference timed out")?;
        rpc.close().await;
        result
    }
}

/// Delegate authentication to the supported Codex CLI; never read token stores.
pub async fn auth(command: &str, action: &str) -> Result<()> {
    let args: &[&str] = match action {
        "login" => &["login"],
        "logout" => &["logout"],
        "status" => &["login", "status"],
        other => bail!("unknown authentication action: {other}"),
    };
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("cannot start Codex authentication; install codex first")?;
    let status = timeout(Duration::from_secs(600), child.wait())
        .await
        .context("Codex authentication timed out")??;
    ensure!(
        status.success(),
        "Codex authentication exited with {status}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Script, request};

    const SERVER: &str = r#"
import json, sys
def send(x): print(json.dumps(x),flush=True)
for line in sys.stdin:
 q=json.loads(line); m=q.get('method'); p=q.get('params',{}); i=q.get('id')
 if m=='initialize':
  assert p['capabilities']['experimentalApi']
  r={'userAgent':'fixture'}
 elif m=='initialized': continue
 elif m=='config/read': r={'config':{'mcp_servers':{'unsafe':{'enabled':True}}}}
 elif m=='model/list':
  assert p['includeHidden']
  if p['cursor'] is None:
   r={'data':[{'model':'future-hidden','displayName':'Future Hidden','supportedReasoningEfforts':[{'reasoningEffort':'ultra-new'}],'defaultReasoningEffort':'ultra-new'}],'nextCursor':'next'}
  else:
   r={'data':[{'id':'preferred','isDefault':True}], 'nextCursor':None}
 elif m=='thread/start':
  assert p['ephemeral'] and p['approvalPolicy']=='never'
  assert p['permissions']=='kuru-inference' and 'sandbox' not in p
  assert p['config']['features.shell_tool']==False and p['config']['agents.enabled']==False
  assert p['config']['mcp_servers.unsafe.enabled']==False
  assert p['environments']==[] and p['dynamicTools']==[]
  r={'thread':{'id':'thread-a'}}
 elif m=='turn/start':
  assert p['effort']=='future-effort'
  assert p['outputSchema']['additionalProperties']==False
  assert 'Actor-specific' not in p['input'][0]['text']
  send({'method':'item/completed','params':{'threadId':'other','item':{'type':'agentMessage','text':'wrong actor'}}})
  send({'id':900,'method':'item/tool/call','params':{'name':'shell'}})
  reply=json.loads(sys.stdin.readline()); assert reply['id']==900 and 'error' in reply
  send({'method':'thread/tokenUsage/updated','params':{'threadId':'thread-a','tokenUsage':{'total':{'inputTokens':7,'outputTokens':3}}}})
  send({'method':'item/completed','params':{'threadId':'thread-a','item':{'type':'agentMessage','text':json.dumps({'text':'structured answer','calls':[{'name':'file_read','arguments_json':'{"path":"test.txt"}'}]})}}})
  send({'method':'turn/completed','params':{'threadId':'thread-a','turn':{'status':'completed','items':[]}}})
  r={'turn':{'id':'turn-a','status':'inProgress'}}
 else: raise Exception('unexpected method '+str(m))
 send({'id':i,'result':r})
"#;

    #[tokio::test]
    async fn app_server_structured_completion_enforces_isolation_and_buffers_early_events() {
        let script = Script::new(SERVER);
        let provider = CodexProvider::new(script.command());
        let result = provider.complete(request()).await.unwrap();
        assert_eq!(result.text, "structured answer");
        assert_eq!(result.calls[0].name, "file_read");
        assert_eq!(result.calls[0].arguments["path"], "test.txt");
        assert_eq!((result.input_tokens, result.output_tokens), (7, 3));
        let (one, two) = tokio::join!(provider.complete(request()), provider.complete(request()));
        assert!(one.is_ok() && two.is_ok());
        assert_ne!(one.unwrap().calls[0].id, two.unwrap().calls[0].id);
    }

    #[tokio::test]
    async fn model_catalog_paginates_hidden_models_and_preserves_new_efforts() {
        let script = Script::new(SERVER);
        let provider = CodexProvider::new(script.command());
        let models = provider.models().await.unwrap();
        assert_eq!(models[0].id, "preferred");
        assert_eq!(models[1].efforts, vec!["ultra-new"]);
        assert_eq!(models[1].default_effort.as_deref(), Some("ultra-new"));
        let script = Script::new(&SERVER.replace("'nextCursor':None", "'nextCursor':'next'"));
        assert!(
            CodexProvider::new(script.command())
                .models()
                .await
                .unwrap_err()
                .to_string()
                .contains("pagination")
        );
    }

    #[tokio::test]
    async fn app_server_errors_and_completed_turn_item_fallback() {
        let script = Script::new(&SERVER.replace(
            "'status':'completed','items':[]",
            "'status':'failed','items':[], 'error':{'message':'offline'}",
        ));
        assert!(
            CodexProvider::new(script.command())
                .complete(request())
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
        let fallback = SERVER.replace("send({'method':'item/completed','params':{'threadId':'thread-a','item':{'type':'agentMessage','text':json.dumps({'text':'structured answer','calls':[{'name':'file_read','arguments_json':'{\"path\":\"test.txt\"}'}]})}}})", "pass").replace("'status':'completed','items':[]", "'status':'completed','items':[{'type':'agentMessage','text':'{\"text\":\"fallback\",\"calls\":[]}' }]");
        let script = Script::new(&fallback);
        assert_eq!(
            CodexProvider::new(script.command())
                .complete(request())
                .await
                .unwrap()
                .text,
            "fallback"
        );
        assert!(
            CodexProvider::new("/not-installed/codex")
                .models()
                .await
                .is_err()
        );
    }

    #[test]
    fn structured_output_rejects_invalid_tool_requests() {
        for text in [
            "not JSON",
            "{}",
            r#"{"text":"a","calls":[{}]}"#,
            r#"{"text":"a","calls":[{"name":"x","arguments_json":"not JSON"}]}"#,
        ] {
            assert!(parse_output(text, 0, 0).is_err());
        }
    }

    #[tokio::test]
    async fn auth_delegates_only_supported_native_commands() {
        let script = Script::new(
            "import sys\nassert sys.argv[1:] in [['login'],['logout'],['login','status']]\n",
        );
        for action in ["login", "logout", "status"] {
            auth(script.command(), action).await.unwrap();
        }
        assert!(auth(script.command(), "tokens").await.is_err());
        assert!(auth("/not-installed/codex", "login").await.is_err());
        let failed = Script::new("import sys\nsys.exit(1)");
        assert!(auth(failed.command(), "login").await.is_err());
    }
}
