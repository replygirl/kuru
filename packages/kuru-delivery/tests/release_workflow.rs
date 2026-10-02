#![cfg(feature = "tooling")]
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use kuru_delivery::release::{self, GitHub, Version};
use kuru_delivery::{
    archive::digest,
    coverage::{Mode, OS_TARGETS, WORKSPACE_PACKAGES, os_target, partition_count},
    shell_support,
};
use kuru_platform::fs::make_executable;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tempfile::TempDir;

#[path = "support/repository_environment.rs"]
mod repository_environment;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
fn v(input: &str) -> Version {
    input.parse().unwrap()
}

#[test]
fn required_release_checks_precede_the_only_publication_job() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    let job = |name: &str| {
        workflow
            .split_once(&format!("\n  {name}:\n"))
            .unwrap_or_else(|| panic!("missing release job {name}"))
            .1
            .lines()
            .take_while(|line| !line.starts_with("  ") || line.starts_with("    "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // Without condition overrides, failed, cancelled, or skipped prerequisites
    // prevent their dependent jobs from running under GitHub's default policy.
    for (name, needs) in [
        ("build", "needs: [plan, bump, verify, dolt-windows-arm64]"),
        ("assemble-candidate", "needs: [plan, bump, build, notes]"),
        (
            "verify-staged",
            "needs: [plan, bump, assemble-candidate, dolt-windows-arm64]",
        ),
        ("build-docs", "needs: [bump, assemble-candidate]"),
        ("deploy-docs", "needs: [build-docs, verify-staged]"),
        (
            "publish",
            "needs: [plan, bump, assemble-candidate, verify-staged, deploy-docs]",
        ),
    ] {
        let body = job(name);
        assert!(body.contains(needs), "{name} lost required dependencies");
        assert!(
            !body.lines().any(|line| line.starts_with("    if:")),
            "{name} must retain the default prerequisite success condition"
        );
        assert!(
            !body.contains("continue-on-error:"),
            "{name} must fail on unsuccessful required work"
        );
        // Native matrix jobs select their platform's steps with runner.os.
        if name != "build" && name != "verify-staged" {
            assert!(
                !body
                    .lines()
                    .any(|line| line.trim_start().starts_with("if:")),
                "{name} must not skip required steps"
            );
        }
    }
    // Planning follows static checks and one ordinary native test pass on the
    // dispatch SHA; coverage is CI's gate, never rerun inside the release.
    assert!(job("plan").contains("needs: [source-quality, tests]"));
    assert!(!workflow.contains("native-tests.yml"));
    assert!(!workflow.contains("source-tests"));
    assert!(!workflow.contains("verify-tests"));
    assert!(!workflow.contains("mise run coverage"));
    let tests = job("tests");
    for required in [
        "if: github.ref == 'refs/heads/main'",
        "runs-on: ubuntu-latest",
        "timeout-minutes: 60",
        "CARGO_PROFILE_TEST_DEBUG: \"0\"",
        "ref: ${{ github.sha }}",
        "install_args: rust github:aligned-team/cospec",
        "mise run //packages/kuru-delivery:setup:test-tools",
        "dbus-run-session -- bash -euo pipefail <<'KURU_NATIVE_STORE'",
        "            mise run test\n          KURU_NATIVE_STORE",
    ] {
        assert!(tests.contains(required), "release tests lost {required}");
    }
    assert!(!tests.contains("continue-on-error:"));
    // The OS secret-store session is the same text CI runs around each coverage shard.
    let native = fs::read_to_string(root.join(".github/workflows/native-tests.yml")).unwrap();
    let session = |text: &str, command: &str| {
        let start = text
            .find("          # The Linux keyring backend requires")
            .unwrap();
        let end = text[start..]
            .find(&format!(
                "            {command}\n          KURU_NATIVE_STORE\n"
            ))
            .unwrap();
        text[start..start + end].to_owned()
    };
    assert_eq!(
        session(&tests, "mise run test"),
        session(&native, "mise run //packages/kuru-delivery:coverage:shard")
    );

    let assembly = job("assemble-candidate");
    assert!(assembly.contains("mise run release:tool -- assemble"));
    assert!(assembly.contains("name=release-candidate-%s"));
    assert!(assembly.contains("RUN_ATTEMPT: ${{ github.run_attempt }}"));
    assert!(assembly.contains("artifact_name: ${{ steps.artifact.outputs.name }}"));
    assert!(assembly.contains("dist/*\n            RELEASE_NOTES.md"));
    assert!(assembly.contains("if-no-files-found: error"));
    assert!(!assembly.contains("contents: write"));
    assert!(!assembly.contains("GH_TOKEN:"));

    let build = job("build");
    for required in [
        "mise run //apps/kuru-tui:shell-support:generate",
        "mise run //packages/kuru-delivery:package:shell-support",
        "Get-FileHash -Algorithm SHA256",
        "openssl dgst -sha256",
        // Linux release archives keep the documented Ubuntu 24.04 glibc floor.
        "- os: ubuntu-24.04\n            target: x86_64-unknown-linux-gnu\n",
        "- os: ubuntu-24.04-arm\n            target: aarch64-unknown-linux-gnu\n",
        "- os: windows-latest\n            target: x86_64-pc-windows-msvc\n",
        "- os: windows-11-arm\n            target: aarch64-pc-windows-msvc\n",
    ] {
        assert!(build.contains(required), "native build lost {required}");
    }
    // The source-built Windows on Arm engine reaches its legs only through the
    // one pin-verified bundle-build input, imported before any build.
    let input = job("dolt-windows-arm64");
    for required in [
        "needs: bump",
        "uses: ./.github/workflows/bundle-build.yml",
        "ref: ${{ needs.bump.outputs.sha }}",
    ] {
        assert!(input.contains(required), "engine input lost {required}");
    }
    let imports = |body: &str, before: &str| {
        let steps = format!("\n{}", body.split("    steps:\n").nth(1).unwrap());
        let steps: Vec<_> = steps.split("\n      - ").skip(1).collect();
        let position = |name: &str| {
            steps
                .iter()
                .position(|step| step.starts_with(&format!("name: {name}\n")))
                .unwrap_or_else(|| panic!("missing step {name}"))
        };
        let download = position("Download the Windows arm64 engine input");
        let import = position("Import the pin-verified Windows arm64 engine");
        assert!(download < import && import < position(before));
        for step in [steps[download], steps[import]] {
            assert!(step.contains("if: matrix.target == 'aarch64-pc-windows-msvc'\n"));
        }
        assert!(steps[download].contains(
            "uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1\n"
        ));
        assert!(steps[download].contains("name: ${{ needs.dolt-windows-arm64.outputs.artifact }}"));
        assert!(steps[import].contains(
            "mise run //packages/kuru-memory:bundle:prepare -- --target aarch64-pc-windows-msvc --archive $archive --offline"
        ));
    };
    imports(&build, "Build and package native Windows executable");
    assert!(build.contains("KURU_DOLT_BUNDLE_DIR=$env:RUNNER_TEMP/kuru-bundles-release"));
    let bundle = fs::read_to_string(root.join(".github/workflows/bundle-build.yml")).unwrap();
    let bundle_job = |name: &str| {
        bundle
            .split_once(&format!("\n  {name}:\n"))
            .unwrap_or_else(|| panic!("missing bundle job {name}"))
            .1
            .lines()
            .take_while(|line| !line.starts_with("  ") || line.starts_with("    "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // The path-filtered determinism proof and the called input are selected by
    // the caller's input, never the caller's event, and only the proof cancels.
    let proof = bundle_job("windows-arm64-engine");
    let call = bundle_job("windows-arm64-input");
    assert!(proof.contains("    if: ${{ !inputs.ref }}\n"));
    assert!(proof.contains("      group: bundle-build-${{ github.ref }}\n"));
    assert!(proof.contains("--print-pins"));
    assert!(!bundle.contains("\nconcurrency:"));
    for required in [
        "    if: ${{ inputs.ref }}\n",
        "ref: ${{ inputs.ref }}",
        "uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "key: bundle-input-${{ env.BUILT_TARGET }}-${{ steps.pin.outputs.sha256 }}-",
        "name: Verify restored bytes against the pin",
        "mise run //packages/kuru-memory:bundle:build -- \\",
        "name: Verify built bytes against the pin",
        "if: steps.restore.outputs.cache-hit != 'true' && github.ref == 'refs/heads/main'",
        "uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "printf 'artifact=bundle-input-%s\\n' \"$BUILT_TARGET\"",
        "overwrite: true",
    ] {
        assert!(call.contains(required), "engine input job lost {required}");
    }
    assert!(!call.contains("--print-pins"));
    assert!(!call.contains("concurrency:"));

    let verifier = job("verify-staged");
    // Staged acceptance runs on every supported platform before promotion.
    let matrix = verifier
        .split("        include:\n")
        .nth(1)
        .unwrap()
        .split("    runs-on:")
        .next()
        .unwrap();
    assert_eq!(
        matrix,
        "          - os: windows-latest\n            target: x86_64-pc-windows-msvc\n          - os: windows-11-arm\n            target: aarch64-pc-windows-msvc\n          - os: ubuntu-latest\n            target: x86_64-unknown-linux-gnu\n          - os: ubuntu-24.04-arm\n            target: aarch64-unknown-linux-gnu\n          - os: macos-latest\n            target: aarch64-apple-darwin\n"
    );
    for required in [
        "fail-fast: false",
        "runs-on: ${{ matrix.os }}",
        "timeout-minutes: 45",
        "ref: ${{ needs.bump.outputs.sha }}",
        "version: 2026.9.18",
        "name: ${{ needs.assemble-candidate.outputs.artifact_name }}",
        "path: candidate",
        "KURU_STAGED_WINDOWS_ARCHIVE: ${{ github.workspace }}/candidate/dist/kuru-${{ needs.plan.outputs.version }}-${{ matrix.target }}.zip",
        "mise run //apps/kuru-tui:verify:staged-windows",
        "RELEASE_ARCHIVE: kuru-${{ needs.plan.outputs.version }}-${{ matrix.target }}.tar.gz",
        "candidate/dist/SHA256SUMS",
        "shasum -a 256 -c",
        "KURU_EMBEDDED_TEST_BINARY: ${{ runner.temp }}/kuru-staged/kuru",
        "KURU_UPDATE_CANDIDATE_BINARY: ${{ runner.temp }}/kuru-staged/kuru",
        "mise run //apps/kuru-tui:test:embedded-runtime",
        "mise run //packages/kuru-delivery:test:previous-release-update",
    ] {
        assert!(verifier.contains(required), "missing {required}");
    }
    // Each platform's steps are selected by runner.os, and the Unix legs check
    // the archive checksum before extracting and running the staged executable.
    let steps = format!("\n{}", verifier.split("    steps:\n").nth(1).unwrap());
    let steps: Vec<_> = steps.split("\n      - ").skip(1).collect();
    let step = |name: &str| {
        steps
            .iter()
            .position(|step| step.starts_with(&format!("name: {name}\n")))
            .unwrap_or_else(|| panic!("missing staged step {name}"))
    };
    for (name, condition) in [
        (
            "Extract the exact staged Unix archive",
            "if: runner.os != 'Windows'",
        ),
        (
            "Verify the staged Unix executable's offline runtime",
            "if: runner.os != 'Windows'",
        ),
        (
            "Accept the staged Unix executable from the previous published release",
            "if: runner.os != 'Windows'",
        ),
        (
            "Verify the exact staged package",
            "if: runner.os == 'Windows'",
        ),
    ] {
        assert!(
            steps[step(name)].contains(condition),
            "{name} lost {condition}"
        );
    }
    imports(&verifier, "Verify the exact staged package");
    let extract = steps[step("Extract the exact staged Unix archive")];
    assert!(extract.find("shasum -a 256 -c").unwrap() < extract.find("tar -xzf").unwrap());
    assert!(
        step("Extract the exact staged Unix archive")
            < step("Verify the staged Unix executable's offline runtime")
    );
    assert!(
        step("Verify the staged Unix executable's offline runtime")
            < step("Accept the staged Unix executable from the previous published release")
    );
    let updater =
        steps[step("Accept the staged Unix executable from the previous published release")];
    assert!(updater.contains("GITHUB_TOKEN: ${{ github.token }}"));
    assert!(!updater.contains("CARGO_NET_OFFLINE"));
    // The Windows mise-route step is the job's last step, so this slice holds
    // only that step. Its task re-runs previous-release update acceptance, so
    // it receives only the read-only workflow token for that release listing;
    // the fixture, mise and Kuru children never inherit it.
    assert_eq!(step("Verify the exact staged package"), steps.len() - 1);
    let execution = verifier
        .split("- name: Verify the exact staged package")
        .nth(1)
        .unwrap();
    assert_eq!(execution.matches("GITHUB_TOKEN").count(), 1);
    assert!(execution.contains("GITHUB_TOKEN: ${{ github.token }}"));
    assert!(!execution.contains("GH_TOKEN"));
    assert!(!execution.contains("CARGO_NET_OFFLINE"));

    let publication = job("publish");
    assert!(publication.contains("ref: ${{ needs.bump.outputs.sha }}"));
    assert!(publication.contains("name: ${{ needs.assemble-candidate.outputs.artifact_name }}"));
    assert!(!publication.contains("pattern:"));
    assert_eq!(
        workflow.matches("mise run release:tool -- publish").count(),
        1
    );
    let published = job("verify-published-windows");
    // Each Windows target verifies its own public download on its native runner.
    assert!(published.contains(
        "        include:\n          - os: windows-latest\n            target: x86_64-pc-windows-msvc\n          - os: windows-11-arm\n            target: aarch64-pc-windows-msvc\n    runs-on: ${{ matrix.os }}\n"
    ));
    for required in [
        "needs: [plan, bump, publish]",
        "fail-fast: false",
        "contents: read",
        "ref: ${{ needs.bump.outputs.sha }}",
        "RELEASE_VERSION: ${{ needs.plan.outputs.version }}",
        "RELEASE_SHA: ${{ needs.bump.outputs.sha }}",
        "KURU_PUBLISHED_TARGET: ${{ matrix.target }}",
        "KURU_PUBLISHED_WINDOWS_RECEIPT: ${{ runner.temp }}/published-windows-${{ matrix.target }}-receipt.json",
        "mise run //packages/kuru-delivery:verify:published-windows",
        "name: published-windows-${{ matrix.target }}-${{ needs.plan.outputs.version }}-${{ github.run_attempt }}",
        "path: ${{ runner.temp }}/published-windows-${{ matrix.target }}-receipt.json",
        "if-no-files-found: error",
    ] {
        assert!(published.contains(required), "missing {required}");
    }
    assert!(
        workflow.trim_end().ends_with("if-no-files-found: error"),
        "public verification receipt upload must be the final workflow step"
    );
}

#[test]
fn optional_published_windows_diagnostic_keeps_its_native_launcher() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task = fs::read_to_string(root.join("packages/kuru-delivery/mise.toml")).unwrap();
    let task = task
        .split("[tasks.\"verify:published-windows\"]")
        .nth(1)
        .unwrap();
    assert!(task.contains(
        "run_windows = \"pwsh.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File packages/kuru-delivery/support/verify-published-windows.ps1\""
    ));
    let script = fs::read_to_string(
        root.join("packages/kuru-delivery/support/verify-published-windows.ps1"),
    )
    .unwrap();
    let resolve = script
        .find("Get-Command mise -CommandType Application")
        .unwrap();
    let clear = script.find("verify-published-windows --mise").unwrap();
    assert!(
        resolve < clear,
        "native mise must be resolved before verifier env clearing"
    );
    assert!(!task.contains("kuru-memory"));
    assert!(!task.contains("kuru-tui"));
}

/// A mise task as a release job reaches it on Linux: the config root that
/// owns it, its resolved dependencies, its `run` commands and the tools it
/// declares for itself, with the version each declaration pins.
struct MiseTask {
    root: String,
    depends: Vec<String>,
    run: Vec<String>,
    tools: BTreeMap<String, String>,
}

/// The root, app and package mise configurations: every task by address,
/// and each config root's `[vars]` and `[tools]` versions.
struct MiseGraph {
    tasks: BTreeMap<String, MiseTask>,
    vars: BTreeMap<String, BTreeMap<String, String>>,
    configured: BTreeMap<String, BTreeMap<String, String>>,
}

impl MiseGraph {
    /// Substitutes `{{vars.NAME}}` from the config root, then the repository
    /// root. Any other template is refused rather than guessed at.
    fn resolve(&self, root: &str, text: &str) -> String {
        let mut resolved = text.to_owned();
        while let Some(start) = resolved.find("{{") {
            let end = start
                + resolved[start..]
                    .find("}}")
                    .unwrap_or_else(|| panic!("unterminated template in {text}"));
            let name = resolved[start + 2..end].trim();
            let var = name
                .strip_prefix("vars.")
                .and_then(|var| {
                    [root, ""]
                        .iter()
                        .find_map(|root| self.vars.get(*root).and_then(|vars| vars.get(var)))
                })
                .unwrap_or_else(|| {
                    panic!("the tool derivation does not model `{name}` in {text} for //{root}")
                });
            resolved.replace_range(start..end + 2, var);
        }
        resolved
    }

    /// The version a tool resolves to from a config root without a task
    /// declaration: that root's `[tools]` pin, else the repository root's.
    fn configured(&self, root: &str, tool: &str) -> Option<String> {
        [root, ""]
            .iter()
            .find_map(|root| self.configured.get(*root).and_then(|tools| tools.get(tool)))
            .map(|version| self.resolve(root, version))
    }
}

/// A tool's identity in mise configuration, install arguments and task
/// tool tables: its name without a version or backend options, so that
/// `aqua:cocogitto/cocogitto@{{vars.kuru_cocogitto_version}}` names
/// `tools."aqua:cocogitto/cocogitto"`.
fn tool_identity(spec: &str) -> String {
    spec.trim_matches(['"', '\''])
        .split(['@', '['])
        .next()
        .unwrap()
        .to_owned()
}

/// A tool an install argument names, with the version it installs: the one
/// it names explicitly, else the one configured where the install runs.
fn installed_tool(graph: &MiseGraph, root: &str, spec: &str) -> (String, Option<String>) {
    let spec = spec.trim_matches(['"', '\'']);
    let tool = tool_identity(spec);
    let version = match spec.rsplit_once('@') {
        Some((_, version)) => Some(graph.resolve(root, version)),
        None => graph.configured(root, &tool),
    };
    (tool, version)
}

/// The version string of a `[tools]` or task `tools` entry: the string
/// itself or its table's `version`.
fn tool_version(entry: &toml::Value, context: &str) -> String {
    entry
        .as_str()
        .or_else(|| entry.get("version").and_then(toml::Value::as_str))
        .unwrap_or_else(|| panic!("{context} has no modelled version: {entry}"))
        .to_owned()
}

/// A task address as mise's monorepo resolves it from a config root: an
/// unqualified name belongs to the same config, `//path:name` to that one.
fn task_address(reference: &str, root: &str) -> String {
    if reference.starts_with("//") {
        reference.to_owned()
    } else {
        format!("//{root}:{reference}")
    }
}

/// Every task of the root, app and package mise configurations, by address,
/// with each config root's variables and configured tool versions.
fn mise_tasks(repository: &Path) -> MiseGraph {
    let mut roots = vec![String::new()];
    for parent in ["apps", "packages"] {
        let mut members: Vec<String> = fs::read_dir(repository.join(parent))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.join("mise.toml").is_file())
            .map(|path| format!("{parent}/{}", path.file_name().unwrap().to_string_lossy()))
            .collect();
        members.sort();
        roots.extend(members);
    }
    let mut graph = MiseGraph {
        tasks: BTreeMap::new(),
        vars: BTreeMap::new(),
        configured: BTreeMap::new(),
    };
    let mut declared = Vec::new();
    for root in roots {
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(repository.join(&root).join("mise.toml")).unwrap())
                .unwrap();
        let table = |key: &str| manifest.get(key).and_then(toml::Value::as_table);
        if let Some(vars) = table("vars") {
            let vars = vars
                .iter()
                .map(|(name, value)| {
                    let value = value
                        .as_str()
                        .unwrap_or_else(|| panic!("//{root} var {name} is not a string"));
                    (name.clone(), value.to_owned())
                })
                .collect();
            graph.vars.insert(root.clone(), vars);
        }
        if let Some(tools) = table("tools") {
            let tools = tools
                .iter()
                .map(|(tool, entry)| {
                    let context = format!("//{root} tools.{tool}");
                    (tool_identity(tool), tool_version(entry, &context))
                })
                .collect();
            graph.configured.insert(root.clone(), tools);
        }
        let Some(tasks) = table("tasks") else {
            continue;
        };
        for (name, task) in tasks {
            let strings = |key: &str| -> Vec<String> {
                match task.get(key) {
                    None => Vec::new(),
                    Some(toml::Value::String(one)) => vec![one.clone()],
                    // A `{ task = "...", args = [...] }` entry runs that task.
                    Some(toml::Value::Array(many)) => many
                        .iter()
                        .map(|item| match (item.as_str(), item.get("task")) {
                            (Some(one), _) => one.to_owned(),
                            (None, Some(toml::Value::String(task))) if key == "run" => {
                                format!("mise run {task}")
                            }
                            (None, Some(toml::Value::String(task))) => task.clone(),
                            _ => panic!("task //{root}:{name} has an unmodelled {key}: {item}"),
                        })
                        .collect(),
                    Some(other) => panic!("task //{root}:{name} has an unmodelled {key}: {other}"),
                }
            };
            let address = task_address(name, &root);
            let tools: Vec<(String, String)> = task
                .get("tools")
                .and_then(toml::Value::as_table)
                .map(|tools| {
                    tools
                        .iter()
                        .map(|(tool, entry)| {
                            let context = format!("task {address} tools.{tool}");
                            (tool_identity(tool), tool_version(entry, &context))
                        })
                        .collect()
                })
                .unwrap_or_default();
            declared.push((address.clone(), tools));
            let depends = strings("depends")
                .iter()
                .map(|reference| task_address(reference, &root))
                .collect();
            graph.tasks.insert(
                address,
                MiseTask {
                    root: root.clone(),
                    depends,
                    run: strings("run"),
                    tools: BTreeMap::new(),
                },
            );
        }
    }
    // Resolve declared versions once every config root's variables are known.
    for (address, tools) in declared {
        let root = graph.tasks[&address].root.clone();
        let tools = tools
            .into_iter()
            .map(|(tool, version)| (tool, graph.resolve(&root, &version)))
            .collect();
        graph.tasks.get_mut(&address).unwrap().tools = tools;
    }
    graph
}

/// The words of each simple command in a script: continuation lines joined,
/// split at newlines, `;`, `&&`, `||` and `|`, comments and quotes removed
/// and leading assignments dropped. Nothing is expanded, so a command
/// substitution or a template branch is refused rather than guessed at.
fn script_commands(script: &str) -> Vec<Vec<String>> {
    assert!(
        !script.contains("$(") && !script.contains('`') && !script.contains("{%"),
        "the tool derivation does not model command substitution or templates: {script}"
    );
    let mut commands = Vec::new();
    for line in script.replace("\\\n", " ").lines() {
        let line = line.replace("&&", "\n").replace("||", "\n");
        for command in line.split(['\n', ';', '|']) {
            let words: Vec<String> = command
                .split_whitespace()
                .take_while(|word| !word.starts_with('#'))
                .map(|word| word.trim_matches(['"', '\'']).to_owned())
                .skip_while(|word| {
                    word.split_once('=').is_some_and(|(name, _)| {
                        !name.is_empty()
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    })
                })
                .collect();
            if !words.is_empty() {
                commands.push(words);
            }
        }
    }
    commands
}

/// The configured tools each program a reached command runs needs on PATH.
/// Programs the Ubuntu runner image provides need none. Any other program
/// fails the derivation until someone classifies it here.
fn program_tools(program: &str) -> &'static [&'static str] {
    match program {
        "cargo" | "rustc" | "rustup" => &["rust"],
        "node" => &["node"],
        "npm" | "npx" => &["node", "npm"],
        "git" | "printf" | "echo" | "test" => &[],
        // npm's generated bin scripts run under `#!/usr/bin/env node`.
        _ if program.starts_with("node_modules/.bin/") => &["node"],
        _ => panic!("the tool derivation does not know which tool provides `{program}`"),
    }
}

/// What one top-level command does to a job's tools: every tool it needs,
/// with the version that user resolves and the task or step that needs it;
/// the tools it installs by name, with their versions; and any install that
/// names no tool, which installs every configured one.
#[derive(Default)]
struct ToolEffect {
    uses: Vec<(String, Option<String>, String)>,
    installs: Vec<(String, Option<String>)>,
    unnamed_installs: Vec<String>,
}

impl ToolEffect {
    fn command(&mut self, graph: &MiseGraph, root: &str, by: &str, words: &[String]) {
        let program = words[0].as_str();
        if program != "mise" {
            for tool in program_tools(program) {
                let version = graph.configured(root, tool);
                self.uses.push(((*tool).to_owned(), version, by.to_owned()));
            }
            return;
        }
        let mut operands = words[1..]
            .iter()
            .take_while(|word| *word != "--")
            .filter(|word| !word.starts_with('-'));
        match operands.next().map(String::as_str) {
            Some("run") => {
                let task = operands
                    .next()
                    .unwrap_or_else(|| panic!("{by} runs mise without a task"));
                self.task(graph, &task_address(task, root));
            }
            Some("install" | "i") => {
                let named: Vec<(String, Option<String>)> = operands
                    .map(|word| installed_tool(graph, root, word))
                    .collect();
                if named.is_empty() {
                    self.unnamed_installs.push(by.to_owned());
                }
                self.installs.extend(named);
            }
            other => panic!("the tool derivation does not model `mise {other:?}` in {by}"),
        }
    }

    /// A task's dependencies run first, inside the same `mise run`; then its
    /// declared tools and the programs of its commands are needed.
    fn task(&mut self, graph: &MiseGraph, address: &str) {
        let task = graph
            .tasks
            .get(address)
            .unwrap_or_else(|| panic!("no mise task {address}"));
        for dependency in &task.depends {
            self.task(graph, dependency);
        }
        for (tool, version) in &task.tools {
            self.uses
                .push((tool.clone(), Some(version.clone()), address.to_owned()));
        }
        for script in &task.run {
            for words in script_commands(script) {
                self.command(graph, &task.root, address, &words);
            }
        }
    }
}

/// Every tool one Linux job needs, with the steps that need it.
type NeededTools = BTreeMap<String, BTreeSet<String>>;

/// The workflow-level mise settings the derivation models. Any other `MISE_`
/// variable, and any at job or step level, could select other configuration.
const MODELLED_WORKFLOW_MISE_ENV: [&str; 3] = [
    "MISE_LOCKED",
    "MISE_EXEC_AUTO_INSTALL",
    "MISE_TASK_RUN_AUTO_INSTALL",
];

/// The mise-action inputs the derivation models; others can skip or
/// redirect its install.
const MODELLED_MISE_ACTION_INPUTS: [&str; 3] = ["experimental", "version", "install_args"];

/// The `MISE_` variables an `env` mapping sets.
fn mise_variables(env: &serde_yaml_ng::Value) -> Vec<String> {
    env.as_mapping()
        .into_iter()
        .flat_map(|env| env.keys())
        .filter_map(serde_yaml_ng::Value::as_str)
        .filter(|key| key.starts_with("MISE_"))
        .map(str::to_owned)
        .collect()
}

/// Derives the tools one Linux job needs from its steps and the mise tasks
/// they run, and reports each tool needed before an earlier command
/// installed its resolved version by name, and each install that names no
/// tool.
///
/// mise-action's `install_args` are installed before the next step. A named
/// `mise install`, directly or inside a task, counts only once its whole
/// top-level command has finished: `mise run` fixes a task's PATH when it
/// starts, so with automatic installation off a dependency that installs a
/// tool does not put it on the PATH of the same run (quality.yml's docs job
/// installs `setup:tools` in its own step for this reason). An install counts
/// only for the version it installs: an explicit `tool@version`, else the
/// version configured where it runs. Declared task tools count as needed,
/// at their declared version, even where the task's program does not run
/// them.
///
/// An install step that carries `if` or `continue-on-error`, or a
/// mise-action input beyond the modelled ones, counts for nothing and is
/// reported. So is anything that could change which configuration the
/// commands read: workflow or job `defaults`, a step `working-directory` or
/// `shell`, a `MISE_` variable other than the modelled workflow settings,
/// and a step that writes `GITHUB_ENV` or `GITHUB_PATH`.
fn job_tool_findings(
    workflow: &serde_yaml_ng::Value,
    graph: &MiseGraph,
    id: &str,
) -> (NeededTools, Vec<String>) {
    let job = &workflow["jobs"][id];
    assert_eq!(
        job["runs-on"].as_str(),
        Some("ubuntu-latest"),
        "{id}: the derivation reads Linux `run` commands only"
    );
    let mut findings = Vec::new();
    let mut unmodelled =
        |what: String| findings.push(format!("{id}: {what}, which the derivation does not model"));
    if !workflow["defaults"].is_null() {
        unmodelled("the workflow sets `defaults`".to_owned());
    }
    if !job["defaults"].is_null() {
        unmodelled("the job sets `defaults`".to_owned());
    }
    for key in mise_variables(&workflow["env"]) {
        if !MODELLED_WORKFLOW_MISE_ENV.contains(&key.as_str()) {
            unmodelled(format!("the workflow env sets {key}"));
        }
    }
    for key in mise_variables(&job["env"]) {
        unmodelled(format!("the job env sets {key}"));
    }
    let mut available: BTreeMap<String, BTreeSet<Option<String>>> = BTreeMap::new();
    let mut needed = NeededTools::new();
    for (index, step) in job["steps"].as_sequence().unwrap().iter().enumerate() {
        let label = step["name"]
            .as_str()
            .map_or_else(|| format!("step {}", index + 1), str::to_owned);
        for key in mise_variables(&step["env"]) {
            findings.push(format!(
                "{id}: {label} env sets {key}, which the derivation does not model"
            ));
        }
        for key in ["working-directory", "shell"] {
            if !step[key].is_null() {
                findings.push(format!(
                    "{id}: {label} sets `{key}`, which the derivation does not model"
                ));
            }
        }
        // A skipped or failure-tolerated step may not have installed anything.
        let conditions: Vec<&str> = ["if", "continue-on-error"]
            .into_iter()
            .filter(|key| !step[*key].is_null())
            .collect();
        let install = |installs: Vec<(String, Option<String>)>,
                       reliable: bool,
                       available: &mut BTreeMap<String, BTreeSet<Option<String>>>,
                       findings: &mut Vec<String>| {
            if installs.is_empty() {
                return;
            }
            for key in &conditions {
                findings.push(format!(
                    "{id}: {label} installs tools under `{key}`, so they do not count as installed"
                ));
            }
            if reliable && conditions.is_empty() {
                for (tool, version) in installs {
                    available.entry(tool).or_default().insert(version);
                }
            }
        };
        if let Some(uses) = step["uses"].as_str() {
            if uses.starts_with("jdx/mise-action@") {
                let mut reliable = true;
                for input in step["with"].as_mapping().unwrap().keys() {
                    let input = input.as_str().unwrap();
                    if !MODELLED_MISE_ACTION_INPUTS.contains(&input) {
                        reliable = false;
                        findings.push(format!(
                            "{id}: {label} sets mise-action `{input}`, which the derivation does not model"
                        ));
                    }
                }
                let installs = step["with"]["install_args"]
                    .as_str()
                    .unwrap()
                    .split_whitespace()
                    .filter(|word| !word.starts_with('-'))
                    .map(|word| installed_tool(graph, "", word))
                    .collect();
                install(installs, reliable, &mut available, &mut findings);
            }
            continue;
        }
        let script = step["run"].as_str().unwrap();
        for file in ["GITHUB_ENV", "GITHUB_PATH"] {
            if script.contains(file) {
                findings.push(format!(
                    "{id}: {label} writes {file}, which the derivation does not model"
                ));
            }
        }
        for words in script_commands(script) {
            let by = format!("{label} (`{}`)", words.join(" "));
            let mut effect = ToolEffect::default();
            effect.command(graph, "", &by, &words);
            for (tool, version, user) in effect.uses {
                let installed = available.get(&tool).is_some_and(|versions| {
                    versions
                        .iter()
                        .any(|installed| version.is_none() || *installed == version)
                });
                if !installed {
                    findings.push(format!(
                        "{id}: {by} needs {tool} for {user} before an earlier command installs it by name"
                    ));
                }
                needed.entry(tool).or_default().insert(label.clone());
            }
            for task in effect.unnamed_installs {
                findings.push(format!(
                    "{id}: {by} installs every configured tool through {task} instead of naming them"
                ));
            }
            install(effect.installs, true, &mut available, &mut findings);
        }
    }
    findings.sort();
    findings.dedup();
    (needed, findings)
}

fn release_repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn release_notes_and_docs_jobs_install_every_tool_they_run_before_its_first_use() {
    // PR CI cannot run the release workflow, so derive what these two jobs
    // run from their steps and the mise task graph, and require an explicit,
    // earlier, named install of each tool with automatic installation off.
    let root = release_repository();
    let text = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    let workflow: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
    let tasks = mise_tasks(&root);
    let mut problems = Vec::new();
    for variable in ["MISE_EXEC_AUTO_INSTALL", "MISE_TASK_RUN_AUTO_INSTALL"] {
        if workflow["env"][variable].as_str() != Some("false") {
            problems.push(format!(
                "the derivation needs {variable}: \"false\" for the whole release workflow"
            ));
        }
    }
    let set = |tools: &[&str]| -> BTreeSet<String> {
        tools.iter().map(|tool| (*tool).to_owned()).collect()
    };
    for (id, expected) in [
        (
            "notes",
            set(&["aqua:cocogitto/cocogitto", "github:jdx/communique", "rust"]),
        ),
        ("build-docs", set(&["node", "npm", "rust"])),
    ] {
        let (needed, findings) = job_tool_findings(&workflow, &tasks, id);
        problems.extend(findings);
        let derived: BTreeSet<String> = needed.keys().cloned().collect();
        if derived != expected {
            problems.push(format!("{id} needs {derived:?}, not {expected:?}"));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn release_tool_derivation_rejects_each_unscoped_install_it_replaced() {
    let root = release_repository();
    let text = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    let tasks = mise_tasks(&root);
    let findings = |job: &str, old: &str, new: &str| {
        assert!(text.contains(old), "release.yml no longer contains {old}");
        let workflow = serde_yaml_ng::from_str(&text.replacen(old, new, 1)).unwrap();
        job_tool_findings(&workflow, &tasks, job).1
    };
    let missing = |job: &str, by: &str, tool: &str, user: &str| {
        format!("{job}: {by} needs {tool} for {user} before an earlier command installs it by name")
    };
    // Before this change notes ran the delivery `setup` task, a bare
    // `mise install --include-task-tools` that installs every configured tool.
    let notes_install = "      - name: Install package-owned release tools\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n        run: mise run //packages/kuru-delivery:setup:test-tools\n";
    let generate = "Generate notes without publishing (`mise run release:tool -- notes --sha $RELEASE_SHA --version $RELEASE_VERSION --output RELEASE_NOTES.md`)";
    let release = "//packages/kuru-delivery:release";
    assert_eq!(
        findings(
            "notes",
            notes_install,
            &notes_install.replace(":setup:test-tools", ":setup")
        ),
        [
            missing("notes", generate, "aqua:cocogitto/cocogitto", release),
            missing("notes", generate, "github:jdx/communique", release),
            "notes: Install package-owned release tools (`mise run //packages/kuru-delivery:setup`) installs every configured tool through //packages/kuru-delivery:setup instead of naming them".to_owned(),
        ]
    );
    // A named install of other tools leaves the notes tools missing.
    assert_eq!(
        findings(
            "notes",
            notes_install,
            &notes_install.replace(":setup:test-tools", ":setup:advisories")
        ),
        [
            missing("notes", generate, "aqua:cocogitto/cocogitto", release),
            missing("notes", generate, "github:jdx/communique", release),
        ]
    );
    // Without the explicit docs tool step, setup:tools runs only as a
    // dependency inside the same `mise run` as its first Node/npm use.
    let docs_tools = "      - name: Install app-owned documentation tools before task activation\n        # With automatic installation disabled, setup's PATH must see the pins\n        # before it starts; the runner's preinstalled Node/npm are not substitutes.\n        run: mise run //apps/kuru-docs:setup:tools\n";
    let setup_step = "step 3 (`mise run //apps/kuru-docs:setup`)";
    let early_use = [
        missing(
            "build-docs",
            setup_step,
            "node",
            "//apps/kuru-docs:check:toolchain",
        ),
        missing("build-docs", setup_step, "node", "//apps/kuru-docs:setup"),
        missing("build-docs", setup_step, "npm", "//apps/kuru-docs:setup"),
    ];
    assert_eq!(findings("build-docs", docs_tools, ""), early_use);
    // An install after the first use does not count either.
    let setup = "      - run: mise run //apps/kuru-docs:setup\n";
    assert_eq!(
        findings(
            "build-docs",
            &format!("{docs_tools}{setup}"),
            &format!("{setup}{docs_tools}")
        ),
        early_use
    );
    // A tool nothing installs is found where the task graph first needs it.
    let action = "          install_args: rust aqua:jdx/hk\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - name: Install app-owned";
    assert_eq!(
        findings(
            "build-docs",
            action,
            &action.replace(
                "install_args: rust aqua:jdx/hk",
                "install_args: aqua:jdx/hk"
            )
        ),
        [missing(
            "build-docs",
            "Build and validate public documentation (`mise run //packages/kuru-delivery:tool -- docs --base $KURU_DOCS_BASE`)",
            "rust",
            "//packages/kuru-delivery:tool"
        )]
    );
}

#[test]
fn release_tool_derivation_rejects_conditional_installs_and_unmodelled_configuration() {
    let root = release_repository();
    let text = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    let tasks = mise_tasks(&root);
    let findings = |job: &str, old: &str, new: &str| {
        assert_eq!(
            text.matches(old).count(),
            1,
            "release.yml must contain {old} once"
        );
        let workflow = serde_yaml_ng::from_str(&text.replacen(old, new, 1)).unwrap();
        job_tool_findings(&workflow, &tasks, job).1
    };
    let missing = |job: &str, by: &str, tool: &str, user: &str| {
        format!("{job}: {by} needs {tool} for {user} before an earlier command installs it by name")
    };
    let unmodelled =
        |job: &str, what: &str| format!("{job}: {what}, which the derivation does not model");
    let generate = "Generate notes without publishing (`mise run release:tool -- notes --sha $RELEASE_SHA --version $RELEASE_VERSION --output RELEASE_NOTES.md`)";
    let release = "//packages/kuru-delivery:release";
    let notes_missing = [
        missing("notes", generate, "aqua:cocogitto/cocogitto", release),
        missing("notes", generate, "github:jdx/communique", release),
    ];

    // A skipped or failure-tolerated install step installs nothing the
    // later steps can rely on.
    let notes_install = "      - name: Install package-owned release tools\n        env:\n";
    assert_eq!(
        findings(
            "notes",
            notes_install,
            "      - name: Install package-owned release tools\n        if: ${{ false }}\n        env:\n"
        ),
        [
            notes_missing[0].clone(),
            notes_missing[1].clone(),
            "notes: Install package-owned release tools installs tools under `if`, so they do not count as installed".to_owned(),
        ]
    );
    let docs_tools = "        run: mise run //apps/kuru-docs:setup:tools\n";
    let setup_step = "step 4 (`mise run //apps/kuru-docs:setup`)";
    assert_eq!(
        findings(
            "build-docs",
            docs_tools,
            &format!("        if: ${{{{ false }}}}\n{docs_tools}")
        ),
        [
            "build-docs: Install app-owned documentation tools before task activation installs tools under `if`, so they do not count as installed".to_owned(),
            missing("build-docs", setup_step, "node", "//apps/kuru-docs:check:toolchain"),
            missing("build-docs", setup_step, "node", "//apps/kuru-docs:setup"),
            missing("build-docs", setup_step, "npm", "//apps/kuru-docs:setup"),
        ]
    );
    let action = "          install_args: rust aqua:jdx/hk\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - name: Install app-owned";
    let build = "Build and validate public documentation (`mise run //packages/kuru-delivery:tool -- docs --base $KURU_DOCS_BASE`)";
    let rust_missing = missing("build-docs", build, "rust", "//packages/kuru-delivery:tool");
    assert_eq!(
        findings(
            "build-docs",
            action,
            &action.replace("        env:\n", "        continue-on-error: true\n        env:\n")
        ),
        [
            rust_missing.clone(),
            "build-docs: step 2 installs tools under `continue-on-error`, so they do not count as installed".to_owned(),
        ]
    );
    // mise-action inputs beyond the modelled ones can skip or redirect its install.
    assert_eq!(
        findings(
            "build-docs",
            action,
            &action.replace(
                "          install_args:",
                "          install: false\n          install_args:"
            )
        ),
        [
            rust_missing,
            unmodelled("build-docs", "step 2 sets mise-action `install`"),
        ]
    );

    // A working directory, shell or mise environment the derivation does not
    // read would change which configuration and tasks the commands reach.
    let concurrency = "concurrency:\n  group: release\n";
    assert_eq!(
        findings(
            "notes",
            concurrency,
            &format!(
                "defaults:\n  run:\n    working-directory: packages/kuru-delivery\n{concurrency}"
            )
        ),
        [unmodelled("notes", "the workflow sets `defaults`")]
    );
    let docs_permissions = "    permissions:\n      contents: read\n      pages: read\n";
    assert_eq!(
        findings(
            "build-docs",
            docs_permissions,
            &format!(
                "    defaults:\n      run:\n        working-directory: apps/kuru-docs\n{docs_permissions}"
            )
        ),
        [unmodelled("build-docs", "the job sets `defaults`")]
    );
    let setup = "      - run: mise run //apps/kuru-docs:setup\n";
    assert_eq!(
        findings(
            "build-docs",
            setup,
            &format!("{setup}        working-directory: apps/kuru-docs\n")
        ),
        [unmodelled("build-docs", "step 4 sets `working-directory`")]
    );
    assert_eq!(
        findings("build-docs", setup, &format!("{setup}        shell: sh\n")),
        [unmodelled("build-docs", "step 4 sets `shell`")]
    );
    let version_env = "          OPENAI_API_KEY: ${{ secrets.ANTHROPIC_API_KEY_COMMUNIQUE }}\n";
    assert_eq!(
        findings(
            "notes",
            version_env,
            &format!("{version_env}          MISE_ENV: release\n")
        ),
        [unmodelled(
            "notes",
            "Generate notes without publishing env sets MISE_ENV"
        )]
    );
    assert_eq!(
        findings(
            "build-docs",
            docs_permissions,
            &format!("    env:\n      MISE_ENV: docs\n{docs_permissions}")
        ),
        [unmodelled("build-docs", "the job env sets MISE_ENV")]
    );
    let workflow_env = "  MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n";
    assert_eq!(
        findings(
            "notes",
            workflow_env,
            &format!("{workflow_env}  MISE_CONFIG_FILE: release.toml\n")
        ),
        [unmodelled(
            "notes",
            "the workflow env sets MISE_CONFIG_FILE"
        )]
    );
    let output = "printf 'name=github-pages-%s\\n' \"$RUN_ATTEMPT\" >> \"$GITHUB_OUTPUT\"";
    assert_eq!(
        findings(
            "build-docs",
            output,
            &output.replace("GITHUB_OUTPUT", "GITHUB_ENV")
        ),
        [unmodelled(
            "build-docs",
            "Select this build attempt's Pages artifact writes GITHUB_ENV"
        )]
    );

    // A named install counts only for the version the task pins.
    let test_tools = "          GITHUB_TOKEN: ${{ github.token }}\n        run: mise run //packages/kuru-delivery:setup:test-tools\n";
    assert_eq!(
        findings(
            "notes",
            test_tools,
            "          GITHUB_TOKEN: ${{ github.token }}\n        run: mise install aqua:cocogitto/cocogitto@6.0.0 github:jdx/communique@0.1.0\n"
        ),
        notes_missing
    );
    assert_eq!(
        findings(
            "notes",
            test_tools,
            "          GITHUB_TOKEN: ${{ github.token }}\n        run: mise install aqua:cocogitto/cocogitto@7.0.0 github:jdx/communique@0.1.0\n"
        ),
        [notes_missing[1].clone()]
    );
    assert_eq!(
        findings(
            "notes",
            test_tools,
            "          GITHUB_TOKEN: ${{ github.token }}\n        run: mise install aqua:cocogitto/cocogitto@7.0.0 github:jdx/communique@1.4.2\n"
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        findings(
            "build-docs",
            docs_tools,
            "        run: mise install node@24.0.0 npm@12.1.0\n"
        ),
        [
            missing(
                "build-docs",
                setup_step,
                "node",
                "//apps/kuru-docs:check:toolchain"
            ),
            missing("build-docs", setup_step, "node", "//apps/kuru-docs:setup"),
        ]
    );
}

fn native_workflow() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(".github/workflows/native-tests.yml")).unwrap()
}

fn workflow_job<'a>(workflow: &'a str, name: &str, next: &str) -> &'a str {
    workflow
        .split(&format!("\n  {name}:\n"))
        .nth(1)
        .unwrap_or_else(|| panic!("missing native job {name}"))
        .split(&format!("\n  {next}:\n"))
        .next()
        .unwrap()
}

fn workflow_steps(job: &str) -> Vec<String> {
    let steps = format!("\n{}", job.split("    steps:\n").nth(1).unwrap());
    steps
        .split("\n      - ")
        .skip(1)
        .map(str::to_owned)
        .collect()
}

/// The JSON partition list a workflow matrix names for a partition count.
fn partition_list(count: u32) -> String {
    let items: Vec<_> = (1..=count).map(|k| k.to_string()).collect();
    format!("[{}]", items.join(","))
}

fn table_count(os: &str, mode: Mode) -> u32 {
    partition_count(os, mode).unwrap_or_else(|| panic!("no {} partitions for {os}", mode.name()))
}

fn step_env(step: &str) -> String {
    step.split("        env:\n")
        .nth(1)
        .unwrap()
        .split("        run:")
        .next()
        .unwrap()
        .to_owned()
}

fn named_step<'a>(steps: &'a [String], name: &str) -> &'a str {
    steps
        .iter()
        .find(|step| {
            step.starts_with(&format!("name: {name}\n"))
                || step.contains(&format!("\n        name: {name}\n"))
        })
        .unwrap_or_else(|| panic!("missing step {name}"))
}

#[test]
fn native_workflow_partitions_every_os_and_keeps_the_aggregate_fail_closed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = native_workflow();
    // Every OS runs the same checked partitions and one Ubuntu merge; the
    // package shards, the rebuilding collect job and the unsharded coverage
    // job are gone.
    for retired in [
        "\n  coverage:\n",
        "\n  collect:\n",
        "windows-coverage",
        "coverage:windows:",
        "coverage:collect",
        "mise run coverage",
        "matrix.shard",
        "KURU_COVERAGE_SHARD:",
        "inputs.os != 'windows-latest'",
        "inputs.os == 'windows-latest'",
        "KURU_NATIVE_OS",
        "KURU_NATIVE_WINDOWS_",
    ] {
        assert!(!workflow.contains(retired), "retained {retired}");
    }
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let native_tests = ci
        .split("\n  native-tests:\n")
        .nth(1)
        .unwrap()
        .split("\n  native-build:\n")
        .next()
        .unwrap();
    // Every hosted native-tests label in the partition table is called, and
    // nothing else; ubuntu-24.04-arm runs its memory partitions in ci.yml.
    assert!(
        native_tests.contains(
            "        os: [ubuntu-latest, macos-latest, windows-latest, windows-11-arm]\n"
        )
    );
    for label in [
        "ubuntu-latest",
        "macos-latest",
        "windows-latest",
        "windows-11-arm",
    ] {
        assert!(os_target(label).is_some(), "{label} is not a hosted label");
    }
    assert!(native_tests.contains("install: true"));
    for required in [
        "native-gate:",
        "needs: shard\n",
        "needs: [shard, merge, install]",
        "test \"$KURU_NATIVE_SHARDS\" = success",
        "test \"$KURU_NATIVE_REPORT\" = success",
        "test \"$KURU_NATIVE_INSTALL_RESULT\" = success",
        "test \"$KURU_NATIVE_INSTALL_RESULT\" = skipped",
        "KURU_NATIVE_REPORT: ${{ needs.merge.result }}",
        "KURU_COVERAGE_OS: ${{ inputs.os }}",
        "KURU_COVERAGE_TARGET: ${{ runner.temp }}/kuru-coverage-target\n",
        "KURU_COVERAGE_SOURCE: ${{ inputs.ref }}",
        "KURU_COVERAGE_ATTEMPT: ${{ github.run_attempt }}",
        "KURU_COVERAGE_PARTITION: ${{ matrix.partition }}",
        "KURU_COVERAGE_OUTPUT",
        "KURU_COVERAGE_DIAGNOSTICS",
        "KURU_COVERAGE_SEED: ${{ runner.temp }}/kuru-coverage-seed\n",
        "KURU_COVERAGE_HELPER_CACHE: ${{ steps.helper-cache.outputs.cache-hit }}",
        "KURU_COVERAGE_SEED_CACHE: ${{ steps.seed.outputs.cache-hit }}",
        "KURU_COVERAGE_SEED_MATCHED_KEY: ${{ steps.seed.outputs.cache-matched-key }}",
        "KURU_COVERAGE_MODE: ${{ inputs.os == 'windows-11-arm' && 'uninstrumented' || 'instrumented' }}\n",
        "KURU_COVERAGE_INPUTS",
        "KURU_COVERAGE_REPORT",
        "run: mise run //packages/kuru-delivery:coverage:shard",
        "            mise run //packages/kuru-delivery:coverage:shard\n          KURU_NATIVE_STORE",
        "mise run //packages/kuru-delivery:coverage:merge",
        "if-no-files-found: error",
    ] {
        assert!(workflow.contains(required), "missing {required}");
    }
    let shards = workflow_job(&workflow, "shard", "merge");
    let merge = workflow_job(&workflow, "merge", "install");
    // Partitions run on their own OS; every OS merges on Ubuntu from the
    // partitions' exported LCOV, rebuilding and instrumenting nothing.
    assert!(shards.contains("    runs-on: ${{ inputs.os }}\n"));
    assert!(merge.contains("    runs-on: ubuntu-latest\n"));
    assert!(merge.contains(
        "    name: ${{ inputs.os == 'windows-11-arm' && 'Behavior merge' || 'Coverage merge' }} (${{ inputs.os }})\n"
    ));
    assert!(shards.contains(
        "    name: ${{ inputs.os == 'windows-11-arm' && 'Behavior partition' || 'Coverage partition' }} (${{ inputs.os }}, ${{ matrix.partition }})\n"
    ));
    for forbidden in [
        "cargo-llvm-cov",
        "component add llvm-tools",
        "bundle:prepare",
        "coverage:shard",
        "CARGO_PROFILE_TEST_DEBUG: ${{",
    ] {
        assert!(!merge.contains(forbidden), "merge still uses {forbidden}");
    }
    assert!(merge.contains("install_args: rust\n"));
    for job in [shards, merge] {
        assert!(!job.contains("inputs.install"));
        for moved in ["mise run install", "test:embedded-runtime"] {
            assert!(!job.contains(moved), "coverage still runs {moved}");
        }
    }
    // The matrix and the declared partition count are exactly the table the
    // receipt and merge code enforce, so the workflow cannot drift from it.
    let ubuntu = table_count("ubuntu-latest", Mode::Instrumented);
    let macos = table_count("macos-latest", Mode::Instrumented);
    let windows = table_count("windows-latest", Mode::Instrumented);
    let windows_arm = table_count("windows-11-arm", Mode::Uninstrumented);
    assert_eq!(ubuntu, windows, "the non-macOS branch names one count");
    assert_eq!(ubuntu, windows_arm, "the non-macOS branch names one count");
    assert!(shards.contains(&format!(
        "        partition: ${{{{ fromJSON(inputs.os == 'macos-latest' && '{}' || '{}') }}}}\n",
        partition_list(macos),
        partition_list(ubuntu)
    )));
    let count = format!(
        "KURU_COVERAGE_PARTITIONS: ${{{{ inputs.os == 'macos-latest' && '{macos}' || '{ubuntu}' }}}}\n"
    );
    assert_eq!(shards.matches(count.as_str()).count(), 3);
    assert_eq!(merge.matches(count.as_str()).count(), 1);
    // Every partition and the merge share one helper key per OS; exactly
    // partition 1 saves it. The merge keeps its own helper cache.
    assert_eq!(
        workflow
            .matches("shared-key: native-coverage-${{ inputs.os }}\n")
            .count(),
        1
    );
    assert_eq!(
        shards
            .matches(
                "save-if: ${{ matrix.partition == 1 && github.ref == 'refs/heads/main' }}
"
            )
            .count(),
        1
    );
    assert!(merge.contains("shared-key: native-coverage-merge\n"));
    // The seed restores and saves the same path under one pinned action,
    // exports only from main partition 1 on a cache miss, and saves only a
    // staged export, never the consumed remainder of a restored seed.
    let steps = workflow_steps(shards);
    // Each OS and mode seeds its own dependency outputs: Windows on Arm's
    // uninstrumented partitions never restore an instrumented seed.
    let restore = named_step(&steps, "Restore the dependency seed");
    for required in [
        "uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "path: ${{ runner.temp }}/kuru-coverage-seed\n",
        "key: kuru-coverage-seed-v1-${{ inputs.os }}-${{ env.KURU_NATIVE_MODE }}-${{ hashFiles('Cargo.lock', 'mise.toml', 'mise.lock') }}",
        "restore-keys: kuru-coverage-seed-v1-${{ inputs.os }}-${{ env.KURU_NATIVE_MODE }}-",
    ] {
        assert!(restore.contains(required), "seed restore lost {required}");
    }
    assert!(restore.starts_with("id: seed\n"));
    assert!(named_step(&steps, "Select the dependency seed export").contains(
        "if: github.ref == 'refs/heads/main' && matrix.partition == 1 && steps.seed.outputs.cache-hit != 'true'"
    ));
    let stage = named_step(&steps, "Stage the exported dependency seed");
    assert!(stage.contains("if: env.KURU_COVERAGE_SEED_EXPORT != ''"));
    assert!(stage.contains("rm -rf -- \"$RUNNER_TEMP/kuru-coverage-seed\""));
    let save = named_step(&steps, "Save the dependency seed");
    for required in [
        "if: env.KURU_COVERAGE_SEED_READY == 'true'",
        "uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "path: ${{ runner.temp }}/kuru-coverage-seed\n",
        "key: ${{ steps.seed.outputs.cache-primary-key }}",
    ] {
        assert!(save.contains(required), "seed save lost {required}");
    }
    let position = |name: &str| {
        steps
            .iter()
            .position(|step| step.starts_with(&format!("name: {name}\n")))
            .unwrap_or_else(|| panic!("missing step {name}"))
    };
    assert!(
        position("Run one checked coverage partition with the OS secret store")
            < position("Stage the exported dependency seed")
    );
    // The merge downloads every partition's attempts in one pattern step;
    // the only other downloads are the partitions' run-verified bundle
    // inputs and the Windows on Arm engine imports of the partitions and the
    // install job.
    assert_eq!(
        workflow
            .matches("actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c")
            .count(),
        4
    );
    assert_eq!(workflow.matches("actions/download-artifact@").count(), 4);
    assert_eq!(merge.matches("actions/download-artifact@").count(), 1);
    assert!(merge.contains(
        "          pattern: ${{ inputs.artifact-prefix }}-coverage-${{ inputs.os }}-partition-*\n          merge-multiple: false\n          path: ${{ runner.temp }}/kuru-coverage-inputs\n"
    ));
    // Only a successful partition publishes receipt evidence under the name
    // the merge accepts; failures publish diagnostics under a name it rejects.
    assert_eq!(workflow.matches("if: ${{ !cancelled() }}").count(), 1);
    assert!(!shards.contains("if: ${{ !cancelled() }}"));
    // A host cancel or the job time limit also ends a stalled partition, so
    // its diagnostics upload runs on cancellation as well as on failure.
    assert_eq!(
        shards
            .matches("if: ${{ failure() || cancelled() }}")
            .count(),
        1
    );
    assert!(!shards.contains("if: ${{ failure() }}"));
    assert!(shards.contains(
        "name: ${{ inputs.artifact-prefix }}-coverage-diagnostics-${{ inputs.os }}-partition-${{ matrix.partition }}-attempt-${{ github.run_attempt }}"
    ));
    assert!(shards.contains(
        "name: ${{ inputs.artifact-prefix }}-coverage-${{ inputs.os }}-partition-${{ matrix.partition }}-attempt-${{ github.run_attempt }}"
    ));
    assert!(merge.contains(
        "name: ${{ inputs.artifact-prefix }}-coverage-${{ inputs.os }}-attempt-${{ github.run_attempt }}"
    ));
    // The job start is recorded before any other step, and the inner deadline
    // derives from the same limit the host enforces.
    assert!(steps[0].starts_with("name: Record the job start for the inner test deadline\n"));
    assert!(steps[0].contains("KURU_COVERAGE_JOB_STARTED=%s"));
    let timeout: Vec<_> = shards
        .lines()
        .filter_map(|line| line.trim().strip_prefix("timeout-minutes: "))
        .collect();
    assert_eq!(timeout, ["45"]);
    let deadlines: Vec<_> = shards
        .lines()
        .filter_map(|line| line.trim().strip_prefix("KURU_COVERAGE_JOB_MINUTES: \""))
        .map(|value| value.strip_suffix('"').unwrap())
        .collect();
    assert_eq!(deadlines, [timeout[0], timeout[0], timeout[0]]);
    // The Linux secret-store leg and the other runners run the same
    // partition under an identical environment, selected by runner.os.
    let other = named_step(&steps, "Run one checked coverage partition");
    let linux = named_step(
        &steps,
        "Run one checked coverage partition with the OS secret store",
    );
    assert!(other.contains(
        "if: runner.os != 'Linux' && !(runner.os == 'Windows' && runner.arch == 'ARM64')\n"
    ));
    assert!(linux.contains("if: runner.os == 'Linux'"));
    assert_eq!(step_env(other), step_env(linux));
    assert!(step_env(other).contains(
        "CARGO_PROFILE_TEST_DEBUG: ${{ runner.os == 'Linux' && '0' || 'line-tables-only' }}\n"
    ));
    // An older successful attempt must not stand in for a partition whose
    // latest attempt failed: the merge runs even after failures, then
    // refuses before any other step unless every partition job succeeded.
    assert!(merge.starts_with("    if: always()\n    needs: shard\n"));
    let merge_steps = workflow_steps(merge);
    assert!(
        merge_steps[0]
            .starts_with("name: Require every coverage partition job to have succeeded\n")
    );
    assert!(merge_steps[0].contains("KURU_COVERAGE_SHARDS_RESULT: ${{ needs.shard.result }}"));
    assert!(merge_steps[0].contains("if [ \"$KURU_COVERAGE_SHARDS_RESULT\" != success ]; then\n"));
    // Every step-level OS selection uses the runner, never the caller's label.
    for line in workflow.lines() {
        if line.starts_with("        if:") {
            assert!(!line.contains("inputs.os"), "step condition {line}");
        }
    }
}

#[test]
fn windows_on_arm_partitions_are_uninstrumented_behavioral_evidence_with_an_imported_engine() {
    let workflow = native_workflow();
    let label = "windows-11-arm";
    // The table is the fail-closed allowlist: Windows on Arm has only an
    // uninstrumented set (rust-lang/rust#150123 holds instrumentation) with
    // the Windows partition count and the native arm64 host target.
    assert_eq!(partition_count(label, Mode::Instrumented), None);
    assert_eq!(
        partition_count(label, Mode::Uninstrumented),
        partition_count("windows-latest", Mode::Instrumented)
    );
    assert_eq!(os_target(label), Some("aarch64-pc-windows-msvc"));

    // One engine job, selected by the caller's label, calls the single
    // pin-verifying bundle-build implementation at the validated commit.
    let engine = workflow_job(&workflow, "dolt-windows-arm64", "shard");
    assert_eq!(
        engine,
        "    if: inputs.os == 'windows-11-arm'\n    uses: ./.github/workflows/bundle-build.yml\n    with:\n      ref: ${{ inputs.ref }}\n"
    );
    // Partitions and installation wait for it on Windows on Arm, require its
    // skip everywhere else and never run after a cancellation.
    let requires_engine = "if: ${{ !cancelled() && needs.dolt-windows-arm64.result == (inputs.os == 'windows-11-arm' && 'success' || 'skipped') }}\n";
    let shards = workflow_job(&workflow, "shard", "merge");
    let merge = workflow_job(&workflow, "merge", "install");
    let install = workflow_job(&workflow, "install", "native-gate");
    assert!(shards.contains(&format!(
        "    needs: dolt-windows-arm64\n    {requires_engine}"
    )));
    assert!(install.starts_with(
        "    needs: dolt-windows-arm64\n    if: ${{ inputs.install && !cancelled() && needs.dolt-windows-arm64.result == (inputs.os == 'windows-11-arm' && 'success' || 'skipped') }}\n"
    ));
    assert!(merge.starts_with("    if: always()\n    needs: shard\n"));
    // The mode follows the label table; the merge and partitions agree on it.
    let mode = "${{ inputs.os == 'windows-11-arm' && 'uninstrumented' || 'instrumented' }}\n";
    assert!(shards.contains(&format!("      KURU_NATIVE_MODE: {mode}")));
    assert!(merge.contains(&format!("          KURU_COVERAGE_MODE: {mode}")));

    // The distinct uninstrumented task runs every workspace package, with
    // the coverage partition's inputs and no coverage tooling.
    let packages = WORKSPACE_PACKAGES.join(",");
    assert_eq!(
        workflow.matches("KURU_COVERAGE_PACKAGES").count(),
        2,
        "only the Windows on Arm partition and merge name a package scope"
    );
    assert!(merge.contains(&format!(
        "          KURU_COVERAGE_PACKAGES: ${{{{ inputs.os == 'windows-11-arm' && '{packages}' || '' }}}}\n"
    )));
    let steps = workflow_steps(shards);
    let arm = named_step(
        &steps,
        "Run one checked uninstrumented Windows on Arm partition",
    );
    assert!(arm.contains("        if: runner.os == 'Windows' && runner.arch == 'ARM64'\n"));
    assert!(arm.contains(&format!("          KURU_COVERAGE_PACKAGES: {packages}\n")));
    assert!(arm.ends_with("        run: mise run //packages/kuru-delivery:test:partition"));
    assert!(!arm.contains("coverage:shard"));
    let other = step_env(named_step(&steps, "Run one checked coverage partition"));
    let arm_env = step_env(arm);
    for line in other
        .lines()
        .filter(|line| line.trim().starts_with("KURU_COVERAGE_"))
    {
        assert!(
            arm_env.contains(line),
            "Windows on Arm partition lost {line}"
        );
    }
    assert!(
        named_step(&steps, "Install coverage components")
            .contains("if: env.KURU_NATIVE_MODE == 'instrumented'\n")
    );
    // Only instrumented partitions install cargo-llvm-cov; the Arm task
    // declares no coverage tool, so its mise setup omits it as well.
    assert!(shards.contains(
        "          install_args: ${{ inputs.os == 'windows-11-arm' && 'rust github:aligned-team/cospec' || 'rust aqua:taiki-e/cargo-llvm-cov github:aligned-team/cospec' }}\n"
    ));
    assert_eq!(shards.matches("install_args:").count(), 1);

    // Each Windows on Arm job imports the pinned engine before anything
    // builds; bundle:prepare checks the committed pin again.
    let download = "        if: runner.os == 'Windows' && runner.arch == 'ARM64'\n        uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1\n        with:\n          name: ${{ needs.dolt-windows-arm64.outputs.artifact }}\n          path: ${{ runner.temp }}/bundle-input";
    let import = "        if: runner.os == 'Windows' && runner.arch == 'ARM64'\n        shell: pwsh\n        run: |\n          $archive = Join-Path $env:RUNNER_TEMP 'bundle-input/dolt-windows-arm64.zip'\n          mise run //packages/kuru-memory:bundle:prepare -- --target aarch64-pc-windows-msvc --archive $archive --offline\n          if ($LASTEXITCODE -ne 0) { throw 'Windows arm64 engine import failed' }";
    for (job, first_build) in [
        (
            shards,
            "Run one checked uninstrumented Windows on Arm partition",
        ),
        (
            install,
            "Fetch locked Cargo inputs before offline verification",
        ),
    ] {
        let steps = workflow_steps(job);
        let position = |name: &str| {
            steps
                .iter()
                .position(|step| step.starts_with(&format!("name: {name}\n")))
                .unwrap_or_else(|| panic!("missing step {name}"))
        };
        let fetched = position("Download the Windows arm64 engine input");
        let imported = position("Import the pin-verified Windows arm64 engine");
        assert!(steps[fetched].ends_with(download), "{}", steps[fetched]);
        assert!(steps[imported].ends_with(import), "{}", steps[imported]);
        assert_eq!(fetched + 1, imported);
        assert!(imported < position(first_build));
        let tools = steps
            .iter()
            .position(|step| step.starts_with("uses: jdx/mise-action@"))
            .unwrap();
        assert!(tools < fetched);
    }
    assert_eq!(workflow.matches(import).count(), 2);
}

/// The target set each partition label imports: its host engine unless that
/// engine is built from source (imported from bundle-build instead), plus
/// the Windows x64 archive that the decoder tests read on every host.
fn expected_bundle_inputs(label: &str) -> Vec<String> {
    const FIXTURE: &str = "x86_64-pc-windows-msvc";
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(root.join("packages/kuru-memory/support/dolt-assets.json")).unwrap(),
    )
    .unwrap();
    let host = os_target(label).unwrap();
    let provenance = manifest["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["target"] == host)
        .unwrap_or_else(|| panic!("no manifest asset for {host}"))["provenance"]
        .clone();
    let mut targets = vec![FIXTURE.to_owned()];
    if provenance == "upstream" && host != FIXTURE {
        targets.insert(0, host.to_owned());
    }
    targets
}

#[test]
fn partitions_import_bundle_inputs_one_job_of_the_run_fetched_and_verified() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let job = |name: &str, next: &str| {
        ci.split(&format!("\n  {name}:\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("missing CI job {name}"))
            .split(&format!("\n  {next}:\n"))
            .next()
            .unwrap()
            .to_owned()
    };
    // One job prepares every upstream archive once through the verifying
    // package task and publishes one set per partition label.
    let fetch = job("bundle-inputs", "native-tests");
    assert!(fetch.contains("    runs-on: ubuntu-latest\n"));
    assert!(
        !fetch.contains("strategy:"),
        "the fetch job must not fan out"
    );
    assert!(fetch.contains(
        "            mise run //packages/kuru-memory:bundle:prepare -- --target \"$target\"\n"
    ));
    assert!(!fetch.contains("KURU_DOLT_BUNDLE_OFFLINE"));
    let sets: Vec<(String, Vec<String>)> = fetch
        .lines()
        .filter_map(|line| line.trim().strip_prefix('['))
        .filter_map(|line| line.split_once("]=\""))
        .map(|(label, targets)| {
            let targets = targets
                .trim_end_matches('"')
                .split_whitespace()
                .map(|target| target.replace("$fixture", "x86_64-pc-windows-msvc"))
                .collect();
            (label.to_owned(), targets)
        })
        .collect();
    assert_eq!(sets.len(), OS_TARGETS.len());
    for (label, _) in OS_TARGETS {
        let (_, targets) = sets
            .iter()
            .find(|(name, _)| name == label)
            .unwrap_or_else(|| panic!("no bundle input set for {label}"));
        assert_eq!(targets, &expected_bundle_inputs(label), "{label}");
        assert!(fetch.contains(&format!(
            "          name: ci-bundle-inputs-{label}\n          path: ${{{{ runner.temp }}}}/kuru-bundle-inputs/{label}\n          if-no-files-found: error\n"
        )));
    }
    assert_eq!(
        fetch
            .matches("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a")
            .count(),
        OS_TARGETS.len()
    );
    assert!(
        !fetch.contains("run_attempt"),
        "reruns import the run's inputs"
    );

    // Every partition job waits for that job, imports its label's set with
    // --archive --offline before anything builds, and cannot download.
    let import = "        run: |\n          # bundle:prepare checks each archive against its committed pin again.\n";
    let command = "mise run //packages/kuru-memory:bundle:prepare -- --target \"${archive%.archive}\" --archive \"$RUNNER_TEMP/bundle-inputs/$archive\" --offline\n";
    let native = job("native-tests", "native-build");
    assert!(native.starts_with("    needs: bundle-inputs\n"));
    assert!(native.contains("      bundle-inputs: ci-bundle-inputs\n"));
    let memory = job("native-memory", "native-memory-merge");
    assert!(memory.contains("    needs: bundle-inputs\n"));
    let workflow = native_workflow();
    let shards = workflow_job(&workflow, "shard", "merge");
    for (partitions, name, first_build) in [
        (
            shards,
            "${{ inputs.bundle-inputs }}-${{ inputs.os }}",
            "Run one checked coverage partition",
        ),
        (
            memory.as_str(),
            "ci-bundle-inputs-ubuntu-24.04-arm",
            "Run one checked memory partition",
        ),
    ] {
        assert!(partitions.contains("      KURU_DOLT_BUNDLE_OFFLINE: \"true\"\n"));
        let steps = workflow_steps(partitions);
        let position = |step: &str| {
            steps
                .iter()
                .position(|body| body.starts_with(&format!("name: {step}\n")))
                .unwrap_or_else(|| panic!("missing step {step}"))
        };
        let fetched = position("Download the run's verified bundle inputs");
        let imported = position("Import the run's verified bundle inputs");
        assert!(steps[fetched].contains(&format!(
            "          name: {name}\n          path: ${{{{ runner.temp }}}}/bundle-inputs"
        )));
        assert!(!steps[fetched].contains("if:"), "every partition imports");
        assert!(steps[imported].contains(import));
        assert!(steps[imported].contains(command));
        assert!(!steps[imported].contains("if:"));
        assert_eq!(fetched + 1, imported);
        assert!(imported < position(first_build));
        let tools = steps
            .iter()
            .position(|step| step.starts_with("uses: jdx/mise-action@"))
            .unwrap();
        assert!(tools < fetched);
    }
    let native_inputs = workflow.split("\n      bundle-inputs:\n").nth(1).unwrap();
    assert!(native_inputs.contains("        required: true\n"));
}

/// Splits workflow text into whole step blocks (the same `\n      - `
/// step-level marker `workflow_steps` uses) and keeps only the ones that
/// contain a `uses: Swatinem/rust-cache@...` key, however it is ordered
/// within the step (first key or after `name:`). Each returned block is
/// exactly one step, bounded by the next step marker or the file end, so a
/// following `- run:` or `- uses:` step can never be read as part of it.
fn rust_cache_blocks(text: &str) -> Vec<&str> {
    text.split("\n      - ")
        .skip(1)
        .filter(|block| block.contains("uses: Swatinem/rust-cache@"))
        .collect()
}

/// Extracts a rust-cache step block's own `save-if:` value (never one merely
/// mentioned in a comment line), trimmed and with a surrounding `${{ }}`
/// template stripped so callers compare against the bare expression.
fn save_if_value(block: &str) -> Option<&str> {
    let raw = block
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("save-if:").map(str::trim))?;
    Some(
        raw.strip_prefix("${{")
            .and_then(|value| value.strip_suffix("}}"))
            .map(str::trim)
            .unwrap_or(raw),
    )
}

/// A `save-if:` value restricts saves to `main` when it is exactly `false`,
/// solely the main-ref check, or that check combined with another condition
/// through `&&`. A bare mention of the ref check without `&&` (or without
/// being the whole expression) does not count, since that shape cannot arise
/// from a deliberate combination.
fn save_if_restricts_to_main(value: &str) -> bool {
    const MAIN_REF: &str = "github.ref == 'refs/heads/main'";
    value == "false" || value == MAIN_REF || (value.contains(MAIN_REF) && value.contains("&&"))
}

#[test]
fn every_rust_cache_step_restricts_saves_to_main() {
    // Every Swatinem/rust-cache step across the CI, quality and native-tests
    // workflows must only save from `main` (or never save at all), so PR
    // runs restore the warm cache instead of thrashing it. A step whose
    // save-if already carries another condition (e.g. one coverage shard)
    // must combine it with the main-ref check rather than drop it.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflows = root.join(".github/workflows");
    for name in ["ci.yml", "quality.yml", "native-tests.yml"] {
        let text = fs::read_to_string(workflows.join(name)).unwrap();
        let blocks = rust_cache_blocks(&text);
        assert!(!blocks.is_empty(), "{name} has no rust-cache steps");
        for (index, block) in blocks.iter().enumerate() {
            let value = save_if_value(block).unwrap_or_else(|| {
                panic!("{name} rust-cache step {index} has no save-if:\n{block}")
            });
            assert!(
                save_if_restricts_to_main(value),
                "{name} rust-cache step {index} does not restrict saves to main (save-if: {value}):\n{block}"
            );
        }
    }
}

#[test]
fn rust_cache_step_without_save_if_has_no_extracted_value() {
    // A rust-cache step that never sets save-if must be rejected by the
    // regression test above (via the `unwrap_or_else` panic), not silently
    // treated as restricted.
    let yaml = "\
  job:
    steps:
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          cache-bin: false
          shared-key: example
      - run: mise run lint:rust
";
    let blocks = rust_cache_blocks(yaml);
    assert_eq!(blocks.len(), 1);
    assert!(save_if_value(blocks[0]).is_none());
}

#[test]
fn rust_cache_step_with_main_ref_only_in_a_comment_is_not_restricted() {
    // The main-ref check must come from the actual save-if value, not merely
    // appear anywhere in the step's text (e.g. an explanatory comment). This
    // also proves the block boundary stops at the next `- run:` step instead
    // of running on into it.
    let yaml = "\
  job:
    steps:
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          cache-bin: false
          # Only main saves: github.ref == 'refs/heads/main'
          save-if: true
      - run: mise run lint:rust
";
    let blocks = rust_cache_blocks(yaml);
    assert_eq!(blocks.len(), 1);
    assert!(!blocks[0].contains("mise run lint:rust"));
    let value = save_if_value(blocks[0]).unwrap();
    assert_eq!(value, "true");
    assert!(!save_if_restricts_to_main(value));
}

#[test]
fn rust_cache_step_named_before_uses_still_extracts_its_save_if() {
    // `uses:` need not be the step's first key (e.g. a `name:` line comes
    // first, as in the real windows platform step); the block must still
    // capture the whole step, including a save-if several keys later.
    let yaml = "\
  job:
    steps:
      - name: Cache platform Rust dependencies
        uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          cache-bin: false
          shared-key: native-platform-windows
          save-if: ${{ github.ref == 'refs/heads/main' }}
      - name: Test native platform primitives
        run: mise run //packages/kuru-platform:coverage
";
    let blocks = rust_cache_blocks(yaml);
    assert_eq!(blocks.len(), 1);
    let value = save_if_value(blocks[0]).unwrap();
    assert!(save_if_restricts_to_main(value));
}

#[test]
fn arm64_memory_suite_runs_as_gated_uninstrumented_partitions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let job = |name: &str, next: &str| {
        ci.split(&format!("\n  {name}:\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("missing CI job {name}"))
            .split(&format!("\n  {next}:\n"))
            .next()
            .unwrap()
            .to_owned()
    };
    // The memory suite moved out of the arm64 build job into partitions.
    let build = job("native-build", "native-memory");
    assert!(!build.contains("//packages/kuru-memory:test"));
    assert!(build.contains("test:embedded-runtime"));
    let partitions = job("native-memory", "native-memory-merge");
    let merge = job("native-memory-merge", "native-platform");
    let count = table_count("ubuntu-24.04-arm", Mode::Uninstrumented);
    assert!(partitions.contains(&format!(
            "        partition: [{}]\n",
            (1..=count)
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    assert!(partitions.contains("    runs-on: ubuntu-24.04-arm\n"));
    assert!(partitions.contains(
        "    name: Native memory partition (ubuntu-24.04-arm, ${{ matrix.partition }})\n"
    ));
    let declared = format!("KURU_COVERAGE_PARTITIONS: \"{count}\"\n");
    assert!(partitions.contains(&declared));
    assert!(merge.contains(&declared));
    for required in [
        "install_args: rust\n",
        "run: mise run //packages/kuru-delivery:setup:test-tools",
        "KURU_COVERAGE_OS: ubuntu-24.04-arm\n",
        "KURU_COVERAGE_PACKAGES: kuru-memory\n",
        "KURU_COVERAGE_TARGET: ${{ runner.temp }}/kuru-coverage-target\n",
        "KURU_COVERAGE_JOB_MINUTES: \"45\"",
        "timeout-minutes: 45\n",
        "key: kuru-coverage-seed-v1-ubuntu-24.04-arm-uninstrumented-",
        "uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "run: mise run //packages/kuru-delivery:test:partition",
        "name: ci-coverage-ubuntu-24.04-arm-partition-${{ matrix.partition }}-attempt-${{ github.run_attempt }}",
    ] {
        assert!(
            partitions.contains(required),
            "memory partitions lost {required}"
        );
    }
    // Uninstrumented: no coverage tooling, profiles or instrumented task.
    for forbidden in [
        "cargo-llvm-cov",
        "component add llvm-tools",
        "coverage:shard",
        "CARGO_PROFILE_TEST_DEBUG",
    ] {
        assert!(
            !partitions.contains(forbidden),
            "memory partitions use {forbidden}"
        );
    }
    assert!(merge.starts_with("    if: always()\n    needs: native-memory\n"));
    for required in [
        "    runs-on: ubuntu-latest\n",
        "KURU_COVERAGE_SHARDS_RESULT: ${{ needs.native-memory.result }}",
        "pattern: ci-coverage-ubuntu-24.04-arm-partition-*\n",
        "KURU_COVERAGE_MODE: uninstrumented\n",
        // The merge refuses partitions of any other scope.
        "KURU_COVERAGE_PACKAGES: kuru-memory\n",
        "mise run //packages/kuru-delivery:coverage:merge",
    ] {
        assert!(merge.contains(required), "memory merge lost {required}");
    }
    assert!(
        merge
            .split("    steps:\n")
            .nth(1)
            .unwrap()
            .starts_with("      - name: Require every memory partition job to have succeeded\n")
    );
    assert!(ci.contains(
        "needs: [bundle-inputs, quality, windows-lint, native-tests, native-build, native-memory, native-memory-merge, native-platform, usage-scan-scaling]"
    ));
}

#[test]
fn native_platform_runs_windows_on_arm_as_separately_named_behavioral_evidence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let job = workflow_job(&ci, "native-platform", "usage-scan-scaling");
    // One job, two legs: the x64 leg keeps the 90% platform gate and Windows
    // on Arm runs the distinct uninstrumented task under its own name.
    for required in [
        "    name: ${{ matrix.mode == 'coverage' && 'Native platform primitives' || 'Native platform behavior' }} (${{ matrix.target }})\n",
        "    runs-on: ${{ matrix.os }}\n",
        "      fail-fast: false\n",
        "        include:\n          - os: windows-latest\n            target: x86_64-pc-windows-msvc\n            mode: coverage\n          - os: windows-11-arm\n            target: aarch64-pc-windows-msvc\n            mode: behavior\n",
        "      CARGO_BUILD_TARGET: ${{ matrix.target }}\n",
        "install_args: ${{ matrix.mode == 'coverage' && 'rust aqua:taiki-e/cargo-llvm-cov' || 'rust' }}\n",
        "shared-key: native-platform-${{ matrix.target }}\n",
    ] {
        assert!(job.contains(required), "native-platform lost {required}");
    }
    for label in ["windows-latest", "windows-11-arm"] {
        assert!(os_target(label).is_some(), "{label} is not a hosted label");
    }
    let steps = workflow_steps(job);
    let coverage = named_step(
        &steps,
        "Test native platform primitives and require 90% coverage",
    );
    assert_eq!(
        coverage,
        "name: Test native platform primitives and require 90% coverage\n        if: matrix.mode == 'coverage'\n        run: mise run //packages/kuru-platform:coverage"
    );
    // Never the coverage task with instrumentation quietly off.
    let behavior = named_step(
        &steps,
        "Test native platform primitives uninstrumented as behavioral evidence",
    );
    assert_eq!(
        behavior,
        "name: Test native platform primitives uninstrumented as behavioral evidence\n        if: matrix.mode == 'behavior'\n        run: mise run //packages/kuru-platform:test"
    );
    assert!(
        named_step(&steps, "Set up platform Rust components")
            .contains("        if: matrix.mode == 'coverage'\n")
    );
    let upload = named_step(&steps, "Upload native platform coverage");
    assert!(upload.contains("        if: ${{ matrix.mode == 'coverage' && !cancelled() && "));
    assert!(upload.contains("name: coverage-native-platform-${{ matrix.target }}\n"));
    assert!(
        named_step(&steps, "Require unchanged dependency locks")
            .ends_with("run: git diff --exit-code -- mise.lock Cargo.lock")
    );
    assert!(!job.contains("KURU_COVERAGE_"));
    assert!(ci.contains(
        "needs: [bundle-inputs, quality, windows-lint, native-tests, native-build, native-memory, native-memory-merge, native-platform, usage-scan-scaling]"
    ));
}

/// The usage-scan scaling check gates CI from the day it lands: a required
/// Ubuntu job imports the run's verified engine inputs, ages its fixture
/// in-job on every run and asserts the calibrated bounds, uploading its
/// evidence on any outcome. The aged fixture never enters the shared Actions
/// cache: even after DOLT_GC it exceeds the cache budget.
#[test]
fn usage_scan_scaling_is_a_required_job_that_ages_its_fixture_uncached() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let job = workflow_job(&ci, "usage-scan-scaling", "ci-gate");
    for required in [
        "    name: Usage scan scaling (ubuntu-latest, calibrated bounds)\n",
        "    needs: bundle-inputs\n",
        "    runs-on: ubuntu-latest\n",
        "    timeout-minutes: 45\n",
        "      KURU_DOLT_BUNDLE_OFFLINE: \"true\"\n",
        "          install_args: rust\n",
        "          shared-key: usage-scan-scaling\n",
        "          save-if: ${{ github.ref == 'refs/heads/main' }}\n",
        "printf 'KURU_DOLT_BUNDLE_DIR=%s/kuru-bundles-usage-scan-${{ github.run_attempt }}\\n' \"$RUNNER_TEMP\"\n",
        "name: ci-usage-scan-attempt-${{ github.run_attempt }}\n",
    ] {
        assert!(job.contains(required), "usage-scan-scaling lost {required}");
    }
    for forbidden in [
        "continue-on-error",
        "restore-keys:",
        "KURU_OPEN_TIMELINE",
        "actions/cache/restore@",
        "actions/cache/save@",
        "actions/cache@",
        "steps.fixture",
        "cache-hit",
        "-- key --root",
    ] {
        assert!(
            !job.contains(forbidden),
            "usage-scan-scaling uses {forbidden}"
        );
    }
    let steps = workflow_steps(job);
    let position = |name: &str| {
        steps
            .iter()
            .position(|step| step.contains(&format!("name: {name}\n")))
            .unwrap_or_else(|| panic!("missing step {name}"))
    };
    let imported = position("Import the run's verified bundle inputs");
    let created = position("Build the release tooling and create the fixture stores");
    let aged = position("Age the fixture stores in-job");
    let measured = position("Measure and assert the usage scan growth (calibrated bounds)");
    let uploaded = position("Upload the usage scan records and timelines");
    assert!(imported < created && created < aged && aged < measured && measured < uploaded);
    assert!(
        steps[created].contains(
            "mise run //packages/kuru-memory:measure:usage-scan:fixture -- create --root"
        )
    );
    // Every run ages in-job and never skips the measurement.
    for step in [created, aged, measured] {
        assert!(!steps[step].contains("\n        if:"));
    }
    assert!(
        steps[aged]
            .contains("mise run //packages/kuru-memory:measure:age-store -- --profile release")
    );
    assert!(steps[aged].contains("-- seal --root"));
    assert!(
        steps[measured].contains("mise run //packages/kuru-memory:measure:usage-scan -- --root")
    );
    assert!(steps[measured].contains("--samples 5 --assert"));
    assert!(steps[uploaded].contains("if: always()\n"));
    // The measuring task alone sets the gate; the fixture task never does.
    let mise = fs::read_to_string(root.join("packages/kuru-memory/mise.toml")).unwrap();
    let task = |name: &str| {
        mise.split(&format!("[tasks.\"{name}\"]\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("missing task {name}"))
            .split("\n[")
            .next()
            .unwrap()
            .to_owned()
    };
    assert!(task("measure:usage-scan").contains("env.KURU_OPEN_TIMELINE = \"1\"\n"));
    assert!(!task("measure:usage-scan:fixture").contains("KURU_OPEN_TIMELINE"));
    // Coverage partitions run gated owners, task-scoped: never job-wide and
    // never the uninstrumented partitions.
    let delivery = fs::read_to_string(root.join("packages/kuru-delivery/mise.toml")).unwrap();
    let section = |name: &str| {
        delivery
            .split(&format!("[{name}]\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("missing section {name}"))
            .split("\n[")
            .next()
            .unwrap()
            .to_owned()
    };
    assert!(section("tasks.\"coverage:shard\".env").contains("\nKURU_OPEN_TIMELINE = \"1\"\n"));
    for name in [
        "tasks.\"coverage:shard\"",
        "tasks.\"test:partition\"",
        "tasks.\"test:partition\".env",
    ] {
        assert!(!section(name).contains("KURU_OPEN_TIMELINE"), "{name}");
    }
    assert!(ci.contains(
        "needs: [bundle-inputs, quality, windows-lint, native-tests, native-build, native-memory, native-memory-merge, native-platform, usage-scan-scaling]"
    ));
}

/// Rust compiles `cfg(windows)` items only for a Windows target, so the
/// Ubuntu Lint job cannot see them. One static CI job lints every package
/// that has such code for the Windows target, offline, and the gate needs it.
#[test]
fn windows_only_rust_is_linted_by_a_required_static_job() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let job = workflow_job(&ci, "windows-lint", "bundle-inputs");
    for required in [
        "    name: Lint (x86_64-pc-windows-msvc)\n",
        "    needs: bundle-inputs\n",
        "    runs-on: ubuntu-latest\n",
        "      KURU_DOLT_BUNDLE_OFFLINE: \"true\"\n",
        "          install_args: rust\n",
        "          shared-key: windows-lint\n",
    ] {
        assert!(job.contains(required), "windows-lint lost {required}");
    }
    let steps = workflow_steps(job);
    let position = |name: &str| {
        steps
            .iter()
            .position(|step| step.contains(&format!("name: {name}\n")))
            .unwrap_or_else(|| panic!("missing step {name}"))
    };
    let fetched = position("Download the run's verified bundle inputs");
    let imported = position("Import the run's verified bundle inputs");
    let linted = position("Lint Windows-only Rust");
    assert!(steps[fetched].contains("          name: ci-bundle-inputs-ubuntu-latest\n"));
    assert!(
        steps[imported].contains("--archive \"$RUNNER_TEMP/bundle-inputs/$archive\" --offline\n")
    );
    assert!(fetched < imported && imported < linted);
    assert!(steps[linted].ends_with("run: mise run lint:windows"));
    assert!(
        named_step(&steps, "Install the Windows standard library and Clippy")
            .contains("rustup target add x86_64-pc-windows-msvc --toolchain 1.98.1\n")
    );
    assert!(ci.contains(
        "needs: [bundle-inputs, quality, windows-lint, native-tests, native-build, native-memory, native-memory-merge, native-platform, usage-scan-scaling]"
    ));

    // The root task aggregates package-owned tasks, and every package with
    // Windows-only Rust owns one for the MSVC target with warnings denied.
    let mise = fs::read_to_string(root.join("mise.toml")).unwrap();
    assert!(mise.contains(
        "[tasks.\"lint:windows\"]\ndescription = \"Lint Windows-only Rust for x86_64-pc-windows-msvc in every package that owns the task\"\ndepends = [\"//apps/kuru-tui:lint:windows\", \"//packages/*:lint:windows\"]\n"
    ));
    for member in ["apps/kuru-tui"].into_iter().map(str::to_owned).chain(
        WORKSPACE_PACKAGES
            .iter()
            .filter(|package| **package != "kuru")
            .map(|package| format!("packages/{package}")),
    ) {
        let directory = root.join(&member);
        let mut pending = vec![directory.join("src"), directory.join("tests")];
        let mut windows = false;
        while let Some(path) = pending.pop() {
            let Ok(entries) = fs::read_dir(&path) else {
                continue;
            };
            for entry in entries {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    windows |= fs::read_to_string(&path).unwrap().contains("cfg(windows)");
                }
            }
        }
        let tasks = fs::read_to_string(directory.join("mise.toml")).unwrap();
        let task = tasks.split("[tasks.\"lint:windows\"]\n").nth(1);
        assert_eq!(task.is_some(), windows, "{member}: lint:windows ownership");
        if let Some(task) = task {
            let task = task.split("\n[").next().unwrap();
            let package = if member == "apps/kuru-tui" {
                "kuru"
            } else {
                member.trim_start_matches("packages/")
            };
            let command = format!(
                "cargo clippy -p {package} --target x86_64-pc-windows-msvc --all-targets --all-features --locked -- -D warnings"
            );
            assert!(task.contains(&command), "{member}: {task}");
        }
    }
}

#[test]
fn native_workflow_installs_and_accepts_the_previous_release_update_on_every_os() {
    let workflow = native_workflow();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflows = root.join(".github/workflows");
    for name in ["ci.yml", "native-tests.yml", "release.yml"] {
        let text = fs::read_to_string(workflows.join(name)).unwrap();
        assert!(!text.contains("windows-2025"), "{name} pins windows-2025");
    }
    let ci = fs::read_to_string(workflows.join("ci.yml")).unwrap();
    let native_tests = ci
        .split("\n  native-tests:\n")
        .nth(1)
        .unwrap()
        .split("\n  native-build:\n")
        .next()
        .unwrap();
    assert!(native_tests.contains("install: true"));
    assert!(
        !native_tests.contains("\n    if:"),
        "native tests must run on every CI event"
    );
    for event in [
        "pull_request:",
        "push:\n    branches: [main]",
        "merge_group:",
    ] {
        assert!(ci.contains(event), "CI lost {event}");
    }
    // One install job serves every OS; no Windows-only install job remains.
    assert!(!workflow.contains("windows-install"));
    assert_eq!(workflow.matches("\n  install:\n").count(), 1);
    let install = workflow_job(&workflow, "install", "native-gate");
    let header = install.split("    steps:\n").next().unwrap();
    assert!(header.starts_with("    needs: dolt-windows-arm64\n    if: ${{ inputs.install && "));
    for required in ["runs-on: ${{ inputs.os }}", "timeout-minutes: 45"] {
        assert!(header.contains(required), "install job lost {required}");
    }
    assert!(install.contains("shared-key: native-install-${{ inputs.os }}"));
    // Step order is part of the contract: offline installation first, the
    // installed runtime next, and only then the online updater acceptance.
    let steps = workflow_steps(install);
    let step = |name: &str| {
        steps
            .iter()
            .position(|step| step.starts_with(&format!("name: {name}\n")))
            .unwrap_or_else(|| panic!("missing install step {name}"))
    };
    let order = [
        "Reserve space for release and test builds",
        "Download the Windows arm64 engine input",
        "Import the pin-verified Windows arm64 engine",
        "Fetch locked Cargo inputs before offline verification",
        "Prepare the package-owned bundle",
        "Prepare the package-owned bundle fixtures",
        "Install package-owned test tools",
        "Source install smoke",
        "Native Windows source install smoke",
        "Verify native offline Cargo input failures",
        "Verify installed offline runtime",
        "Inspect native shipping DLL imports",
        "Accept an update from the previous published release",
        "Require unchanged dependency locks",
    ];
    let positions: Vec<_> = order.iter().map(|name| step(name)).collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    for (name, condition) in [
        (
            "Reserve space for release and test builds",
            Some("if: runner.os == 'Linux'"),
        ),
        (
            "Download the Windows arm64 engine input",
            Some("if: runner.os == 'Windows' && runner.arch == 'ARM64'"),
        ),
        (
            "Import the pin-verified Windows arm64 engine",
            Some("if: runner.os == 'Windows' && runner.arch == 'ARM64'"),
        ),
        (
            "Prepare the package-owned bundle fixtures",
            Some("if: runner.os == 'Windows'"),
        ),
        ("Source install smoke", Some("if: runner.os != 'Windows'")),
        (
            "Native Windows source install smoke",
            Some("if: runner.os == 'Windows'"),
        ),
        (
            "Verify native offline Cargo input failures",
            Some("if: runner.os == 'Windows'"),
        ),
        ("Verify installed offline runtime", None),
        (
            "Inspect native shipping DLL imports",
            Some("if: runner.os == 'Windows'"),
        ),
        ("Accept an update from the previous published release", None),
    ] {
        let body = &steps[step(name)];
        match condition {
            Some(condition) => assert!(body.contains(condition), "{name} lost {condition}"),
            None => assert!(!body.contains("if:"), "{name} must run on every OS"),
        }
        assert!(
            !body.contains("inputs.os"),
            "{name} must select by runner.os"
        );
    }
    assert!(steps[step("Native Windows source install smoke")].contains("shell: pwsh"));
    for name in [
        "Source install smoke",
        "Native Windows source install smoke",
    ] {
        let body = &steps[step(name)];
        assert!(body.contains("KURU_INSTALL_DIR: ${{ runner.temp }}/kuru-bin"));
        assert!(body.contains("CARGO_NET_OFFLINE: \"true\""));
        assert!(body.contains("KURU_DOLT_BUNDLE_OFFLINE: \"true\""));
        assert!(body.contains("mise run install"));
    }
    assert!(steps[step("Verify installed offline runtime")].contains(
        "KURU_EMBEDDED_TEST_BINARY: ${{ runner.temp }}/kuru-bin/kuru${{ runner.os == 'Windows' && '.exe' || '' }}"
    ));
    // The updater step is online, receives the token only for release
    // listing, and names the installed shipping executable, never Cargo's
    // target directory, whose memory build-script fingerprint the Windows
    // build-input check between installation and this step leaves dirty.
    let updater = &steps[step("Accept an update from the previous published release")];
    for required in [
        "KURU_UPDATE_CANDIDATE_BINARY: ${{ runner.temp }}/kuru-bin/kuru${{ runner.os == 'Windows' && '.exe' || '' }}",
        "GITHUB_TOKEN: ${{ github.token }}",
        "run: mise run //packages/kuru-delivery:test:previous-release-update",
    ] {
        assert!(updater.contains(required), "updater step lost {required}");
    }
    assert!(!updater.contains("CARGO_NET_OFFLINE"));
    assert!(!updater.contains("KURU_DOLT_BUNDLE_OFFLINE"));
    assert!(workflow.contains("permissions:\n  contents: read\n"));
}

#[test]
fn native_build_input_check_reuses_the_installed_shipping_build() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |path: &str| fs::read_to_string(root.join(path)).unwrap();
    let script = read("packages/kuru-memory/support/verify-bundle-build.ps1");
    let task = |manifest: &str, header: &str| {
        read(manifest)
            .split_once(&format!("\n{header}\n"))
            .unwrap_or_else(|| panic!("missing {header} in {manifest}"))
            .1
            .split("\n[")
            .next()
            .unwrap()
            .to_owned()
    };
    let build = task("apps/kuru-tui/mise.toml", r#"[tasks."build:release"]"#);
    let verify = task(
        "packages/kuru-memory/mise.toml",
        r#"[tasks."bundle:verify-native-build"]"#,
    );
    // The source installation's release build is the verifier's positive
    // control, so both must run the same shipping command. Restoring
    // --all-features (or any other drift) would make the control recompile.
    let cargo: Vec<_> = script
        .lines()
        .filter(|line| line.contains("& cargo "))
        .collect();
    assert_eq!(
        cargo.len(),
        1,
        "the verifier must run exactly one Cargo command"
    );
    assert!(
        cargo[0]
            .contains("& cargo build -p kuru --release --locked --offline --target $target 2>&1")
    );
    // Both sides build the native host tuple: the shipping task names Cargo's
    // host-tuple and the verifier resolves the same tuple, restricted to the two
    // Windows catalog targets, so the control and the installed build share one
    // Cargo output directory on x64 and on Arm64.
    assert!(script.contains("$target = ([string](& rustc --print host-tuple)).Trim()"));
    assert!(script.contains(
        "if ($LASTEXITCODE -ne 0 -or @('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc') -cnotcontains $target) {"
    ));
    assert!(build.contains(
        "run_windows = 'cargo build -p kuru --release --locked --target {% if usage.target == \"host\" %}host-tuple"
    ));
    assert!(!script.contains("--all-features") && !build.contains("--all-features"));
    for text in [&build, &verify] {
        for required in [
            r#"env.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = "-C target-feature=+crt-static""#,
            r#"env.CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_RUSTFLAGS = "-C target-feature=+crt-static""#,
            "env.CARGO_BUILD_TARGET = false",
        ] {
            assert!(
                text.contains(required),
                "build or verifier task lost {required}"
            );
        }
    }
    for required in [
        r#"env.CARGO_NET_OFFLINE = "true""#,
        r#"env.KURU_DOLT_BUNDLE_OFFLINE = "true""#,
    ] {
        assert!(verify.contains(required), "verifier task lost {required}");
    }
    // The fresh control runs first with the caller's own mirror selection; both
    // offline negatives follow; no valid rebuild relinks target/ afterwards.
    let calls: Vec<_> = script
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("Invoke-Build '"))
        .collect();
    assert_eq!(
        calls,
        [
            "Invoke-Build 'installed build is the fresh offline control' $mirror '' -control",
            "Invoke-Build 'missing archive' $missing 'open checked build-input file'",
            "Invoke-Build 'same-size corrupt archive' $corrupt 'prepared Dolt archive checksum mismatch'",
        ]
    );
    for required in [
        "if ($control) { $env:KURU_DOLT_BUNDLE_DIR = $savedMirror }",
        "$message.Contains('Compiling ')",
        "if (-not $message.Contains('Finished '))",
        r#"$built = Join-Path $targetDirectory "$target/release/kuru.exe""#,
        "$builtHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $built).Hash",
        "if ($builtHash -cne $installedHash)",
        "failed to run custom build command for `kuru-memory",
        "-not $message.Contains((Join-Path $inputMirror $archiveName))",
        "changed the installed executable or original prepared archive",
    ] {
        assert!(script.contains(required), "verifier lost {required}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn native_workflow_gate_rejects_incomplete_results() {
    use kuru_delivery::command;
    use std::time::Duration;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = native_workflow();
    let gate = workflow
        .split("\n  native-gate:\n")
        .nth(1)
        .unwrap()
        .split("        run: |\n")
        .nth(1)
        .unwrap();
    // The same gate applies to every OS: it never branches on the caller's label.
    assert!(!gate.contains("inputs.os"));
    async fn run(
        root: &Path,
        gate: &str,
        (install, shards, report, install_result): (&str, &str, &str, &str),
    ) -> bool {
        let mut command = command::rooted(root, "bash");
        command
            .args(["-c", gate])
            .env("KURU_NATIVE_INSTALL", install)
            .env("KURU_NATIVE_SHARDS", shards)
            .env("KURU_NATIVE_REPORT", report)
            .env("KURU_NATIVE_INSTALL_RESULT", install_result);
        command::bounded_output(&mut command, Duration::from_secs(5), 4096)
            .await
            .unwrap()
            .status
            .success()
    }

    for accepted in [
        ("true", "success", "success", "success"),
        ("false", "success", "success", "skipped"),
    ] {
        assert!(
            run(&root, gate, accepted).await,
            "gate rejected {accepted:?}"
        );
    }
    for (label, shards, report, install) in [
        ("missing shard", "skipped", "success", "success"),
        ("failed shard", "failure", "success", "success"),
        ("cancelled shard", "cancelled", "success", "success"),
        ("missing report", "success", "skipped", "success"),
        ("failed report", "success", "failure", "success"),
        ("cancelled report", "success", "cancelled", "success"),
        ("missing install", "success", "success", "skipped"),
        ("failed install", "success", "success", "failure"),
        ("cancelled install", "success", "success", "cancelled"),
    ] {
        assert!(
            !run(&root, gate, ("true", shards, report, install)).await,
            "native gate accepted {label}"
        );
    }
    for (label, shards, report, install) in [
        ("failed shard", "failure", "success", "skipped"),
        ("missing report", "success", "skipped", "skipped"),
        ("unrequested install", "success", "success", "success"),
        (
            "failed unrequested install",
            "success",
            "success",
            "failure",
        ),
    ] {
        assert!(
            !run(&root, gate, ("false", shards, report, install)).await,
            "native gate accepted {label} without installation"
        );
    }
}
struct Repo {
    temp: TempDir,
}
impl Repo {
    async fn new() -> Self {
        let this = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        let root = this.root();
        release::git(root, &["init", "-b", "main"]).await.unwrap();
        for (key, value) in [
            ("user.name", "Release fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            release::git(root, &["config", key, value]).await.unwrap();
        }
        fs::create_dir(root.join("app")).unwrap();
        fs::write(
            root.join("app/Cargo.toml"),
            "[package]\nname = \"kuru\"\nversion.workspace = true\n",
        )
        .unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"app\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
        fs::write(root.join("Cargo.lock"), "version = 4\n\n[[package]]\nname = \"kuru\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.229\"\nsource = \"registry+https://example.invalid/index\"\nchecksum = \"unchanged\"\n").unwrap();
        fs::write(root.join("cog.toml"), include_str!("../../../cog.toml")).unwrap();
        fs::write(
            root.join("communique.toml"),
            include_str!("../../../communique.toml"),
        )
        .unwrap();
        this.commit("feat: initial peer harness").await;
        this
    }
    fn root(&self) -> &Path {
        self.temp.path()
    }
    async fn commit(&self, message: &str) -> String {
        release::git(self.root(), &["add", "."]).await.unwrap();
        release::git(self.root(), &["commit", "--allow-empty", "-m", message])
            .await
            .unwrap();
        self.head().await
    }
    async fn head(&self) -> String {
        release::git(self.root(), &["rev-parse", "HEAD"])
            .await
            .unwrap()
    }
    async fn tag(&self, tag: &str) {
        release::git(self.root(), &["tag", tag]).await.unwrap();
    }
}
#[tokio::test]
async fn hook_environment_cannot_redirect_rooted_commands() {
    if std::env::var_os(repository_environment::CHILD).is_some() {
        let repo = Repo::new().await;
        fs::write(
            repo.root().join("requested.txt"),
            "belongs to requested root",
        )
        .unwrap();
        repo.commit("fix: update the requested repository").await;
        assert_eq!(
            release::git(repo.root(), &["show", "HEAD:requested.txt"])
                .await
                .unwrap(),
            "belongs to requested root"
        );
        assert_eq!(
            release::git(
                repo.root(),
                &["config", "--default", "clean", "--get", "fixture.hook"]
            )
            .await
            .unwrap(),
            "clean"
        );
        assert_eq!(
            release::git(repo.root(), &["config", "--get", "user.signingkey"])
                .await
                .unwrap(),
            "fixture-signing-key"
        );
        release::run(
            repo.root(),
            env!("CARGO_BIN_EXE_kuru-delivery-fixture"),
            &["check-hook-env"],
        )
        .await
        .unwrap();
        return;
    }
    repository_environment::ForeignRepository::new()
        .assert_test_isolated("hook_environment_cannot_redirect_rooted_commands")
        .await;
}

#[tokio::test]
async fn hook_environment_preserves_versioning_and_private_recovery_index() {
    let foreign = repository_environment::ForeignRepository::new();
    for test in [
        "actual_cog_conventional_history_and_noop_are_read_only",
        "lost_bump_response_reuses_exact_tree_and_parent_even_after_main_advances",
    ] {
        foreign.assert_test_isolated(test).await;
    }
}

#[tokio::test]
async fn actual_cog_conventional_history_and_noop_are_read_only() {
    let repo = Repo::new().await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.1.0")
    );
    repo.tag("v0.1.0").await;
    assert!(
        release::compute_version(repo.root(), "auto")
            .await
            .unwrap_err()
            .to_string()
            .contains("no commits")
    );
    repo.commit("fix: preserve isolated memories").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.1.1")
    );
    repo.commit("feat: add peer relationships").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.2.0")
    );
    repo.commit("feat!: replace configuration format").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.2.0")
    );
    for (kind, expected) in [("major", "1.0.0"), ("minor", "0.2.0"), ("patch", "0.1.1")] {
        assert_eq!(
            release::compute_version(repo.root(), kind).await.unwrap(),
            v(expected)
        );
    }
    repo.tag("v1.0.0").await;
    repo.commit("fix!: remove legacy configuration").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("2.0.0")
    );
    assert_eq!(
        release::git(repo.root(), &["status", "--porcelain"])
            .await
            .unwrap(),
        ""
    );
    assert!(
        release::compute_version(repo.root(), "--help")
            .await
            .is_err()
    );
}
#[tokio::test]
async fn stamps_only_workspace_versions_and_rejects_inconsistent_locks() {
    let repo = Repo::new().await;
    let path = repo.root().join("Cargo.lock");
    let before: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(release::stamp(repo.root(), v("0.2.0")).unwrap().len(), 2);
    assert_eq!(
        release::workspace_version(repo.root(), None).await.unwrap(),
        v("0.2.0")
    );
    let after: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(before["package"][1], after["package"][1]);
    assert_eq!(after["package"][0]["version"].as_str(), Some("0.2.0"));
    assert!(release::stamp(repo.root(), v("0.2.0")).unwrap().is_empty());
    let manifest = fs::read(repo.root().join("Cargo.toml")).unwrap();
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("version = \"0.2.0\"", "version = \"0.0.1\""),
    )
    .unwrap();
    assert!(
        release::stamp(repo.root(), v("0.3.0"))
            .unwrap_err()
            .to_string()
            .contains("inconsistent")
    );
    assert_eq!(fs::read(repo.root().join("Cargo.toml")).unwrap(), manifest);
    fs::write(path, "version = 4\n").unwrap();
    assert!(
        release::stamp(repo.root(), v("0.3.0"))
            .unwrap_err()
            .to_string()
            .contains("every workspace")
    );
}
#[tokio::test]
async fn plans_initial_release_and_reuses_prepared_or_tagged_heads_without_inputs() {
    let repo = Repo::new().await;
    let head = repo.head().await;
    let initial = release::plan(repo.root(), "auto").await.unwrap();
    assert_eq!(initial.base_sha, head);
    assert_eq!(initial.version, "0.1.0");
    assert!(release::plan(repo.root(), "patch").await.is_err());
    repo.tag("v0.1.0").await;
    for strategy in ["auto", "major", "minor", "patch"] {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, head);
        assert_eq!(retry.version, "0.1.0");
    }
    repo.commit("feat: add relationships").await;
    let base = repo.head().await;
    let mut plans = Vec::new();
    for strategy in ["auto", "major", "minor", "patch"] {
        plans.push((
            strategy,
            release::plan(repo.root(), strategy).await.unwrap(),
        ));
    }
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let prepared = repo.commit("chore(release): v0.2.0").await;
    for strategy in ["auto", "major", "minor", "patch"] {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, prepared);
        assert_eq!(retry.version, "0.2.0");
    }
    repo.tag("v0.2.0").await;
    repo.commit("feat!: later work").await;
    repo.tag("v1.0.0").await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    for (strategy, original) in plans {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, original.base_sha);
        assert_eq!(retry.version, original.version);
    }
    assert!(release::plan(repo.root(), "invalid").await.is_err());
    fs::write(repo.root().join("untracked"), "dirty").unwrap();
    assert!(
        release::plan(repo.root(), "auto")
            .await
            .unwrap_err()
            .to_string()
            .contains("clean checkout")
    );
    for invalid in ["1.2", "01.2.3", "1.0.0\nx=y", "1.0.0;id", "1.0.0-rc.1", ""] {
        assert!(invalid.parse::<Version>().is_err());
    }
    for invalid in ["HEAD", "abcd", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"] {
        assert!(release::checked_sha(invalid).is_err());
    }
}

#[tokio::test]
async fn real_cli_calculates_plans_and_stamps_without_remote_credentials() {
    let repo = Repo::new().await;
    let binary = env!("CARGO_BIN_EXE_kuru-release");
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["version", "auto"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "0.1.0");
    let outputs = repo.root().join("outputs");
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["plan", "--bump", "auto"])
        .env("GITHUB_OUTPUT", &outputs)
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(
        fs::read_to_string(outputs)
            .unwrap()
            .contains("version=0.1.0\n")
    );
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["stamp", "0.2.0"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        release::workspace_version(repo.root(), None).await.unwrap(),
        v("0.2.0")
    );
    let output = kuru_delivery::command::Command::new(binary)
        .args(["commit", "--version", "0.2.0", "--expected-sha", A])
        .env_remove("GH_REPO")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GH_REPO is required"));
    let output = kuru_delivery::command::Command::new(binary)
        .args([
            "publish",
            "--version",
            "0.2.0",
            "--sha",
            A,
            "--notes",
            "missing",
        ])
        .env("GH_REPO", "fixture/kuru")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GH_TOKEN is required"));
    let output = kuru_delivery::command::Command::new(binary)
        .args(["stamp", "1.0.0;id"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
}

#[tokio::test]
async fn real_cli_assembles_a_candidate_without_github_credentials() {
    let archives = Archives::new();
    let output = kuru_delivery::command::Command::new(env!("CARGO_BIN_EXE_kuru-release"))
        .args([
            "assemble",
            "--version",
            "0.1.0",
            "--directory",
            archives.directory.to_str().unwrap(),
            "--notes",
            archives.notes.to_str().unwrap(),
        ])
        .env_remove("GH_REPO")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!(
            "Assembled {} release candidate checks",
            release::TARGETS.len() * 2 + 1
        )
    );
    assert_eq!(
        fs::read_to_string(archives.directory.join("SHA256SUMS"))
            .unwrap()
            .lines()
            .count(),
        release::TARGETS.len() * 2
    );
}

#[derive(Default)]
struct Remote {
    head: String,
    tag: Option<Value>,
    tag_object: Option<Value>,
    release: Option<Value>,
    calls: Vec<(Method, String, Value)>,
    uploads: Vec<String>,
    fail_upload: Option<usize>,
    corrupt_upload: bool,
    malformed: bool,
    prepared_commit: Option<Value>,
    comparison: Option<Value>,
    lose_commit_response: bool,
    manifest: Vec<u8>,
    manifest_redirect: Option<String>,
    lose_publish_response: bool,
}
struct Server {
    api: GitHub,
    state: Arc<Mutex<Remote>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(head: &str) -> Self {
        let state = Arc::new(Mutex::new(Remote {
            head: head.into(),
            ..Remote::default()
        }));
        let app = Router::new().fallback(handler).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            api: GitHub::with_endpoints("fixture/kuru", "fixture-token", &endpoint, &endpoint)
                .unwrap(),
            state,
            task,
        }
    }
}
fn response(status: StatusCode, value: Value) -> Response {
    (status, axum::Json(value)).into_response()
}
async fn handler(State(shared): State<Arc<Mutex<Remote>>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer fixture-token"
    );
    let bytes = to_bytes(request.into_body(), 300 * 1024 * 1024)
        .await
        .unwrap();
    let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut state = shared.lock().unwrap();
    state
        .calls
        .push((method.clone(), path.clone(), payload.clone()));
    if state.malformed {
        return (StatusCode::OK, "invalid-json").into_response();
    }
    let suffix = path.strip_prefix("/repos/fixture/kuru/").unwrap_or(&path);
    let value = match (method.clone(), suffix) {
        (Method::POST, "/graphql") => {
            if payload["variables"]["input"]["expectedHeadOid"].as_str() != Some(&state.head) {
                return response(
                    StatusCode::OK,
                    json!({"errors":[{"message":"expected head changed; private context"}]}),
                );
            }
            let sha = state
                .prepared_commit
                .as_ref()
                .map(|commit| commit["sha"].as_str().unwrap())
                .unwrap_or(B)
                .to_owned();
            state.head = sha.clone();
            if state.lose_commit_response {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"response lost after commit"}),
                );
            }
            json!({"data":{"createCommitOnBranch":{"commit":{"oid":sha}}}})
        }
        (Method::GET, "commits/main") => json!({"sha":state.head}),
        (Method::GET, value) if value.starts_with("compare/") => {
            assert_eq!(query, "per_page=1");
            state
                .comparison
                .clone()
                .unwrap_or(json!({"status":"diverged"}))
        }
        (Method::GET, value) if value.starts_with("git/ref/tags/") => {
            if let Some(tag) = &state.tag {
                json!({"object":tag})
            } else {
                return response(StatusCode::NOT_FOUND, json!({"message":"missing"}));
            }
        }
        (Method::POST, "git/tags") => {
            state.tag_object = Some(json!({"type":"commit","sha":payload["object"]}));
            json!({"sha":C})
        }
        (Method::POST, "git/refs") => {
            state.tag = Some(json!({"type":"tag","sha":payload["sha"]}));
            json!({"object":state.tag})
        }
        (Method::GET, value) if value.starts_with("git/tags/") => {
            json!({"object":state.tag_object})
        }
        (Method::GET, "releases") => state
            .release
            .as_ref()
            .map(|r| json!([r]))
            .unwrap_or(json!([])),
        (Method::POST, "releases") => {
            let mut value = payload;
            value["id"] = json!(17);
            value["assets"] = json!([]);
            value["html_url"] = json!("https://example.invalid/release");
            state.release = Some(value.clone());
            value
        }
        (Method::GET, "releases/17") => state.release.clone().unwrap(),
        (Method::POST, "releases/17/assets") => {
            if state.fail_upload == Some(state.uploads.len()) {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"fixture interrupted"}),
                );
            }
            let name = url::form_urlencoded::parse(query.as_bytes())
                .find(|(key, _)| key == "name")
                .unwrap()
                .1
                .into_owned();
            state.uploads.push(name.clone());
            let digest = if state.corrupt_upload {
                "0".repeat(64)
            } else {
                digest(&bytes)
            };
            let id = state.uploads.len() as u64;
            if name == "SHA256SUMS" {
                state.manifest = bytes.to_vec();
            }
            let asset = json!({"id":id,"name":name,"digest":format!("sha256:{digest}")});
            state.release.as_mut().unwrap()["assets"]
                .as_array_mut()
                .unwrap()
                .push(asset.clone());
            asset
        }
        (Method::GET, value) if value.starts_with("releases/assets/") => {
            if let Some(location) = &state.manifest_redirect {
                return (StatusCode::FOUND, [("location", location.clone())]).into_response();
            }
            return (StatusCode::OK, state.manifest.clone()).into_response();
        }
        (Method::PATCH, "releases/17") => {
            assert_eq!(
                payload["make_latest"], "legacy",
                "recovery must not force an older draft to become latest"
            );
            let release = state.release.as_mut().unwrap();
            release["draft"] = payload["draft"].clone();
            let result = release.clone();
            if state.lose_publish_response {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"response lost after publication"}),
                );
            }
            result
        }
        _ => {
            return response(
                StatusCode::BAD_REQUEST,
                json!({"message":"unexpected request"}),
            );
        }
    };
    response(StatusCode::OK, value)
}
#[tokio::test]
async fn signed_api_commit_uses_expected_head_and_only_scoped_payloads() {
    let repo = Repo::new().await;
    let head = repo.head().await;
    let server = Server::new(&head).await;
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.1.0"))
            .await
            .unwrap(),
        head
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
            .await
            .unwrap(),
        B
    );
    let payload = server
        .state
        .lock()
        .unwrap()
        .calls
        .iter()
        .find(|(_, path, _)| path == "/graphql")
        .unwrap()
        .2
        .clone();
    let input = &payload["variables"]["input"];
    assert_eq!(input["expectedHeadOid"], head);
    assert_eq!(input["branch"]["branchName"], "main");
    assert_eq!(input["message"]["headline"], "chore(release): v0.2.0");
    let files = input["fileChanges"]["additions"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let manifest = files.iter().find(|f| f["path"] == "Cargo.toml").unwrap();
    let data: toml::Value = toml::from_str(
        &String::from_utf8(
            STANDARD
                .decode(manifest["contents"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        data["workspace"]["package"]["version"].as_str(),
        Some("0.2.0")
    );
    server.state.lock().unwrap().head = C.into();
    let error = release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("diverged"));
    assert!(!error.contains("private context"));
    fs::write(repo.root().join("cog.toml"), "# unrelated change\n").unwrap();
    let requests_before = server.state.lock().unwrap().calls.len();
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
            .await
            .unwrap_err()
            .to_string(),
        "release stamp changed unrelated files: cog.toml",
    );
    assert_eq!(server.state.lock().unwrap().calls.len(), requests_before);
}
async fn comparison(repo: &Repo, base: &str, head: &str) -> Value {
    let commits = release::git(
        repo.root(),
        &["rev-list", "--reverse", &format!("{base}..{head}")],
    )
    .await
    .unwrap();
    let first = commits.lines().next().unwrap();
    let parents = release::git(repo.root(), &["show", "-s", "--format=%P", first])
        .await
        .unwrap();
    let tree = release::git(repo.root(), &["rev-parse", &format!("{first}^{{tree}}")])
        .await
        .unwrap();
    let message = release::git(repo.root(), &["show", "-s", "--format=%B", first])
        .await
        .unwrap();
    json!({"status":"ahead", "merge_base_commit":{"sha":base}, "commits":[{
        "sha":first, "parents":parents.split_whitespace().map(|sha| json!({"sha":sha})).collect::<Vec<_>>(),
        "commit":{"message":message, "tree":{"sha":tree}}
    }]})
}
#[tokio::test]
async fn lost_bump_response_reuses_exact_tree_and_parent_even_after_main_advances() {
    let repo = Repo::new().await;
    let base = repo.head().await;
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let prepared = repo.commit("chore(release): v0.2.0").await;
    let expected = comparison(&repo, &base, &prepared).await;
    fs::write(repo.root().join("later.txt"), "not part of this release").unwrap();
    let later = repo.commit("feat: later independent work").await;
    let later_comparison = comparison(&repo, &base, &later).await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let original_index = fs::read(repo.root().join(".git/index")).unwrap();
    let server = Server::new(&base).await;
    {
        let mut remote = server.state.lock().unwrap();
        remote.prepared_commit = Some(expected["commits"][0].clone());
        remote.comparison = Some(expected.clone());
        remote.lose_commit_response = true;
    }
    assert!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .is_err()
    );
    assert_eq!(server.state.lock().unwrap().head, prepared);
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .unwrap(),
        prepared
    );
    {
        let mut remote = server.state.lock().unwrap();
        remote.head = later;
        remote.comparison = Some(later_comparison);
    }
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .unwrap(),
        prepared
    );
    assert_eq!(
        fs::read(repo.root().join(".git/index")).unwrap(),
        original_index
    );
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, _, _)| method == Method::POST)
            .count(),
        1
    );

    // A title alone, or even identical contents with the wrong parent, is not
    // evidence that an intervening commit is the prepared release.
    for field in ["tree", "parent", "message", "merge", "diverged"] {
        let mut bad = expected.clone();
        match field {
            "tree" => bad["commits"][0]["commit"]["tree"]["sha"] = json!(C),
            "parent" => bad["commits"][0]["parents"][0]["sha"] = json!(C),
            "message" => bad["commits"][0]["commit"]["message"] = json!("feat: unrelated work"),
            "merge" => bad["commits"][0]["parents"]
                .as_array_mut()
                .unwrap()
                .push(json!({"sha":C})),
            _ => bad["status"] = json!("diverged"),
        }
        server.state.lock().unwrap().comparison = Some(bad);
        assert!(
            release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
                .await
                .is_err(),
            "accepted {field}"
        );
    }
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, _, _)| method == Method::POST)
            .count(),
        1
    );
}
#[tokio::test]
async fn unchanged_stamp_keeps_original_ancestor_without_including_new_main_work() {
    let repo = Repo::new().await;
    let base = repo.head().await;
    fs::write(repo.root().join("later.txt"), "later").unwrap();
    let later = repo.commit("fix: later work").await;
    let ahead = comparison(&repo, &base, &later).await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    let server = Server::new(&later).await;
    server.state.lock().unwrap().comparison = Some(ahead);
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.1.0"))
            .await
            .unwrap(),
        base
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    server.state.lock().unwrap().comparison.as_mut().unwrap()["merge_base_commit"]["sha"] =
        json!(C);
    assert!(
        release::commit_version(repo.root(), &server.api, &base, v("0.1.0"))
            .await
            .is_err()
    );
}
struct Archives {
    temp: TempDir,
    directory: PathBuf,
    notes: PathBuf,
}
impl Archives {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("dist");
        fs::create_dir(&directory).unwrap();
        // The candidate verifier decodes every exact core and support inventory.
        let binary = temp.path().join("fixture-kuru");
        fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        make_executable(&File::open(&binary).unwrap()).unwrap();
        let generated = temp.path().join("generated");
        fs::create_dir_all(generated.join("completions")).unwrap();
        fs::create_dir_all(generated.join("man")).unwrap();
        for name in shell_support::NAMES {
            fs::write(generated.join(name), format!("fixture {name}\n")).unwrap();
        }
        for target in release::TARGETS {
            kuru_delivery::archive::package(&binary, target, "0.1.0", &directory).unwrap();
            shell_support::package(&generated, target, "0.1.0", &directory).unwrap();
        }
        let notes = temp.path().join("notes.md");
        fs::write(&notes, "# Kuru 0.1.0\n\nPersistent peer conversations.\n").unwrap();
        release::assemble(&directory, v("0.1.0"), &notes).unwrap();
        Self {
            temp,
            directory,
            notes,
        }
    }
    async fn publish(&self, api: &GitHub) -> anyhow::Result<String> {
        release::publish(api, &self.directory, v("0.1.0"), A, &self.notes).await
    }
}
#[tokio::test]
async fn interrupted_draft_resumes_missing_assets_then_publishes_exact_commit() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().fail_upload = Some(2);
    assert!(archives.publish(&server.api).await.is_err());
    {
        let remote = server.state.lock().unwrap();
        assert_eq!(remote.release.as_ref().unwrap()["draft"], true);
        assert!(
            remote
                .calls
                .iter()
                .all(|(method, _, _)| method != Method::PATCH)
        );
    }
    server.state.lock().unwrap().fail_upload = None;
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert_eq!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap()
            .as_deref(),
        Some(A)
    );
    {
        let remote = server.state.lock().unwrap();
        assert_eq!(remote.uploads.len(), release::TARGETS.len() * 2 + 1);
        assert_eq!(remote.release.as_ref().unwrap()["draft"], false);
        assert!(
            remote.release.as_ref().unwrap()["body"]
                .as_str()
                .unwrap()
                .contains(A)
        );
    }
    let snapshot = server.state.lock().unwrap().release.clone();
    server.state.lock().unwrap().calls.clear();
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert_eq!(server.state.lock().unwrap().release, snapshot);
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    assert_eq!(
        fs::read_to_string(archives.directory.join("SHA256SUMS"))
            .unwrap()
            .lines()
            .count(),
        release::TARGETS.len() * 2
    );
}
#[tokio::test]
async fn lost_publication_response_recovers_from_remote_checksums_without_rebuilding() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().lose_publish_response = true;
    assert!(archives.publish(&server.api).await.is_err());
    assert_eq!(
        server.state.lock().unwrap().release.as_ref().unwrap()["draft"],
        false
    );
    fs::remove_dir_all(&archives.directory).unwrap();
    fs::remove_file(&archives.notes).unwrap();
    server.state.lock().unwrap().calls.clear();
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
}
#[tokio::test]
async fn published_recovery_rejects_incomplete_or_mismatched_remote_content() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    archives.publish(&server.api).await.unwrap();
    let original = server.state.lock().unwrap().release.clone().unwrap();
    let original_manifest = server.state.lock().unwrap().manifest.clone();
    for invalid in [
        "asset",
        "digest",
        "marker",
        "manifest",
        "oversize",
        "redirect",
        "missing-tag",
    ] {
        {
            let mut remote = server.state.lock().unwrap();
            remote.release = Some(original.clone());
            remote.manifest = original_manifest.clone();
            remote.calls.clear();
            remote.manifest_redirect = None;
            match invalid {
                "asset" => {
                    remote.release.as_mut().unwrap()["assets"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                "digest" => {
                    remote.release.as_mut().unwrap()["assets"][1]["digest"] =
                        json!(format!("sha256:{}", "0".repeat(64)))
                }
                "marker" => remote.release.as_mut().unwrap()["body"] = json!("unrelated release"),
                "manifest" => remote.manifest = b"inconsistent checksum data".to_vec(),
                "oversize" => remote.manifest = vec![b'x'; 4097],
                "redirect" => {
                    remote.manifest_redirect = Some("http://example.invalid/unsafe".into())
                }
                _ => remote.tag = None,
            }
        }
        assert!(
            archives.publish(&server.api).await.is_err(),
            "accepted {invalid}"
        );
        assert!(
            server
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .all(|(method, _, _)| method == Method::GET)
        );
    }
}
#[tokio::test]
async fn missing_corrupt_assets_and_empty_notes_prevent_all_remote_writes() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    let support_name = shell_support::archive_name("0.1.0", release::TARGETS[0]).unwrap();
    let support_path = archives.directory.join(&support_name);
    let original_support = fs::read(&support_path).unwrap();
    fs::remove_file(&support_path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    fs::write(&support_path, b"not a support envelope").unwrap();
    fs::write(
        archives.directory.join(format!("{support_name}.sha256")),
        format!("{}  {support_name}\n", digest(b"not a support envelope")),
    )
    .unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes).is_err(),
        "candidate accepted a checksummed but invalid support envelope"
    );
    fs::write(&support_path, &original_support).unwrap();
    fs::write(
        archives.directory.join(format!("{support_name}.sha256")),
        format!("{}  {support_name}\n", digest(&original_support)),
    )
    .unwrap();
    let name = kuru_delivery::archive::archive_name("0.1.0", release::TARGETS[0]).unwrap();
    let path = archives.directory.join(name);
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    fs::write(&path, b"corrupt").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    fs::write(&path, &original).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, &original).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(256 * 1024 * 1024 + 1)
        .unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("bounded regular")
    );
    fs::write(path, original).unwrap();
    fs::write(archives.directory.join("unexpected.sha256"), "unexpected\n").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    fs::remove_file(archives.directory.join("unexpected.sha256")).unwrap();
    fs::write(archives.directory.join("SHA256SUMS"), "stale\n").unwrap();
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("candidate checksum manifest")
    );
    release::assemble(&archives.directory, v("0.1.0"), &archives.notes).unwrap();
    fs::write(&archives.notes, "").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("nonempty")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("nonempty")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
}
#[tokio::test]
async fn immutable_tag_conflicts_and_unowned_or_corrupt_drafts_are_rejected() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().tag = Some(json!({"type":"commit","sha":B}));
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("different commit")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    server.state.lock().unwrap().tag = None;
    server.state.lock().unwrap().fail_upload = Some(1);
    assert!(archives.publish(&server.api).await.is_err());
    server.state.lock().unwrap().release.as_mut().unwrap()["assets"][0]["digest"] = json!("bad");
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    server.state.lock().unwrap().release.as_mut().unwrap()["body"] = json!("another draft");
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("does not belong")
    );
}
#[tokio::test]
async fn bad_api_shapes_and_final_uploaded_digest_never_publish() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().corrupt_upload = true;
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method != Method::PATCH)
    );
    server.state.lock().unwrap().malformed = true;
    assert!(release::tag_commit(&server.api, v("0.1.0")).await.is_err());
    server.state.lock().unwrap().malformed = false;
    server.state.lock().unwrap().tag = Some(json!({"type":"tree","sha":C}));
    assert!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("resolve")
    );
    server.state.lock().unwrap().tag = Some(json!({"type":"tag","sha":C}));
    server.state.lock().unwrap().tag_object = Some(json!({"type":"tag","sha":C}));
    assert!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("resolve")
    );
    assert!(GitHub::new("bad/repo/path", "fixture").is_err());
    assert!(
        GitHub::with_endpoints(
            "fixture/kuru",
            "fixture",
            "http://example.invalid/",
            "http://example.invalid/"
        )
        .is_err()
    );
    assert!(GitHub::new("fixture/kuru", "").is_err());
    assert!(archives.temp.path().exists());
}
