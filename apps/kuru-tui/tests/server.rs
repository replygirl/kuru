use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

#[test]
fn authenticated_a2a_cli_routes_a_part_and_shuts_down_cleanly() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("workspace");
    let data = root.path().join("data");
    std::fs::create_dir(&project).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(["-C"])
        .arg(&project)
        .arg("--data-dir")
        .arg(&data)
        .args([
            "--provider",
            "demo",
            "--mode",
            "freudian",
            "--no-dream",
            "serve",
            "--bind",
            "127.0.0.1:0",
        ])
        .env("KURU_A2A_TOKEN", "integration-token-123456")
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    stderr.read_line(&mut line).unwrap();
    assert!(line.contains("listening"), "{line}");
    let address = line.split_whitespace().last().unwrap();
    let output=Command::new("python3").arg("-c").arg(r#"
import json, sys, urllib.request, urllib.error
base = 'http://' + sys.argv[1]
with urllib.request.urlopen(base + '/.well-known/agent-card.json') as response:
    card = json.load(response)
assert card['supportedInterfaces'][0]['url'] == base
payload = {'jsonrpc':'2.0','id':1,'method':'SendMessage','params':{'message':{'messageId':'external-test','role':'ROLE_USER','parts':[{'text':'A2A e2e message'}]}}}
try:
    urllib.request.urlopen(urllib.request.Request(base+'/', data=json.dumps(payload).encode(), headers={'Content-Type':'application/json'}))
except urllib.error.HTTPError as error:
    assert error.code == 401
else:
    raise AssertionError('missing authentication accepted')
request = urllib.request.Request(base+'/agents/ego', data=json.dumps(payload).encode(), headers={'Content-Type':'application/json','Authorization':'Bearer integration-token-123456','A2A-Version':'1.0'})
with urllib.request.urlopen(request) as response:
    result = json.load(response)
assert result['result']['message']['role'] == 'ROLE_AGENT', result
assert 'demo' in result['result']['message']['parts'][0]['text']
with urllib.request.urlopen(base+'/agents/ego/.well-known/agent-card.json') as response:
    card = json.load(response)
assert card['supportedInterfaces'][0]['url'].startswith(base+'/agents/')
"#).arg(address).output().unwrap();
    #[cfg(unix)]
    let _ = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status();
    #[cfg(not(unix))]
    let _ = child.kill();
    let status = child.wait().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(status.success(), "server failed graceful shutdown");
}
