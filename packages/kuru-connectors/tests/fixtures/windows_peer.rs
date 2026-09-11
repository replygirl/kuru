#![forbid(unsafe_code)]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use kuru_platform::windows::process::{NativeSpawnSpec, StandardStream, inherited_stdio};
    use serde_json::{Value, json};
    use std::{
        io::{BufRead, Write},
        os::windows::ffi::OsStrExt,
        time::Duration,
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.first().and_then(|arg| arg.to_str()) {
        Some("argv") => {
            let values: Vec<Vec<u16>> = args[1..]
                .iter()
                .map(|arg| arg.encode_wide().collect())
                .collect();
            println!(
                "{}",
                json!({"args": values, "marker": std::env::var("KURU_NATIVE_TEST_MARKER").ok(), "cwd": std::env::current_dir()?})
            );
        }
        Some("park") => tokio::time::sleep(Duration::from_secs(120)).await,
        Some("mcp") | Some("mcp-tree") => {
            let _descendant = if args[0] == "mcp-tree" {
                let mut spec =
                    NativeSpawnSpec::new(std::env::current_exe()?, std::env::current_dir()?);
                spec.args.push("park".into());
                spec.environment = std::env::vars_os().collect();
                spec.stdout = inherited_stdio(StandardStream::Output)?;
                spec.stderr = inherited_stdio(StandardStream::Error)?;
                Some(spec.spawn().await?)
            } else {
                None
            };
            for line in std::io::stdin().lock().lines() {
                let message: Value = serde_json::from_str(&line?)?;
                let Some(id) = message.get("id") else {
                    continue;
                };
                let result = match message["method"].as_str() {
                    Some("initialize") => {
                        json!({"protocolVersion":"2025-11-25", "capabilities":{"tools":{}}})
                    }
                    Some("tools/list") => {
                        json!({"tools":[{"name":"echo", "inputSchema":{"type":"object"}}]})
                    }
                    Some("tools/call") => {
                        json!({"content":[{"type":"text","text":message["params"]["arguments"].to_string()}],"isError":false})
                    }
                    _ => return Err("unexpected fixture method".into()),
                };
                println!("{}", json!({"jsonrpc":"2.0", "id":id, "result":result}));
                std::io::stdout().flush()?;
            }
            // The real descendant keeps the output pipe open after stdin EOF.
            // Rpc::close must terminate the entire outer Job to finish.
            if _descendant.is_some() {
                tokio::time::sleep(Duration::from_secs(120)).await;
            }
        }
        _ => return Err("unknown Windows peer mode".into()),
    }
    Ok(())
}
