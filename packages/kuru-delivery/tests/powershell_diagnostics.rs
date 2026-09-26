#![cfg(feature = "tooling")]

#[path = "support/powershell_diagnostic.rs"]
mod powershell_diagnostic;

#[test]
fn actual_wrapped_native_acl_error_is_decoded_without_progress_or_warning_text() {
    let output = br#"#< CLIXML
<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04">
<Obj S="progress"><S>Preparing modules for first use.</S></Obj>
<S S="warning">unrelated warning</S>
<S S="Error">install.ps1 : Exception calling &quot;.ctor&quot;: &quot;private _x000D__x000A_</S>
<S S="Error">ACL grants another principal access&quot;_x000D__x000A_</S>
</Objs>"#;
    assert_eq!(
        powershell_diagnostic::message(output),
        "install.ps1 : Exception calling \".ctor\": \"private ACL grants another principal access\""
    );
}

#[test]
fn error_decoding_is_single_pass_and_retains_plain_or_malformed_diagnostics() {
    assert_eq!(
        powershell_diagnostic::message(
            b"#< CLIXML\n<Objs><S S=\"Error\">_x005F_x0041_ _xD83D__xDE80_ &amp;</S></Objs>"
        ),
        "_x0041_ \u{1f680} &"
    );
    for plain in [
        "Error: private ACL grants\r\n another principal access",
        "<S S=\"Error\">literal XML _x000A_</S>",
        "#< CLIXML\nmalformed diagnostic",
    ] {
        assert_eq!(
            powershell_diagnostic::message(plain.as_bytes()),
            plain.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
}

#[test]
fn valid_envelopes_without_errors_cannot_satisfy_a_rejection_message() {
    for envelope in [
        "#< CLIXML\n<Objs><S S=\"warning\">private ACL grants another principal access</S></Objs>",
        "#< CLIXML\n<Objs><Obj S=\"progress\"><S>private ACL grants another principal access</S></Obj></Objs>",
        "#< CLIXML\n<Objs><S S=\"Error\"></S></Objs>",
    ] {
        assert_eq!(powershell_diagnostic::message(envelope.as_bytes()), "");
    }
}

/// Standard aliases shipped by `Microsoft.PowerShell.Utility` and
/// `Microsoft.PowerShell.Management` (PowerShell's documented default alias
/// list for those two modules). Each resolves through module auto-discovery
/// exactly like its full cmdlet name, so a bare alias reached before the
/// matching `Import-Module` would stall a cold profile the same way a
/// capitalized `Verb-Noun` cmdlet would. Matched case-insensitively.
const MODULE_ALIASES: &[&str] = &[
    // Microsoft.PowerShell.Utility
    "echo", "write", "select", "sort", "where", "group", "measure", "compare", "diff", "iex", "fl",
    "ft", "fw", "fc", "oh", "gm", "gu", "gv", "sv", "nv", "rv", "clv",
    // Microsoft.PowerShell.Management
    "gc", "cat", "type", "gci", "ls", "dir", "gp", "sp", "gps", "ps", "kill", "rm", "del", "erase",
    "rd", "ri", "rmdir", "cp", "copy", "cpi", "mv", "move", "mi", "ni", "pwd", "cd", "chdir", "sl",
    "gl", "clc", "cli", "clp", "gcb", "scb", "cvpa", "gdr", "ndr", "rdr", "gtz", "stz",
];

/// Blanks comment prose and quoted string literals, so English words like
/// `module auto-discovery` or a literal ZIP filename like `shell-support`
/// cannot themselves look like a command now that matching is
/// case-insensitive: only real code positions remain. A double-quoted
/// `$(...)` subexpression is kept as code, since PowerShell actually
/// evaluates it. Preserves length, so earlier byte offsets stay valid.
fn mask_non_code(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut masked = String::with_capacity(source.len());
    let mut quote: Option<char> = None;
    let mut expr_depth: u32 = 0;
    let mut in_comment = false;
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if in_comment {
            if character == '\n' {
                in_comment = false;
                masked.push(character);
            } else {
                masked.push(' ');
            }
            index += 1;
            continue;
        }
        if let Some(q) = quote {
            if q == '"' && expr_depth == 0 && character == '$' && chars.get(index + 1) == Some(&'(')
            {
                masked.push_str("$(");
                expr_depth = 1;
                index += 2;
                continue;
            }
            if expr_depth > 0 {
                match character {
                    '(' => expr_depth += 1,
                    ')' => expr_depth -= 1,
                    _ => {}
                }
                masked.push(character);
                index += 1;
                continue;
            }
            if character == q {
                quote = None;
                masked.push(character);
            } else if character == '\n' {
                masked.push(character);
            } else {
                masked.push(' ');
            }
            index += 1;
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
            masked.push(character);
            index += 1;
            continue;
        }
        if character == '#' {
            in_comment = true;
            masked.push(' ');
            index += 1;
            continue;
        }
        masked.push(character);
        index += 1;
    }
    masked
}

/// Offsets and lowercased names of discovered command tokens in PowerShell
/// source: `Verb-Noun` cmdlets in any letter case, plus standard
/// Utility/Management module aliases. Excludes variable references (`$name`)
/// and property/method access (`.name`), which share the same word shape but
/// never trigger auto-discovery.
fn command_tokens(source: &str) -> Vec<(usize, String)> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, character) in source.char_indices().chain([(source.len(), ' ')]) {
        // `_` joins identifier segments (e.g. `$env:KURU_INSTALL_DIR`) into one
        // token so a `SHOUT_CASE` piece can't coincidentally read as a bare
        // alias like `dir`.
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            start.get_or_insert(index);
            continue;
        }
        let Some(begin) = start.take() else { continue };
        let word = &source[begin..index];
        // `$name` (variable), `.name` (property/method) and `[name]` (type
        // literal, e.g. `-as [type]`) share this word shape but are never
        // reached through command auto-discovery.
        if matches!(
            begin.checked_sub(1).map(|i| bytes[i] as char),
            Some('$') | Some('.') | Some('[')
        ) {
            continue;
        }
        let lower = word.to_ascii_lowercase();
        let is_verb_noun = word.split_once('-').is_some_and(|(verb, noun)| {
            [verb, noun]
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphabetic()))
        });
        if is_verb_noun || MODULE_ALIASES.contains(&lower.as_str()) {
            tokens.push((begin, lower));
        }
    }
    tokens
}

#[test]
fn stock_installer_imports_pshome_modules_before_any_discovered_command() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("support/install.ps1"),
    )
    .unwrap();
    // The native bridge's C# here-string is not PowerShell command text. Drop
    // the whole `\n'@\n` closer too: left in place, its bare `'` would read
    // as an unterminated quote to the string-literal scan below and desync
    // quote tracking for the rest of the script.
    let bridge = source.find("Add-Type -TypeDefinition @'\n").unwrap() + "Add-Type".len();
    let bridge_end = bridge + source[bridge..].find("\n'@\n").unwrap() + "\n'@\n".len();
    let script = format!("{}{}", &source[..bridge], &source[bridge_end..]);

    let imports_end = ["Management", "Utility"]
        .map(|module| {
            let import = format!(
                "$null = Microsoft.PowerShell.Core\\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\\Microsoft.PowerShell.{module}\\Microsoft.PowerShell.{module}.psd1')) -Verbose:$false -ErrorAction Stop\n"
            );
            let offset = script
                .find(&import)
                .unwrap_or_else(|| panic!("install.ps1 must import exact PSHOME {module}"));
            offset + import.len()
        })
        .into_iter()
        .max()
        .unwrap();

    let functions: Vec<String> = script
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("function "))
        .filter_map(|rest| rest.split(|c: char| c == '(' || c.is_whitespace()).next())
        .map(str::to_ascii_lowercase)
        .collect();
    // Loaded with the engine; never reached through module auto-discovery.
    let core = ["import-module", "set-strictmode"];
    let imported = [
        "join-path",
        "add-type",
        "convertfrom-json",
        "convertto-json",
        "write-output",
        "write-verbose",
        "write-warning",
    ];
    let mut discovered = 0;
    for (offset, command) in command_tokens(&mask_non_code(&script)) {
        if functions.iter().any(|f| f == &command) || core.contains(&command.as_str()) {
            continue;
        }
        assert!(
            imported.contains(&command.as_str()),
            "install.ps1 uses {command}; import its exact stock PSHOME module before first use"
        );
        assert!(
            offset >= imports_end,
            "install.ps1 reaches {command} before its exact PSHOME module imports"
        );
        discovered += 1;
    }
    assert!(discovered > 0, "command inventory found no stock commands");
}

/// The exact `run` string of one package coverage task.
fn coverage_task_run(name: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(root.join("mise.toml")).unwrap()).unwrap();
    let task = &manifest["tasks"][name];
    assert!(
        task.get("run_windows").is_none(),
        "{name} must run the same orchestrator on every OS"
    );
    assert_eq!(
        task["dir"].as_str(),
        Some("{{config_root}}/../.."),
        "{name}"
    );
    assert_eq!(
        task["env"]["KURU_TEST_SUPERVISOR_PREPARED"].as_bool(),
        Some(false),
        "{name} must use the instrumented supervisor"
    );
    assert_eq!(
        task["depends"].as_array().unwrap(),
        &[
            toml::Value::from("//packages/kuru-memory:bundle:prepare"),
            toml::Value::from("//packages/kuru-memory:bundle:test-fixtures"),
        ],
        "{name} prepares only verified bundle inputs"
    );
    task["run"].as_str().unwrap().to_owned()
}

#[test]
fn coverage_tasks_run_the_rust_orchestrator_without_shell_metacharacters() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (task, mode) in [("coverage:shard", "shard"), ("coverage:collect", "collect")] {
        let run = coverage_task_run(task);
        assert_eq!(
            run,
            format!(
                "cargo run -p kuru-delivery --features tooling --locked --bin kuru-delivery -- coverage {mode}"
            )
        );
        // The same inline command runs under sh and Windows cmd.exe unchanged.
        assert!(
            !run.chars()
                .any(|character| "&|<>^%\"'`$();\\!*?[]{}~#\n".contains(character)),
            "{task} run string has shell metacharacters: {run}"
        );
    }
    assert!(
        !root.join("support/windows-coverage.ps1").exists(),
        "the PowerShell shard runner is retired"
    );
    let manifest = std::fs::read_to_string(root.join("mise.toml")).unwrap();
    assert!(!manifest.contains("coverage:windows:"), "{manifest}");
    assert!(manifest.contains("[tasks.\"coverage:workspace\"]"));
}

/// Launch the orchestrator's delivery binary exactly as a coverage task does,
/// with every coverage input removed, and return its combined diagnostics.
async fn orchestrator_without_inputs(mode: &str, through_cmd: bool) -> (bool, String) {
    use std::time::Duration;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let run = coverage_task_run(&format!("coverage:{mode}"));
    // Cargo's own `run` prefix builds this same binary; a test cannot invoke
    // Cargo under the outer test build lock, so it launches the built binary
    // with the task's exact arguments after `--`.
    let (_, arguments) = run.split_once(" -- ").unwrap();
    let binary = env!("CARGO_BIN_EXE_kuru-delivery");
    let mut child = if through_cmd {
        let mut child = kuru_delivery::command::rooted(&root, "cmd.exe");
        child.args(["/d", "/s", "/c", &format!("{binary} {arguments}")]);
        child
    } else {
        let mut child = kuru_delivery::command::rooted(&root, binary);
        child.args(arguments.split(' '));
        child
    };
    for (variable, _) in std::env::vars_os() {
        if variable.to_string_lossy().starts_with("KURU_COVERAGE_") {
            child.env_remove(&variable);
        }
    }
    let output =
        kuru_delivery::command::bounded_output(&mut child, Duration::from_secs(30), 16 * 1024)
            .await
            .unwrap();
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[tokio::test]
async fn coverage_orchestrator_refuses_missing_inputs_before_any_effect() {
    let (success, diagnostic) = orchestrator_without_inputs("shard", false).await;
    assert!(!success);
    assert!(
        diagnostic.contains(
            "coverage shard requires KURU_COVERAGE_TARGET, KURU_COVERAGE_SOURCE, \
             KURU_COVERAGE_ATTEMPT, KURU_COVERAGE_OS, KURU_COVERAGE_SHARD, \
             KURU_COVERAGE_PACKAGES, KURU_COVERAGE_OUTPUT, KURU_COVERAGE_DIAGNOSTICS, \
             KURU_COVERAGE_JOB_STARTED, KURU_COVERAGE_JOB_MINUTES"
        ),
        "shard did not reach input validation: {diagnostic}"
    );
    let (success, diagnostic) = orchestrator_without_inputs("collect", false).await;
    assert!(!success);
    assert!(
        diagnostic.contains(
            "coverage collect requires KURU_COVERAGE_TARGET, KURU_COVERAGE_SOURCE, \
             KURU_COVERAGE_ATTEMPT, KURU_COVERAGE_OS, KURU_COVERAGE_INPUTS, KURU_COVERAGE_REPORT"
        ),
        "collect did not reach input validation: {diagnostic}"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn cmd_mise_launches_published_windows_task_wrapper_before_cargo() {
    use std::time::Duration;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut child = kuru_delivery::command::rooted(&root, "mise");
    child.args(["run", "//packages/kuru-delivery:verify:published-windows"]);
    child.env("MISE_TASK_RUN_AUTO_INSTALL", "false");
    child.env("MISE_WINDOWS_DEFAULT_INLINE_SHELL_ARGS", "cmd.exe /d /s /c");
    for variable in [
        "RELEASE_VERSION",
        "RELEASE_SHA",
        "RELEASE_RUN_URL",
        "KURU_PUBLISHED_WINDOWS_RECEIPT",
    ] {
        child.env_remove(variable);
    }
    let output =
        kuru_delivery::command::bounded_output(&mut child, Duration::from_secs(15), 16 * 1024)
            .await
            .unwrap();
    assert!(!output.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("Published Windows verification requires"),
        "task did not reach wrapper validation: {diagnostic}"
    );
    assert!(
        !diagnostic.contains("is not recognized as an internal or external command"),
        "task executed its PowerShell body through cmd: {diagnostic}"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn cmd_launches_the_exact_coverage_tasks_and_reaches_input_validation() {
    for mode in ["shard", "collect"] {
        let (success, diagnostic) = orchestrator_without_inputs(mode, true).await;
        assert!(!success);
        assert!(
            diagnostic.contains(&format!("coverage {mode} requires KURU_COVERAGE_TARGET")),
            "{mode} did not reach orchestrator validation through cmd: {diagnostic}"
        );
        assert!(!diagnostic.contains("was unexpected at this time"));
        assert!(!diagnostic.contains("is not recognized as an internal or external command"));
    }
}
