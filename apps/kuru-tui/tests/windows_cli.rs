#![cfg(windows)]

use base64::Engine;
use kuru_delivery::command::BlockingCommand as Command;
use kuru_platform::fs::regular_file_info;
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    path::Path,
};

fn fixture_environment(root: &Path) -> Vec<(OsString, OsString)> {
    let mut environment: Vec<_> = [
        ("USERPROFILE", root.join("home")),
        ("APPDATA", root.join("config")),
        ("LOCALAPPDATA", root.join("local")),
        ("TMP", root.to_owned()),
        ("TEMP", root.to_owned()),
        ("PATH", root.join("tools")),
        ("KURU_CLI_FIXTURE_LOG", root.join("actions.jsonl")),
    ]
    .into_iter()
    .map(|(key, value)| (OsString::from(key), value.into_os_string()))
    .collect();
    // Match the machine-scoped inputs that the production built-in shell
    // deliberately preserves. The fixture still owns every user/cache path.
    for key in [
        "SystemRoot",
        "ProgramData",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_ARCHITEW6432",
        "LLVM_PROFILE_FILE",
    ] {
        if let Some(value) = std::env::var_os(key) {
            environment.push((key.into(), value));
        }
    }
    environment
}

fn command(root: &Path, binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .current_dir(root)
        .envs(fixture_environment(root));
    command
}

fn shell_marker(path: &Path, limit: u64) -> std::io::Result<String> {
    use std::io::Read;

    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other("shell marker exceeds its byte limit"));
    }
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("shell marker is not UTF-8"))
}

fn machine_environment_diagnostic() -> String {
    let mut fields = Vec::new();
    for key in [
        "ProgramData",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
    ] {
        let state = std::env::var_os(key).map_or("missing", |value| {
            if Path::new(&value).is_absolute() {
                "absolute"
            } else {
                "relative"
            }
        });
        fields.push(format!("{key}={state}"));
    }
    for key in ["PROCESSOR_ARCHITECTURE", "PROCESSOR_ARCHITEW6432"] {
        fields.push(format!(
            "{key}={}",
            if std::env::var_os(key).is_some() {
                "present"
            } else {
                "missing"
            }
        ));
    }
    fields.join(",")
}

fn powershell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// Windows PowerShell 5.1 engine/host cold start (module/format data load, first
// runspace build) can stall well past a single `tool shell` call's own budget
// before any script line runs. Warm the exact ToolHost stock-shell launch path
// once per test-binary process, ahead of the first real `tool shell` command,
// so that stall lands here (with its own distinct, generous bound) instead of
// inside a test's real assertion window. See `kuru_connectors::shell_warmup`.
fn ensure_powershell_warm() {
    // Some callers in this file are `#[tokio::test]` async tests already
    // driving a Tokio runtime (default `current_thread` flavor); the shared
    // helper is safe from either a plain thread or an existing runtime of
    // any flavor (see
    // `kuru_connectors::shell_warmup::block_on_dedicated_thread`).
    kuru_connectors::shell_warmup::ensure_stock_powershell_warm();
}

fn stock_shell_source(
    progress: &Path,
    input: &Path,
    sentinel: &str,
    diagnostic_import: Option<&str>,
) -> String {
    let progress = powershell_literal(&progress.to_string_lossy());
    let input = powershell_literal(&input.to_string_lossy());
    let sentinel = powershell_literal(sentinel);
    let diagnostic_import = diagnostic_import.map_or_else(String::new, |module| {
        format!(
        r#"[IO.File]::AppendAllText({progress}, "before-import`nimport-verbose=")
$importCapacity = 1024
$importMarker = '[truncated]'
$importTextCapacity = $importCapacity - $importMarker.Length
$importWritten = 0
$importTruncated = $false
try {{
    Microsoft.PowerShell.Core\Import-Module -Name '{module}' -Verbose -ErrorAction Stop 4>&1 | Microsoft.PowerShell.Core\ForEach-Object {{
        if (-not $importTruncated) {{
            $importRecord = $_.ToString()
            $importRemaining = $importTextCapacity - $importWritten
            if ($importRemaining -le 0) {{
                [IO.File]::AppendAllText({progress}, $importMarker)
                $importWritten += $importMarker.Length
                $importTruncated = $true
            }} elseif ($importRecord.Length -gt $importRemaining) {{
                [IO.File]::AppendAllText({progress}, $importRecord.Substring(0, $importRemaining))
                [IO.File]::AppendAllText({progress}, $importMarker)
                $importWritten += $importRemaining + $importMarker.Length
                $importTruncated = $true
            }} else {{
                [IO.File]::AppendAllText({progress}, $importRecord)
                $importWritten += $importRecord.Length
            }}
        }}
    }}
}} catch {{
    throw
}}
[IO.File]::AppendAllText({progress}, "`n")
[IO.File]::AppendAllText({progress}, "after-import`n")
"#
        )
    });
    format!(
        r#"$ErrorActionPreference = 'Stop'
[IO.File]::WriteAllText({progress}, "entered`n")
if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) {{ throw 'expected stock PowerShell 5.1' }}
if ([string]::IsNullOrWhiteSpace($env:ProgramData) -or [string]::IsNullOrWhiteSpace($env:ProgramFiles) -or [string]::IsNullOrWhiteSpace(${{env:ProgramFiles(x86)}}) -or [string]::IsNullOrWhiteSpace($env:ProgramW6432) -or [string]::IsNullOrWhiteSpace($env:PROCESSOR_ARCHITECTURE)) {{ throw 'expected stock Windows machine environment' }}
{diagnostic_import}[IO.File]::AppendAllText({progress}, "version-checked`nmachine-environment-checked`nhash-started`n")
$hash = (Microsoft.PowerShell.Utility\Get-FileHash -LiteralPath {input} -Algorithm SHA256).Hash
[IO.File]::AppendAllText({progress}, "hashed`n")
[Console]::Write($hash + '|' + {sentinel})
[IO.File]::AppendAllText({progress}, "completed`n")
"#
    )
}

fn product_shell_environment(
    root: &Path,
    private: &Path,
    hostile_modules: &Path,
) -> std::io::Result<Vec<(OsString, OsString)>> {
    use kuru_platform::windows::process::{environment_key_eq, system_directory};

    const ALLOWED: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "USERNAME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "ProgramData",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_ARCHITEW6432",
        "TMPDIR",
        "TMP",
        "TEMP",
        "LANG",
        "LC_ALL",
        "LC_COLLATE",
        "LC_CTYPE",
        "LC_MESSAGES",
        "LC_MONETARY",
        "LC_NUMERIC",
        "LC_TIME",
        "TZ",
        "NO_COLOR",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        "PATHEXT",
    ];
    let mut supplied = fixture_environment(root);
    for (name, value) in [
        ("USERPROFILE", private.join("home")),
        ("APPDATA", private.join("appdata")),
        ("LOCALAPPDATA", private.join("localappdata")),
        ("TMP", private.join("tmp")),
        ("TEMP", private.join("temp")),
    ] {
        let (_, existing) = supplied
            .iter_mut()
            .find(|(key, _)| environment_key_eq(key, OsStr::new(name)))
            .expect("fixture supplies each private Windows user path");
        *existing = value.into_os_string();
    }
    // Supply only fake hostile/sensitive names so filtering them can be
    // asserted without observing a runner's real process environment.
    supplied.extend([
        ("pSmOdUlEpAtH".into(), hostile_modules.as_os_str().into()),
        ("OPENAI_API_KEY".into(), "sk-fixture-not-a-real-key".into()),
    ]);
    let mut projected: Vec<_> = supplied
        .into_iter()
        .filter(|(key, _)| {
            ALLOWED
                .iter()
                .any(|name| environment_key_eq(key, OsStr::new(name)))
        })
        .collect();
    assert!(
        !projected
            .iter()
            .any(|(key, _)| environment_key_eq(key, OsStr::new("PSModulePath")))
    );
    assert!(!projected.iter().any(|(key, _)| {
        environment_key_eq(key, OsStr::new("OPENAI_API_KEY"))
            || environment_key_eq(key, OsStr::new("KURU_CLI_FIXTURE_LOG"))
            || environment_key_eq(key, OsStr::new("LLVM_PROFILE_FILE"))
    }));
    for name in ["USERPROFILE", "APPDATA", "LOCALAPPDATA", "TMP", "TEMP"] {
        assert!(
            projected
                .iter()
                .any(|(key, _)| environment_key_eq(key, OsStr::new(name))),
            "product-shaped control retains {name}"
        );
    }
    if !projected
        .iter()
        .any(|(key, _)| environment_key_eq(key, OsStr::new("PATHEXT")))
    {
        projected.push(("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()));
    }
    let system = system_directory()?;
    let windows = system
        .parent()
        .ok_or_else(|| std::io::Error::other("native system directory lacks Windows parent"))?;
    projected.extend([
        ("SystemRoot".into(), windows.as_os_str().into()),
        ("WINDIR".into(), windows.as_os_str().into()),
        ("ComSpec".into(), system.join("cmd.exe").into()),
    ]);
    for name in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"] {
        assert!(
            projected
                .iter()
                .any(|(key, _)| environment_key_eq(key, OsStr::new(name))),
            "product-shaped control injects {name}"
        );
    }
    Ok(projected)
}

async fn drain_shell_control(
    pipe: &mut kuru_platform::windows::pipe::Pipe,
    tail: &mut Vec<u8>,
    total: &mut usize,
    eof: &mut bool,
) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt;

    let mut buffer = [0; 2048];
    loop {
        let length = pipe.read(&mut buffer).await?;
        if length == 0 {
            *eof = true;
            return Ok(());
        }
        *total += length;
        tail.extend_from_slice(&buffer[..length]);
        if tail.len() > 4096 {
            tail.drain(..tail.len() - 4096);
        }
        if *total > 2 * 1024 * 1024 {
            return Err(std::io::Error::other(
                "shell control exceeds 2 MiB on one stream",
            ));
        }
    }
}

// A failed acceptance remains failed. This control retains the source and
// product-shaped environment while explicitly importing the named module.
async fn shell_failure_control(
    label: &str,
    root: &Path,
    project: &Path,
    hostile_modules: &Path,
    input: &Path,
    sentinel: &str,
) -> std::io::Result<String> {
    use kuru_platform::windows::process::{Stdio, configured_command, system_directory};
    use std::{io, time::Duration};
    let private = root.join(format!("failure-control-{label}"));
    fs::create_dir(&private)?;
    for leaf in ["home", "appdata", "localappdata", "tmp", "temp"] {
        fs::create_dir(private.join(leaf))?;
    }
    let progress = private.join("progress");
    let source = format!(
        "$ProgressPreference = 'SilentlyContinue'; [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding;\n{}",
        stock_shell_source(
            &progress,
            input,
            sentinel,
            Some("Microsoft.PowerShell.Utility"),
        )
    );
    let mut args: Vec<OsString> = [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-OutputFormat",
        "Text",
        "-EncodedCommand",
    ]
    .map(Into::into)
    .into();
    let bytes: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    args.push(
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into(),
    );
    let powershell = system_directory()?.join("WindowsPowerShell/v1.0/powershell.exe");
    let environment = product_shell_environment(root, &private, hostile_modules)?;
    let mut spec = configured_command(powershell.as_os_str(), &args, project, environment)?;
    spec.console = kuru_platform::windows::process::Console::Inherit;
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    let mut child = spec.spawn().await?;
    let mut stdout = child.take_stdout();
    let mut stderr = child.take_stderr();
    let (mut stdout_tail, mut stderr_tail) = (Vec::new(), Vec::new());
    let (mut stdout_bytes, mut stderr_bytes) = (0, 0);
    let (mut stdout_eof, mut stderr_eof) = (false, false);
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let (stdout, stderr) = stdout
            .as_mut()
            .zip(stderr.as_mut())
            .ok_or_else(|| io::Error::other("shell control lacks requested output pipes"))?;
        tokio::try_join!(
            drain_shell_control(stdout, &mut stdout_tail, &mut stdout_bytes, &mut stdout_eof),
            drain_shell_control(stderr, &mut stderr_tail, &mut stderr_bytes, &mut stderr_eof),
        )?;
        child.wait(Duration::from_secs(30)).await
    })
    .await
    .unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "shell control timed out after 30 seconds",
        ))
    });
    // No early return after creation: request termination, await ownership, and
    // cancel both native pipes even if the diagnostic's operation failed.
    let terminate = child.terminate();
    let stopped = child.wait(Duration::from_secs(5)).await;
    let mut closed = Vec::new();
    for (stream, pipe) in [("stdout", &mut stdout), ("stderr", &mut stderr)] {
        if let Some(pipe) = pipe {
            closed.push((stream, pipe.close(Duration::from_secs(5)).await));
        }
    }
    Ok(format!(
        "{label}: result={result:?}; stdout_bytes={stdout_bytes} stdout_eof={stdout_eof} stdout_tail={:?}; stderr_bytes={stderr_bytes} stderr_eof={stderr_eof} stderr_tail={:?}; terminate={terminate:?} stopped={stopped:?} closed={closed:?}; stages={:?}",
        String::from_utf8_lossy(&stdout_tail),
        String::from_utf8_lossy(&stderr_tail),
        shell_marker(&progress, 4096),
    ))
}

fn shell_timeout_controls(
    root: &Path,
    project: &Path,
    hostile_modules: &Path,
    input: &Path,
    sentinel: &str,
) -> String {
    let execute = async {
        let mut controls = Vec::new();
        controls.push(
            shell_failure_control(
                "encoded-inherit-import-utility",
                root,
                project,
                hostile_modules,
                input,
                sentinel,
            )
            .await,
        );
        Ok::<_, std::io::Error>(format!("{controls:#?}"))
    };
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .and_then(|runtime| runtime.block_on(execute));
    format!("failure-only stock PowerShell controls: {result:?}")
}

fn success(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn pe_inspection_uses_native_paths_and_real_msvc_imports() {
    use std::io::Read;

    let root = kuru_memory::test_support::tempdir().unwrap();
    let tools = root.path().join("tools");
    let images = root.path().join("PE images 日本語");
    fs::create_dir(&tools).unwrap();
    fs::create_dir(&images).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_kuru-cli-windows-fixture"),
        tools.join("mise.exe"),
    )
    .unwrap();
    let config = kuru_core::MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let engine =
        kuru_memory::provision::provision(&config, &kuru_memory::test_support::cache_dir())
            .await
            .unwrap()
            .canonicalize()
            .unwrap();
    let evidence = |path: &Path| {
        let mut file = File::open(path).unwrap();
        let info = regular_file_info(&file).unwrap();
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let length = file.read(&mut buffer).unwrap();
            if length == 0 {
                break;
            }
            digest.update(&buffer[..length]);
        }
        (file, info, digest.finalize().to_vec())
    };
    let engine_before = evidence(&engine);
    // Instrumented development Kuru may use a dynamic CRT. Both fixture slots
    // instead inspect the actual verified static-runtime Dolt PE. Shipping CI
    // separately inspects the source-installed Kuru and real owning prefetch.
    let binary = images.join("copied engine.exe");
    assert_eq!(fs::copy(&engine, &binary).unwrap(), engine_before.1.len);
    let binary_before = evidence(&binary);
    assert_eq!(binary_before.2, engine_before.2);
    assert_ne!(binary_before.1.identity, engine_before.1.identity);
    let invalid = images.join("not a PE.exe");
    fs::write(&invalid, b"This is not a Portable Executable.\n").unwrap();
    let invalid_before = evidence(&invalid);

    let script = root.path().join("verify-windows-imports.ps1");
    fs::write(
        &script,
        include_bytes!("../support/verify-windows-imports.ps1"),
    )
    .unwrap();
    let launcher = root.path().join("inspect-pe.ps1");
    fs::write(
        &launcher,
        r#"$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) { throw 'expected stock PowerShell 5.1' }
# Native Rust fixture output and exact Unicode result assertions use UTF-8.
# Keep this ASCII script independent of Windows PowerShell's source encoding.
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
& $env:KURU_PE_SCRIPT -Binary $env:KURU_PE_INPUT
"#,
    )
    .unwrap();
    let powershell = kuru_platform::windows::process::system_directory()
        .unwrap()
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let program_files = std::env::var_os("ProgramFiles(x86)")
        .expect("native MSVC acceptance requires ProgramFiles(x86)");
    let launch = |input: &str| {
        let mut child = command(root.path(), &powershell);
        // This build-tool fixture uses the machine's real MSVC installation.
        // Setup Configuration discovers its shared installation state under
        // ProgramData; isolating user homes must not hide that machine root.
        for key in ["ProgramData", "ALLUSERSPROFILE"] {
            let value = std::env::var_os(key)
                .unwrap_or_else(|| panic!("native MSVC acceptance requires {key}"));
            child.env(key, value);
        }
        child
            .env("OS", "Windows_NT")
            .env("ProgramFiles(x86)", &program_files)
            .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
            .env("KURU_CLI_FIXTURE_ENGINE", &engine)
            .env("KURU_PE_SCRIPT", &script)
            .env("KURU_PE_INPUT", input)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-OutputFormat",
                "Text",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&launcher);
        child.output().unwrap()
    };
    let extended = binary.canonicalize().unwrap();
    let extended = extended.to_str().unwrap();
    // Construct alternate spellings of this same retained fixture file only;
    // production resolution preserves the genuine extended prefix unchanged.
    let ordinary = match extended.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => extended.strip_prefix(r"\\?\").unwrap().to_owned(),
    };
    let qualified = format!(r"Microsoft.PowerShell.Core\FileSystem::{extended}");
    for input in [ordinary.as_str(), extended, qualified.as_str()] {
        let output = launch(input);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "input={input:?}: {diagnostic}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<_> = stdout.lines().collect();
        assert_eq!(lines.len(), 2, "input={input:?}: {stdout:?}");
        let mut inventories = Vec::new();
        for (line, expected) in lines.into_iter().zip([&binary, &engine]) {
            let (path, libraries) = line
                .strip_prefix("OS-only PE imports: ")
                .unwrap()
                .rsplit_once(": ")
                .unwrap();
            assert!(!path.contains("FileSystem::"), "{line}");
            assert_eq!(
                Path::new(path).canonicalize().unwrap(),
                expected.canonicalize().unwrap()
            );
            assert!(!libraries.is_empty(), "{line}");
            assert!(
                libraries
                    .split(", ")
                    .all(|library| library.ends_with(".dll"))
            );
            inventories.push(libraries);
        }
        assert_eq!(inventories[0], inventories[1]);
    }
    let output = launch(invalid.to_str().unwrap());
    assert!(!output.status.success(), "non-PE input must fail");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("OS-only PE imports:"));
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("Cannot inspect PE imports")
            || diagnostic.contains("No DLL imports were parsed"),
        "non-PE input must reach actual inspection: {diagnostic}"
    );
    let calls: Vec<Vec<String>> = fs::read_to_string(root.path().join("actions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls,
        vec![vec!["run", "//packages/kuru-memory:prefetch"]; 4]
    );
    for (path, before) in [
        (&engine, &engine_before),
        (&binary, &binary_before),
        (&invalid, &invalid_before),
    ] {
        let after = evidence(path);
        assert_eq!(regular_file_info(&before.0).unwrap(), before.1);
        assert_eq!(
            after.1,
            before.1,
            "identity/size changed: {}",
            path.display()
        );
        assert_eq!(after.2, before.2, "bytes changed: {}", path.display());
    }
}

#[test]
fn built_in_shell_reconstructs_stock_module_paths_without_losing_other_environment() {
    ensure_powershell_warm();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("workspace 日本語");
    let modules = root.path().join("incompatible modules");
    let control_config = root.path().join("control config");
    let control_local = root.path().join("control local");
    for directory in [
        &project,
        &modules,
        &root.path().join("tools"),
        &control_config,
        &control_local,
    ] {
        fs::create_dir(directory).unwrap();
    }
    let incompatible = modules.join("Microsoft.PowerShell.Utility");
    fs::create_dir(&incompatible).unwrap();
    // Qualified auto-loading selects this same-name module before import checks
    // the minimum engine version. No cmdlet, script module or DLL implements
    // the command.
    fs::write(
        incompatible.join("Microsoft.PowerShell.Utility.psd1"),
        r#"@{
    ModuleVersion = '7.0.0.0'
    PowerShellVersion = '7.0'
    CmdletsToExport = @('Get-FileHash')
    FunctionsToExport = @()
    AliasesToExport = @()
}
"#,
    )
    .unwrap();
    let input = project.join("hash input 日本語.bin");
    let bytes = b"real stock PowerShell file hashing\0\xff\n";
    fs::write(&input, bytes).unwrap();
    let identity = regular_file_info(&File::open(&input).unwrap())
        .unwrap()
        .identity;
    // Keep each invocation's observations independent: the failing direct
    // control must never supply evidence that Kuru's own shell entered source.
    let control_progress = root.path().join("control-progress");
    let kuru_progress = root.path().join("kuru-progress");
    let sentinel = "retained & literal 日本語";
    let machine_environment = machine_environment_diagnostic();
    let control_source = stock_shell_source(&control_progress, &input, sentinel, None);
    let kuru_source = stock_shell_source(&kuru_progress, &input, sentinel, None);
    let child = |binary: &Path| {
        let mut child = command(root.path(), binary);
        child
            // This incompatible manifest precedes the system-module fallback
            // that lets stock 5.1 find commands through an empty module path.
            // Mixed casing exercises Windows environment-key comparison.
            .env("pSmOdUlEpAtH", &modules);
        child
    };
    let powershell = kuru_platform::windows::process::system_directory()
        .unwrap()
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let control = child(&powershell)
        // Command discovery writes a cache below LOCALAPPDATA. Keep this
        // deliberately incompatible control from changing the authoritative
        // Kuru invocation's cache before its first stock-module lookup.
        .env("APPDATA", &control_config)
        .env("LOCALAPPDATA", &control_local)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-OutputFormat",
            "Text",
            "-Command",
            &control_source,
        ])
        .output();
    let control_stages = shell_marker(&control_progress, 256);
    let control = control.unwrap_or_else(|error| {
        panic!("stock PowerShell control failed: {error}; stages={control_stages:?}; machine_environment={machine_environment}")
    });
    let diagnostic = String::from_utf8_lossy(&control.stderr);
    let preview =
        |bytes: &[u8]| String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into_owned();
    assert!(
        !control.status.success(),
        "unsanitized control must fail: status={} stdout={:?} stderr={:?} stages={control_stages:?} machine_environment={machine_environment}",
        control.status,
        preview(&control.stdout),
        preview(&control.stderr)
    );
    assert!(
        diagnostic.contains("Get-FileHash")
            && diagnostic.contains("CouldNotAutoLoadModule")
            && diagnostic.contains("Microsoft.PowerShell.Utility"),
        "control must reach the incompatible module's autoload failure: {:?}; stages={control_stages:?}; machine_environment={machine_environment}",
        preview(&control.stderr)
    );
    assert!(control.stdout.is_empty());
    assert_eq!(
        control_stages.as_deref().ok(),
        Some("entered\nversion-checked\nmachine-environment-checked\nhash-started\n"),
        "control must stop at the actual hash command: {control_stages:?}"
    );
    // The first Kuru request must exercise the unwrapped source. Observation
    // wrappers can alter discovery; a diagnostic retry cannot establish a pass.
    let arguments = serde_json::json!({ "command": kuru_source }).to_string();
    let stack_stage = root.path().join("kuru-shell-stack-stage");
    let output = child(Path::new(env!("CARGO_BIN_EXE_kuru")))
        .env("KURU_WINDOWS_SHELL_STACK_STAGE", &stack_stage)
        .arg("-C")
        .arg(&project)
        .args(["--allow-shell", "tool", "shell", "--args", &arguments])
        .output();
    let kuru_stages = shell_marker(&kuru_progress, 256);
    let stack_stages = shell_marker(&stack_stage, 512);
    let output = output.unwrap_or_else(|error| {
        panic!("Kuru shell failed: {error}; stages={kuru_stages:?}; Kuru stages={stack_stages:?}")
    });
    let trace = if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("shell timed out")
    {
        shell_timeout_controls(root.path(), &project, &modules, &input, sentinel)
    } else {
        "trace not run (no original shell timeout)".to_owned()
    };
    assert!(
        output.status.success(),
        "{}\n{}\nstages={kuru_stages:?}\nKuru stages={stack_stages:?}\nmachine_environment={machine_environment}\n{trace}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        kuru_stages.as_deref().ok(),
        Some(
            "entered\nversion-checked\nmachine-environment-checked\nhash-started\nhashed\ncompleted\n"
        ),
        "Kuru must complete its own source sequence: {kuru_stages:?}"
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["exit_code"], 0);
    assert_eq!(result["success"], true);
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect();
    assert_eq!(result["stdout"], format!("{digest}|{sentinel}"));
    assert_eq!(result["stderr"], "");
    assert_eq!(fs::read(&input).unwrap(), bytes);
    assert_eq!(
        regular_file_info(&File::open(&input).unwrap())
            .unwrap()
            .identity,
        identity
    );
}

#[test]
fn source_update_builds_through_mise_and_publishes_after_the_trusted_build() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("tools");
    let install = root.path().join("installed 日本語");
    let checkout = root.path().join("checkout & spaced 日本語");
    let target = root.path().join("target with spaces");
    for directory in [&tools, &install, &checkout] {
        fs::create_dir(directory).unwrap();
    }
    fs::copy(
        env!("CARGO_BIN_EXE_kuru-cli-windows-fixture"),
        tools.join("mise.exe"),
    )
    .unwrap();
    let binary = install.join("kuru.exe");
    fs::copy(env!("CARGO_BIN_EXE_kuru"), &binary).unwrap();
    let before = regular_file_info(&File::open(&binary).unwrap())
        .unwrap()
        .identity;
    let mut update = command(root.path(), &binary);
    update
        .env("KURU_CLI_FIXTURE_BINARY", env!("CARGO_BIN_EXE_kuru"))
        .env("KURU_CLI_FIXTURE_TARGET", &target)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_TARGET", "host")
        .env("git_dir", root.path().join("unrelated repository"))
        .env("GIT_WORK_TREE", root.path().join("unrelated work tree"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_SSH_COMMAND", "fixture-ssh-command")
        .env("GIT_CONFIG_GLOBAL", "fixture-global-config")
        .args(["update", "--source"])
        .arg(&checkout);
    success(&mut update);
    assert_ne!(
        regular_file_info(&File::open(&binary).unwrap())
            .unwrap()
            .identity,
        before
    );
    let actions: Vec<Vec<String>> = fs::read_to_string(root.path().join("actions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(actions.len(), 2);
    let source = checkout
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(actions[0], ["-C", &source, "install", "rust"]);
    assert_eq!(
        actions[1],
        [
            "-C",
            &source,
            "run",
            "//apps/kuru-tui:build:release",
            "--",
            "--target",
            "x86_64-pc-windows-msvc"
        ]
    );
    success(command(root.path(), &binary).arg("--version"));
    let output = command(root.path(), &binary)
        .args(["update", "--version", "0.2.0", "--source"])
        .arg(&checkout)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(root.path().join("actions.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2,
        "invalid invocation must not build or update"
    );
}

fn source_entrypoint_fixture(root: &Path) -> Command {
    let tools = root.join("tools");
    let scripts = root.join("checkout & spaced 日本語").join("scripts");
    fs::create_dir(&tools).unwrap();
    fs::create_dir_all(&scripts).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_kuru-cli-windows-fixture"),
        tools.join("mise.exe"),
    )
    .unwrap();
    // Execute the unmodified public entrypoint in an isolated checkout layout.
    let entrypoint = scripts.join("install.ps1");
    fs::write(&entrypoint, include_bytes!("../../../scripts/install.ps1")).unwrap();
    let probe = root.join("source-entrypoint-probe.ps1");
    fs::write(
        &probe,
        r#"$ErrorActionPreference = 'Stop'
function Snapshot {
    return @{
        no_hooks = [Environment]::GetEnvironmentVariable('MISE_NO_HOOKS', 'Process')
        auto_install = [Environment]::GetEnvironmentVariable('MISE_TASK_RUN_AUTO_INSTALL', 'Process')
        install_dir = [Environment]::GetEnvironmentVariable('KURU_INSTALL_DIR', 'Process')
    }
}
$before = Snapshot
$failure = $null
try {
    if ($env:KURU_ENTRYPOINT_INVALID) {
        & $env:KURU_ENTRYPOINT_SCRIPT -Source -Version '0.2.0' -InstallDir $env:KURU_ENTRYPOINT_INSTALL
    } else {
        & $env:KURU_ENTRYPOINT_SCRIPT -Source -InstallDir $env:KURU_ENTRYPOINT_INSTALL
    }
} catch {
    $failure = $_.Exception.Message
} finally {
    $report = @{ before = $before; after = (Snapshot); failure = $failure }
    [IO.File]::WriteAllText($env:KURU_ENTRYPOINT_REPORT, ($report | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
}
if ($null -ne $failure) { Write-Output $failure; exit 1 }
"#,
    )
    .unwrap();
    let powershell = kuru_platform::windows::process::system_directory()
        .unwrap()
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let mut child = command(root, &powershell);
    child
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .env("KURU_ENTRYPOINT_SCRIPT", &entrypoint)
        .env("KURU_ENTRYPOINT_INSTALL", ".\\installed & 日本語")
        .env("KURU_ENTRYPOINT_REPORT", root.join("restoration.json"))
        .env("KURU_CLI_FIXTURE_SETUP_LOG", root.join("setup.jsonl"))
        .env("KURU_CLI_FIXTURE_SETUP_EXIT", "0")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&probe);
    child
}

#[test]
fn source_install_entrypoint_scopes_first_mise_and_restores_environment_on_success_or_failure() {
    for opposing in [false, true] {
        for status in [0, 23] {
            let root = tempfile::tempdir().unwrap();
            let mut child = source_entrypoint_fixture(root.path());
            child.env("KURU_CLI_FIXTURE_SETUP_EXIT", status.to_string());
            let before = if opposing {
                child
                    .env("MISE_NO_HOOKS", "0")
                    .env("MISE_TASK_RUN_AUTO_INSTALL", "true")
                    .env("KURU_INSTALL_DIR", "caller relative destination 日本語");
                serde_json::json!({
                    "no_hooks": "0", "auto_install": "true",
                    "install_dir": "caller relative destination 日本語",
                })
            } else {
                serde_json::json!({"no_hooks": null, "auto_install": null, "install_dir": null})
            };
            let output = child.output().unwrap();
            let diagnostic = format!(
                "opposing={opposing}, status={status}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.status.success(), status == 0, "{diagnostic}");
            let calls: Vec<serde_json::Value> = fs::read_to_string(root.path().join("setup.jsonl"))
                .expect(&diagnostic)
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(calls.len(), 1, "{diagnostic}");
            let actual = &calls[0];
            assert_eq!(actual["no_hooks"], "1", "{diagnostic}");
            assert_eq!(actual["auto_install"], "false", "{diagnostic}");
            let arguments = actual["arguments"].as_array().unwrap();
            assert_eq!(arguments.len(), 4);
            assert_eq!(arguments[0], "-C");
            assert_eq!(
                Path::new(arguments[1].as_str().unwrap())
                    .canonicalize()
                    .unwrap(),
                root.path()
                    .join("checkout & spaced 日本語")
                    .canonicalize()
                    .unwrap()
            );
            assert_eq!(arguments[2], "run");
            assert_eq!(arguments[3], "//apps/kuru-tui:install");
            let selected = Path::new(actual["install_dir"].as_str().unwrap());
            assert!(selected.is_absolute());
            assert_eq!(selected.file_name().unwrap(), "installed & 日本語");
            assert_eq!(
                selected.parent().unwrap().canonicalize().unwrap(),
                root.path().canonicalize().unwrap()
            );
            let report: serde_json::Value = serde_json::from_slice(
                &fs::read(root.path().join("restoration.json")).expect(&diagnostic),
            )
            .unwrap();
            assert_eq!(report["before"], before, "{diagnostic}");
            assert_eq!(report["after"], before, "{diagnostic}");
            if status == 0 {
                assert!(report["failure"].is_null(), "{diagnostic}");
            } else {
                assert_eq!(report["failure"], "Source installation failed (23).");
            }
        }
    }
}

#[test]
fn source_install_entrypoint_rejects_release_options_before_mise_or_environment_changes() {
    let root = tempfile::tempdir().unwrap();
    let output = source_entrypoint_fixture(root.path())
        .env("KURU_ENTRYPOINT_INVALID", "1")
        .env("MISE_NO_HOOKS", "0")
        .env("MISE_TASK_RUN_AUTO_INSTALL", "true")
        .env("KURU_INSTALL_DIR", "caller relative destination")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("setup.jsonl").exists());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("restoration.json")).unwrap()).unwrap();
    let before = serde_json::json!({
        "no_hooks": "0", "auto_install": "true", "install_dir": "caller relative destination",
    });
    assert_eq!(report["before"], before);
    assert_eq!(report["after"], before);
    assert_eq!(
        report["failure"],
        "Source installation does not accept release-selection or recovery options."
    );
}
