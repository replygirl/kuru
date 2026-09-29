#![cfg(feature = "tooling")]

use kuru_delivery::command::Command;
use std::{collections::BTreeSet, path::Path};

async fn run(configure: impl FnOnce(&mut Command)) -> (bool, String) {
    let root = tempfile::tempdir().unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("kuru-memory/support/dolt-assets.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    command
        .args(["bundle", "build", "--manifest"])
        .arg(manifest)
        .arg("--work-dir")
        .arg(root.path().join("work"))
        .arg("--output")
        .arg(root.path().join("out"))
        .env_remove("KURU_DOLT_BUNDLE_OFFLINE")
        .env_remove("KURU_BUNDLE_BUILD_HOST_OVERRIDE")
        .kill_on_drop(true);
    configure(&mut command);
    let output = kuru_delivery::command::output(&mut command, std::time::Duration::from_secs(15))
        .await
        .expect("bundle build CLI exceeded fixture deadline or failed to execute");
    assert!(!root.path().join("work").exists(), "refusal created work");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[tokio::test]
async fn cli_refuses_offline_builds_before_any_work() {
    let (success, stderr) = run(|command| {
        command
            .args(["--target", "aarch64-pc-windows-msvc", "--print-pins"])
            .env("KURU_DOLT_BUNDLE_OFFLINE", "true");
    })
    .await;
    assert!(!success);
    assert!(
        stderr.contains("unset KURU_DOLT_BUNDLE_OFFLINE"),
        "{stderr}"
    );
}

#[tokio::test]
async fn cli_requires_print_pins_on_a_non_authoritative_host() {
    // Elsewhere the override passes the host gate but cannot verify the
    // committed arm64 pins, so the build stops before any work. While those
    // pins await their first CI build, the unpinned refusal comes first
    // instead. linux-x64 is the authoritative build host, where a verifying
    // build would start the real recipe; the injected-host unit tests cover
    // this refusal there.
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../kuru-memory/support/dolt-assets.json")).unwrap();
    let pinned = manifest["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["target"] == "aarch64-pc-windows-msvc")
        .unwrap()["archive_sha256"]
        != "unpinned";
    let refusal = if pinned {
        "a non-authoritative host cannot verify pins; pass --print-pins"
    } else {
        "is built from source and not yet pinned"
    };
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) || !pinned {
        let (success, stderr) = run(|command| {
            command
                .args(["--target", "aarch64-pc-windows-msvc"])
                .env("KURU_BUNDLE_BUILD_HOST_OVERRIDE", "1");
        })
        .await;
        assert!(!success);
        assert!(stderr.contains(refusal), "{stderr}");
    }
    let (success, stderr) = run(|command| {
        command
            .args(["--target", "x86_64-pc-windows-msvc", "--print-pins"])
            .env("KURU_BUNDLE_BUILD_HOST_OVERRIDE", "1");
    })
    .await;
    assert!(!success);
    assert!(
        stderr.contains("uses the upstream Dolt archive"),
        "{stderr}"
    );
}

#[test]
fn manifest_toolchain_pins_equal_the_memory_package_lockfile() {
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(packages.join("kuru-memory/support/dolt-assets.json")).unwrap(),
    )
    .unwrap();
    let lock: toml::Value =
        toml::from_str(&std::fs::read_to_string(packages.join("kuru-memory/mise.lock")).unwrap())
            .unwrap();
    let tasks: toml::Value =
        toml::from_str(&std::fs::read_to_string(packages.join("kuru-memory/mise.toml")).unwrap())
            .unwrap();
    let built = manifest["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["provenance"] == "built")
        .unwrap();
    // The tools come from the bundle:build task, so a tool it adds needs a
    // pinned manifest toolchain; each pinned toolchain is used exactly once.
    let toolchains = built["build"]["toolchain"].as_object().unwrap();
    let tools = task_tools(&tasks);
    assert_eq!(tools.len(), toolchains.len(), "{tools:?}");
    let mut matched = BTreeSet::new();
    for (tool, task_version) in &tools {
        let entries = lock["tools"][tool.as_str()].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{tool}");
        let entry = &entries[0];
        assert_eq!(entry["version"].as_str().unwrap(), task_version, "{tool}");
        let platform = &entry["platforms.linux-x64"];
        let (name, pinned) = toolchains
            .iter()
            .find(|(_, pinned)| platform["url"].as_str() == pinned["url"].as_str())
            .unwrap_or_else(|| panic!("{tool} has no pinned manifest toolchain"));
        assert!(matched.insert(name.clone()), "{name}");
        let version = pinned["version"].as_str().unwrap();
        assert_eq!(
            entry["version"].as_str().unwrap(),
            version.trim_start_matches("go"),
            "{tool}"
        );
        assert_eq!(
            platform["url"].as_str().unwrap(),
            pinned["url"].as_str().unwrap(),
            "{tool}"
        );
        assert_eq!(
            platform["checksum"].as_str().unwrap(),
            format!("sha256:{}", pinned["sha256"].as_str().unwrap()),
            "{tool}"
        );
    }
}

/// `value` with the memory package's `{{vars.*}}` resolved.
fn resolve_vars(tasks: &toml::Value, value: &str) -> String {
    let vars = tasks.get("vars").and_then(toml::Value::as_table);
    let mut resolved = value.to_owned();
    for (name, var) in vars.into_iter().flatten() {
        if let Some(var) = var.as_str() {
            resolved = resolved.replace(&format!("{{{{vars.{name}}}}}"), var);
        }
    }
    resolved
}

/// The `bundle:build` task's tools with their versions, `{{vars.*}}` resolved.
fn task_tools(tasks: &toml::Value) -> Vec<(String, String)> {
    tasks["tasks"]["bundle:build"]
        .get("tools")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .map(|(tool, spec)| {
            let version = spec
                .as_str()
                .or_else(|| spec.get("version").and_then(toml::Value::as_str))
                .unwrap_or_default();
            (tool.clone(), resolve_vars(tasks, version))
        })
        .collect()
}

/// The `bundle:build` tools as `mise install` arguments, with the linux-x64
/// asset pattern a tool pins. `setup:build-tools` must install exactly these,
/// or the build could find a different installed asset under the same version.
fn tool_specs(tasks: &toml::Value) -> BTreeSet<String> {
    tasks["tasks"]["bundle:build"]
        .get("tools")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .map(|(tool, spec)| {
            let version = spec
                .as_str()
                .or_else(|| spec.get("version").and_then(toml::Value::as_str))
                .unwrap_or_default();
            let version = resolve_vars(tasks, version);
            let pattern = spec
                .get("platforms")
                .and_then(|platforms| platforms.get("linux-x64"))
                .and_then(|platform| platform.get("asset_pattern"))
                .and_then(toml::Value::as_str);
            match pattern {
                Some(pattern) => format!("{tool}[asset_pattern={pattern}]@{version}"),
                None => format!("{tool}@{version}"),
            }
        })
        .collect()
}

const INPUT_JOB: &str = "windows-arm64-input";
const CRATES_ENV: &str = "ENGINE_ARCHIVE_CRATES";
const WORKFLOW: &str = ".github/workflows/bundle-build.yml";
const TASKS: &str = "packages/kuru-memory/mise.toml";
const ROOT_TASKS: &str = "mise.toml";
/// In-repository crates whose modules encode the archive bytes: Rust path,
/// Cargo.lock name and directory. Every module of theirs a keyed file uses is
/// keyed and walked like the helper's own modules.
const ENCODERS: [(&str, &str, &str); 1] =
    [("kuru_archive", "kuru-archive", "packages/kuru-archive")];
/// The helper binary's path to its own library, and that library's directory.
const HELPER_LIBRARY: (&str, &str) = ("kuru_delivery", "packages/kuru-delivery");
/// Helper library modules a keyed file may use although the key does not hash
/// them, as on main before the key was narrowed. None encodes archive bytes,
/// but a change to one could still change them. The pin check, not the key or
/// this test, catches that. A keyed file's use of any other unkeyed module is
/// a finding.
const UNKEYED_MODULES: &[&str] = &[
    // SHA-256 digests, host target and version parsing.
    "archive",
    // Process launch. Command::new copies the parent environment, so this
    // module decides a child's environment; the recipe's runner clears it to
    // PATH and HOME (bundle/build.rs, `INHERITED`).
    "command",
    // The bundle directory lock and `bundle prepare` staging.
    "lease",
    "staging",
    // Modules main.rs dispatches only for their own subcommands.
    "advisory",
    "coverage",
    "docs",
    "published_windows",
    "repo",
    "shell_support",
];
/// In-repository crates a keyed file may use unkeyed, backed by the pin check
/// like `UNKEYED_MODULES`: checked directory and file handles.
const UNKEYED_CRATES: &[&str] = &["kuru_platform"];

/// The `bundle:build` task's exact command. A prefix command or another helper
/// argument would be an input the key does not cover.
const BUNDLE_BUILD_RUN: &str =
    "cargo run -p kuru-delivery --features tooling --locked --bin kuru-delivery -- bundle build";
/// Keys `bundle:build` and `setup:build-tools` may set. `depends`, `shell`,
/// `run_windows`, `usage` and the rest could run or select other inputs.
const BUNDLE_BUILD_KEYS: &[&str] = &["description", "dir", "env", "tools", "run"];
const SETUP_TOOLS_KEYS: &[&str] = &["description", "run"];
/// Top-level tables and `[env]` names of the memory package's mise.toml. Its
/// `[env]` and any `[tools]`, `[hooks]` or `[settings]` apply to the task.
const PACKAGE_MISE_KEYS: &[&str] = &["vars", "tasks", "env"];
const PACKAGE_MISE_ENV: &[&str] = &["RUST_TEST_THREADS"];
/// The root mise.toml configures every package task. Its `[env]` and `[tools]`
/// reach the build's PATH, so their names are fixed here; root tool versions
/// (including the Rust toolchain that compiles the helper) stay unkeyed and
/// are backed by the pin check.
const ROOT_MISE_KEYS: &[&str] = &[
    "min_version",
    "monorepo_root",
    "settings",
    "env",
    "tool_alias",
    "tools",
    "monorepo",
    "hooks",
    "tasks",
];
const ROOT_MISE_ENV: &[&str] = &["HK_MISE", "MBX_TARGET_VIEWS"];
const ROOT_MISE_TOOLS: &[&str] = &[
    "rust",
    "mr-boxington",
    "aqua:jdx/hk",
    "aqua:tamasfe/taplo",
    "aqua:koalaman/shellcheck",
    "aqua:rhysd/actionlint",
    "aqua:taiki-e/cargo-llvm-cov",
    "github:aligned-team/cospec",
];
/// Root mise tables whose exact values decide how the task's tools resolve.
const ROOT_MISE_EXACT: &[(&str, &str)] = &[
    ("settings", "experimental = true\nlockfile = true"),
    ("tool_alias", "npm = \"npm:npm\""),
    ("monorepo", "config_roots = [\"apps/*\", \"packages/*\"]"),
];
const ROOT_MISE_HOOKS: &[&str] = &["postinstall"];
/// Directories whose mise configuration the task loads.
const MISE_CONFIG_DIRECTORIES: &[&str] = &["", "packages/", "packages/kuru-memory/"];

/// The input job's exact toolchain installation and build steps.
const SETUP_STEP_RUN: &str = "mise run //packages/kuru-memory:setup:build-tools";
const BUILD_STEP_RUN: &str = concat!(
    "set -eu\n",
    "mise run //packages/kuru-memory:bundle:build -- \\\n",
    "  --target \"$BUILT_TARGET\" \\\n",
    "  --work-dir \"$RUNNER_TEMP/bundle-build/work\" \\\n",
    "  --output \"$RUNNER_TEMP/bundle-build/out\"\n",
    "mkdir \"$RUNNER_TEMP/bundle-input\"\n",
    "cp \"$RUNNER_TEMP/bundle-build/out/$BUILT_ARCHIVE\" \"$RUNNER_TEMP/bundle-input/\"\n",
);
/// The workflow environment every input job step inherits. It selects mise's
/// behavior (`MISE_ENV` would load other config files) and the helper's build.
const WORKFLOW_ENV: &[(&str, &str)] = &[
    ("MISE_LOCKED", "1"),
    ("KURU_MBX", "0"),
    ("MISE_EXEC_AUTO_INSTALL", "false"),
    ("MISE_NO_HOOKS", "1"),
    ("MISE_TASK_RUN_AUTO_INSTALL", "false"),
    ("CARGO_INCREMENTAL", "0"),
    ("BUILT_TARGET", "aarch64-pc-windows-msvc"),
    ("BUILT_ARCHIVE", "dolt-windows-arm64.zip"),
];
const INPUT_JOB_KEYS: &[&str] = &[
    "name",
    "if",
    "runs-on",
    "timeout-minutes",
    "env",
    "outputs",
    "steps",
];
/// Actions the input job may use; another action could change the build's PATH.
const INPUT_JOB_ACTIONS: &[&str] = &[
    "actions/checkout",
    "actions/cache/restore",
    "jdx/mise-action",
    "Swatinem/rust-cache",
    "actions/cache/save",
    "actions/upload-artifact",
];
const INPUT_STEP_ENV: &[&str] = &["GITHUB_TOKEN", "PINNED_SHA256", "PINNED_BYTES"];
const MISE_ACTION_INPUTS: &[&str] = &["experimental", "version", "install_args"];

/// Crates whose code or features decide the deflate and ZIP bytes.
const COMPRESSION_CRATES: &[&str] = &[
    "adler2",
    "crc32fast",
    "flate2",
    "miniz_oxide",
    "simd-adler32",
    "zip",
    "zlib-rs",
];
/// The workspace's compression dependencies: default features and features.
/// A Cargo.toml feature can switch the deflate backend without changing
/// Cargo.lock, so the key's lock digest would not rotate.
const WORKSPACE_COMPRESSION: &[(&str, bool, &[&str])] = &[
    ("crc32fast", true, &[]),
    ("flate2", true, &[]),
    ("zip", false, &["deflate-flate2"]),
];
/// Registry packages in Cargo.lock that depend on a compression crate, and so
/// can enable its features. A lockfile-only bump that adds one must be reviewed.
const COMPRESSION_DEPENDENTS: &[&str] = &["flate2", "miniz_oxide", "zip"];

/// Keys of `table` outside `allowed`.
fn extra_keys(table: Option<&toml::Table>, allowed: &[&str]) -> Vec<String> {
    table
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .filter(|key| !allowed.contains(&key.as_str()))
        .collect()
}

/// Every input the engine build reads, derived from the `bundle:build` task,
/// the helper's module declarations, its CLI default manifest and Cargo.lock,
/// compared with what the input job's cache key hashes. The task, its mise
/// configuration, the job's environment and steps, and the compression crates'
/// Cargo features must equal what this test reviewed, so a change to any of
/// them fails here instead of reusing a stale key. `tracked` is the
/// repository's file list. Returns one finding per difference.
fn key_findings(root: &Path, help: &str, tracked: &BTreeSet<String>) -> Vec<String> {
    let mut findings = Vec::new();
    let read = |relative: &str| {
        std::fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"))
    };
    let workflow: serde_yaml_ng::Value = serde_yaml_ng::from_str(&read(WORKFLOW)).unwrap();
    findings.extend(workflow_findings(&workflow));
    let job = &workflow["jobs"][INPUT_JOB];
    let steps = job["steps"].as_sequence().unwrap();
    let restore = steps
        .iter()
        .position(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("actions/cache/restore@"))
        })
        .expect("the input job restores a cached archive");
    let key = steps[restore]["with"]["key"].as_str().unwrap();
    let files_argument = key
        .split_once("hashFiles(")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map_or("", |(arguments, _)| arguments);
    let keyed_files: BTreeSet<String> = files_argument
        .split(',')
        .map(|argument| argument.trim().trim_matches('\'').to_owned())
        .filter(|argument| !argument.is_empty())
        .collect();
    let keyed_crates: BTreeSet<String> = job["env"][CRATES_ENV]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let digest = steps.iter().position(|step| {
        step["run"].as_str().is_some_and(|run| {
            run.contains(&format!("${CRATES_ENV}")) && run.contains("Cargo.lock")
        })
    });
    let digest_id = match digest {
        Some(index) if index < restore => steps[index]["id"].as_str().unwrap_or_default(),
        _ => {
            findings.push(format!(
                "no step before the restore digests the {CRATES_ENV} Cargo.lock records"
            ));
            ""
        }
    };
    // The key is exactly the target, the pin, one file hash and the crate
    // digest: a second hashFiles or another expression goes unchecked.
    let shape = format!(
        "bundle-input-${{{{ env.BUILT_TARGET }}}}-${{{{ steps.pin.outputs.sha256 }}}}-${{{{ hashFiles({files_argument}) }}}}-${{{{ steps.{digest_id}.outputs.sha256 }}}}"
    );
    if key != shape {
        findings.push(format!(
            "the key is not the target, the pin, one hashFiles and the {CRATES_ENV} digest: {key}"
        ));
    }

    // The task and the mise configuration it loads.
    let tasks: toml::Value = toml::from_str(&read(TASKS)).unwrap();
    let root_tasks: toml::Value = toml::from_str(&read(ROOT_TASKS)).unwrap();
    findings.extend(mise_findings(&tasks, &root_tasks, tracked));
    let words: Vec<_> = BUNDLE_BUILD_RUN.split_whitespace().collect();
    let value = |flag: &str| {
        words
            .iter()
            .position(|word| *word == flag)
            .and_then(|index| words.get(index + 1))
            .copied()
            .unwrap()
    };
    let (package, binary) = (value("-p"), value("--bin"));
    let arguments = words
        .iter()
        .position(|word| *word == "--")
        .map(|index| &words[index + 1..])
        .unwrap();

    let mut expected = BTreeSet::new();
    let package_dir = format!("packages/{package}");
    let cargo: toml::Value = toml::from_str(&read(&format!("{package_dir}/Cargo.toml"))).unwrap();
    let bin_path = cargo["bin"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|bin| bin["name"].as_str() == Some(binary))
        .and_then(|bin| bin["path"].as_str());
    // Each pending file: its path, whether `#[path]` loaded it, and its
    // crate's directory.
    let mut pending: Vec<(String, bool, String)> = Vec::new();
    match bin_path {
        Some(path) => pending.push((format!("{package_dir}/{path}"), false, package_dir.clone())),
        None => findings.push(format!("{package} has no binary {binary}")),
    }

    // The helper's subcommand module and every module it declares outside
    // tests. Test-only modules never reach the binary.
    let entry = arguments.first().copied().unwrap_or_default();
    let lib = format!("{package_dir}/src/lib.rs");
    let subcommand: Vec<_> = modules(&read(&lib), &lib, false)
        .into_iter()
        .filter(|module| module.name == entry && !module.test)
        .map(|module| (module.path, module.by_path, package_dir.clone()))
        .collect();
    if subcommand.is_empty() {
        findings.push(format!("{package} declares no {entry} module"));
    }
    pending.extend(subcommand);
    let mut test_only = BTreeSet::new();
    // Module paths each keyed file uses: (file, crate directory, module).
    let mut uses = Vec::new();
    while let Some((file, by_path, crate_dir)) = pending.pop() {
        if !expected.insert(file.clone()) {
            continue;
        }
        let text = read(&file);
        for module in modules(&text, &file, by_path) {
            if module.test {
                test_only.insert(module.path);
            } else {
                pending.push((module.path, module.by_path, crate_dir.clone()));
            }
        }
        let code = production_code(&text);
        for forbidden in ["include_str!", "include_bytes!", "include!", "super::super"] {
            if code.contains(forbidden) {
                findings.push(format!(
                    "keyed {file} uses {forbidden}, whose input the key does not cover"
                ));
            }
        }
        // A top-level module's `super` is its crate root.
        let top_level = Path::new(&file).parent() == Some(Path::new(&format!("{crate_dir}/src")));
        let mut roots = vec!["crate"];
        if top_level {
            roots.push("super");
        }
        if crate_dir == HELPER_LIBRARY.1 {
            roots.push(HELPER_LIBRARY.0);
        }
        for path_root in roots {
            for module in path_modules(&code, path_root) {
                uses.push((file.clone(), crate_dir.clone(), module));
            }
        }
        for name in crate_roots(&code) {
            if name == HELPER_LIBRARY.0 {
                continue;
            }
            match ENCODERS.iter().find(|(rust_name, _, _)| *rust_name == name) {
                Some((_, _, directory)) => {
                    for module in path_modules(&code, &name) {
                        pending.push((
                            format!("{directory}/src/{module}.rs"),
                            false,
                            (*directory).to_owned(),
                        ));
                    }
                }
                None if UNKEYED_CRATES.contains(&name.as_str()) => {}
                None => findings.push(format!(
                    "keyed {file} uses in-repository crate {name}, which the key does not cover"
                )),
            }
        }
    }
    for (file, crate_dir, module) in uses {
        let keyed = expected.contains(&format!("{crate_dir}/src/{module}.rs"))
            || expected.contains(&format!("{crate_dir}/src/{module}/mod.rs"));
        let exempt = crate_dir == HELPER_LIBRARY.1 && UNKEYED_MODULES.contains(&module.as_str());
        if !keyed && !exempt {
            findings.push(format!(
                "keyed {file} uses module {module} of {crate_dir}, which is neither keyed nor exempt"
            ));
        }
    }
    let module_dir = format!("{package_dir}/src/{entry}");
    for dir_entry in std::fs::read_dir(root.join(&module_dir))
        .into_iter()
        .flatten()
    {
        let name = dir_entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .into_owned();
        let path = format!("{module_dir}/{name}");
        if name.ends_with(".rs") && !expected.contains(&path) && !test_only.contains(&path) {
            findings.push(format!("{path} is neither keyed nor a test module"));
        }
    }

    // The manifest the helper reads by default, relative to the task's root.
    let manifest_path = help
        .split_once("--manifest")
        .and_then(|(_, rest)| rest.split_once("[default: "))
        .and_then(|(_, rest)| rest.split_once(']'));
    match manifest_path {
        Some((manifest, _)) => {
            expected.insert(manifest.to_owned());
        }
        None => findings.push("bundle build --help shows no default manifest".into()),
    }

    // Each task tool must be a pinned manifest toolchain: the lock entry it
    // installs carries the manifest's URL and digest, so the keyed manifest
    // covers it and neither mise.toml nor mise.lock needs its own hash.
    let lock: toml::Value = toml::from_str(&read("packages/kuru-memory/mise.lock")).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&read("packages/kuru-memory/support/dolt-assets.json")).unwrap();
    let toolchains: Vec<_> = manifest["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|asset| asset["provenance"] == "built")
        .filter_map(|asset| asset["build"]["toolchain"].as_object())
        .flat_map(serde_json::Map::values)
        .collect();
    for (tool, version) in task_tools(&tasks) {
        let pinned = lock["tools"]
            .get(&tool)
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|entry| entry["version"].as_str() == Some(version.as_str()))
            .filter_map(|entry| entry.get("platforms.linux-x64"))
            .any(|platform| {
                toolchains.iter().any(|toolchain| {
                    platform["url"].as_str() == toolchain["url"].as_str()
                        && platform["checksum"].as_str().map(str::to_owned)
                            == toolchain["sha256"]
                                .as_str()
                                .map(|sha| format!("sha256:{sha}"))
                })
            });
        if !pinned {
            findings.push(format!(
                "bundle:build tool {tool}@{version} is not a pinned manifest toolchain, so the key does not cover it"
            ));
        }
    }

    // The encoders' locked crate closure, as Cargo.lock lists it. A
    // dependency names its version only when the lock holds several.
    let cargo_lock: toml::Value = toml::from_str(&read("Cargo.lock")).unwrap();
    let packages = cargo_lock["package"].as_array().unwrap();
    findings.extend(cargo_feature_findings(
        &toml::from_str(&read("Cargo.toml")).unwrap(),
        &read,
        packages,
    ));
    let mut queue: Vec<(String, Option<String>)> = ENCODERS
        .iter()
        .filter(|(_, _, directory)| {
            expected
                .iter()
                .any(|file| file.starts_with(&format!("{directory}/")))
        })
        .map(|(_, name, _)| ((*name).to_owned(), None))
        .collect();
    let mut visited = BTreeSet::new();
    let mut expected_crates = BTreeSet::new();
    while let Some((name, version)) = queue.pop() {
        let matching: Vec<_> = packages
            .iter()
            .filter(|package| {
                package["name"].as_str() == Some(name.as_str())
                    && version
                        .as_deref()
                        .is_none_or(|version| package["version"].as_str() == Some(version))
            })
            .collect();
        assert_eq!(matching.len(), 1, "Cargo.lock resolves {name} {version:?}");
        let package = matching[0];
        if !visited.insert((
            name.clone(),
            package["version"].as_str().unwrap().to_owned(),
        )) {
            continue;
        }
        if package.get("source").is_some() {
            expected_crates.insert(name.clone());
        } else if !ENCODERS.iter().any(|(_, encoder, _)| *encoder == name) {
            findings.push(format!(
                "the archive writer depends on in-repository crate {name}, whose sources the key does not cover"
            ));
        }
        let dependencies = package.get("dependencies").and_then(toml::Value::as_array);
        for dependency in dependencies
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
        {
            let mut words = dependency.split(' ');
            let name = words.next().unwrap_or_default().to_owned();
            queue.push((name, words.next().map(str::to_owned)));
        }
    }

    for file in expected.difference(&keyed_files) {
        findings.push(format!(
            "the build reads {file}, which the key does not cover"
        ));
    }
    for file in keyed_files.difference(&expected) {
        findings.push(format!(
            "the key hashes {file}, which is not a derived engine build input"
        ));
    }
    for name in expected_crates.difference(&keyed_crates) {
        findings.push(format!(
            "the archive writer uses locked crate {name}, which {CRATES_ENV} omits"
        ));
    }
    for name in keyed_crates.difference(&expected_crates) {
        findings.push(format!(
            "{CRATES_ENV} lists {name}, which the archive writer does not use"
        ));
    }
    // hashFiles hashes nothing for a missing path, and a keyed input must
    // still start the determinism job on a pull request.
    let paths: Vec<&str> = workflow["on"]["pull_request"]["paths"]
        .as_sequence()
        .into_iter()
        .flatten()
        .filter_map(serde_yaml_ng::Value::as_str)
        .collect();
    let triggers = |file: &str| {
        paths
            .iter()
            .any(|pattern| match pattern.strip_suffix("/**") {
                Some(directory) => file.starts_with(&format!("{directory}/")),
                None => *pattern == file,
            })
    };
    for file in keyed_files.iter().map(String::as_str).chain(["Cargo.lock"]) {
        if file.contains('*') {
            findings.push(format!(
                "the key hashes pattern {file}; name each input file"
            ));
        } else if !root.join(file).is_file() {
            findings.push(format!("the key hashes missing file {file}"));
        }
        if !triggers(file) {
            findings.push(format!(
                "a change to keyed {file} does not start the determinism job"
            ));
        }
    }
    findings
}

/// The workflow environment, the input job's settings, actions and step
/// environments, and its exact toolchain installation and build steps.
fn workflow_findings(workflow: &serde_yaml_ng::Value) -> Vec<String> {
    let mut findings = Vec::new();
    let text = |value: &serde_yaml_ng::Value| match value {
        serde_yaml_ng::Value::String(text) => Some(text.clone()),
        serde_yaml_ng::Value::Number(number) => Some(number.to_string()),
        serde_yaml_ng::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    };
    let names = |value: &serde_yaml_ng::Value| -> BTreeSet<String> {
        value
            .as_mapping()
            .into_iter()
            .flatten()
            .filter_map(|(name, _)| name.as_str().map(str::to_owned))
            .collect()
    };
    let environment: BTreeSet<(String, Option<String>)> = workflow["env"]
        .as_mapping()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| Some((name.as_str()?.to_owned(), text(value))))
        .collect();
    let reviewed: BTreeSet<(String, Option<String>)> = WORKFLOW_ENV
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Some((*value).to_owned())))
        .collect();
    if environment != reviewed {
        findings.push(format!(
            "the workflow env {environment:?} differs from the reviewed build environment"
        ));
    }
    if !workflow["defaults"].is_null() {
        findings.push("the workflow sets defaults for the build's steps".into());
    }
    let job = &workflow["jobs"][INPUT_JOB];
    for key in names(job) {
        if !INPUT_JOB_KEYS.contains(&key.as_str()) {
            findings.push(format!("{INPUT_JOB} sets {key}"));
        }
    }
    if job["runs-on"].as_str() != Some("ubuntu-latest") {
        findings.push(format!("{INPUT_JOB} no longer runs on ubuntu-latest"));
    }
    if names(&job["env"]) != BTreeSet::from([CRATES_ENV.to_owned()]) {
        findings.push(format!("{INPUT_JOB} sets env other than {CRATES_ENV}"));
    }
    let (mut setup, mut build) = (0, 0);
    for step in job["steps"].as_sequence().into_iter().flatten() {
        let name = step["name"].as_str().unwrap_or_default();
        if let Some(uses) = step["uses"].as_str() {
            let action = uses.split_once('@').map_or(uses, |(action, _)| action);
            if !INPUT_JOB_ACTIONS.contains(&action) {
                findings.push(format!("{INPUT_JOB} uses {action}"));
            }
            if action == "jdx/mise-action" {
                for input in names(&step["with"]) {
                    if !MISE_ACTION_INPUTS.contains(&input.as_str()) {
                        findings.push(format!("{INPUT_JOB}'s mise-action sets {input}"));
                    }
                }
            }
        }
        for variable in names(&step["env"]) {
            if !INPUT_STEP_ENV.contains(&variable.as_str()) {
                findings.push(format!("{INPUT_JOB} step {name:?} sets env {variable}"));
            }
        }
        let run = step["run"].as_str().unwrap_or_default();
        for file in ["GITHUB_ENV", "GITHUB_PATH"] {
            if run.contains(file) {
                findings.push(format!("{INPUT_JOB} step {name:?} writes {file}"));
            }
        }
        if run.contains("setup:build-tools") {
            setup += 1;
            if run != SETUP_STEP_RUN {
                findings.push(format!(
                    "the input job's toolchain step is not exactly {SETUP_STEP_RUN:?}: {run:?}"
                ));
            }
        }
        if run.contains("bundle:build") {
            build += 1;
            if run != BUILD_STEP_RUN {
                findings.push(format!(
                    "the input job's build step is not the reviewed invocation: {run:?}"
                ));
            }
        }
    }
    if (setup, build) != (1, 1) {
        findings.push(format!(
            "the input job has {setup} toolchain and {build} build steps, not one of each"
        ));
    }
    findings
}

/// The `bundle:build` and `setup:build-tools` tasks, the memory package's and
/// the root's mise configuration, and other mise config files the task loads.
fn mise_findings(
    tasks: &toml::Value,
    root_tasks: &toml::Value,
    tracked: &BTreeSet<String>,
) -> Vec<String> {
    let mut findings = Vec::new();
    let task = &tasks["tasks"]["bundle:build"];
    for key in extra_keys(task.as_table(), BUNDLE_BUILD_KEYS) {
        findings.push(format!("bundle:build sets {key}"));
    }
    if task.get("dir").and_then(toml::Value::as_str) != Some("{{config_root}}/../..") {
        findings.push("bundle:build no longer runs from the repository root".into());
    }
    let run = task.get("run").and_then(toml::Value::as_str);
    if run != Some(BUNDLE_BUILD_RUN) {
        findings.push(format!(
            "bundle:build runs {run:?}, not exactly {BUNDLE_BUILD_RUN:?}"
        ));
    }
    let env = task.get("env").and_then(toml::Value::as_table);
    for (name, value) in env.into_iter().flatten() {
        if value.as_bool() != Some(false) {
            findings.push(format!(
                "bundle:build sets env {name}, which the key does not cover"
            ));
        }
    }
    let tools = task.get("tools").and_then(toml::Value::as_table);
    for (tool, spec) in tools.into_iter().flatten() {
        let Some(spec) = spec.as_table() else {
            continue;
        };
        let platforms = spec.get("platforms").and_then(toml::Value::as_table);
        let linux = platforms
            .and_then(|platforms| platforms.get("linux-x64"))
            .and_then(toml::Value::as_table);
        let extra: Vec<_> = extra_keys(Some(spec), &["version", "platforms"])
            .into_iter()
            .chain(extra_keys(platforms, &["linux-x64"]))
            .chain(extra_keys(linux, &["asset_pattern"]))
            .collect();
        if !extra.is_empty() {
            findings.push(format!(
                "bundle:build tool {tool} sets options {extra:?}, which the lock check does not cover"
            ));
        }
    }
    let setup = &tasks["tasks"]["setup:build-tools"];
    for key in extra_keys(setup.as_table(), SETUP_TOOLS_KEYS) {
        findings.push(format!("setup:build-tools sets {key}"));
    }
    let setup_run = setup
        .get("run")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    let installed: Option<BTreeSet<String>> = resolve_vars(tasks, setup_run)
        .strip_prefix("mise install ")
        .map(|specs| {
            specs
                .split_whitespace()
                .map(|spec| spec.trim_matches('\'').to_owned())
                .collect()
        });
    if installed.as_ref() != Some(&tool_specs(tasks)) {
        findings.push(format!(
            "setup:build-tools installs {installed:?}, not exactly the bundle:build tools {:?}",
            tool_specs(tasks)
        ));
    }

    for key in extra_keys(tasks.as_table(), PACKAGE_MISE_KEYS) {
        findings.push(format!("{TASKS} sets top-level {key}"));
    }
    for name in extra_keys(
        tasks.get("env").and_then(toml::Value::as_table),
        PACKAGE_MISE_ENV,
    ) {
        findings.push(format!("{TASKS} sets env {name}"));
    }

    for key in extra_keys(root_tasks.as_table(), ROOT_MISE_KEYS) {
        findings.push(format!("{ROOT_TASKS} sets top-level {key}"));
    }
    for name in extra_keys(
        root_tasks.get("env").and_then(toml::Value::as_table),
        ROOT_MISE_ENV,
    ) {
        findings.push(format!("{ROOT_TASKS} sets env {name}"));
    }
    for name in extra_keys(
        root_tasks.get("tools").and_then(toml::Value::as_table),
        ROOT_MISE_TOOLS,
    ) {
        findings.push(format!("{ROOT_TASKS} has tool {name}"));
    }
    for (table, reviewed) in ROOT_MISE_EXACT {
        let reviewed: toml::Value = toml::from_str(reviewed).unwrap();
        if root_tasks.get(*table) != Some(&reviewed) {
            findings.push(format!(
                "{ROOT_TASKS} {table} differ from the reviewed values"
            ));
        }
    }
    for name in extra_keys(
        root_tasks.get("hooks").and_then(toml::Value::as_table),
        ROOT_MISE_HOOKS,
    ) {
        findings.push(format!("{ROOT_TASKS} sets hook {name}"));
    }

    // mise also reads mise.local.toml, mise.<env>.toml, .mise.toml,
    // mise/ and .mise/ config, .config/mise*, .tool-versions and legacy
    // .rtx.toml beside each mise.toml.
    for directory in MISE_CONFIG_DIRECTORIES {
        for path in tracked {
            let Some(rest) = path.strip_prefix(directory) else {
                continue;
            };
            let config = match rest.split_once('/') {
                Some((first, _)) => {
                    matches!(first, "mise" | ".mise") || rest.starts_with(".config/mise")
                }
                None => {
                    rest.starts_with("mise")
                        || rest.starts_with(".mise")
                        || rest.starts_with(".rtx")
                        || rest == ".tool-versions"
                }
            };
            let reviewed = matches!(*directory, "" | "packages/kuru-memory/")
                && matches!(rest, "mise.toml" | "mise.lock");
            if config && !reviewed {
                findings.push(format!(
                    "{path} is mise configuration the bundle:build task would load"
                ));
            }
        }
    }
    findings
}

/// Cargo features of the compression crates: the workspace specifications,
/// every member's use of them, and the registry packages that could enable
/// them. A feature change leaves Cargo.lock, and so the key, unchanged.
fn cargo_feature_findings(
    workspace: &toml::Value,
    read: &dyn Fn(&str) -> String,
    packages: &[toml::Value],
) -> Vec<String> {
    let mut findings = Vec::new();
    for key in ["patch", "replace"] {
        if workspace.get(key).is_some() {
            findings.push(format!(
                "Cargo.toml sets {key}, which can replace a compression crate"
            ));
        }
    }
    if workspace["workspace"]
        .get("resolver")
        .and_then(toml::Value::as_str)
        != Some("3")
    {
        findings.push("Cargo.toml no longer uses feature resolver 3".into());
    }
    let dependencies = workspace["workspace"]["dependencies"].as_table().unwrap();
    for (key, spec) in dependencies {
        let name = spec
            .get("package")
            .and_then(toml::Value::as_str)
            .unwrap_or(key);
        if !COMPRESSION_CRATES.contains(&name) {
            continue;
        }
        let reviewed = WORKSPACE_COMPRESSION
            .iter()
            .find(|(crate_name, _, _)| *crate_name == key);
        let default_features = spec
            .get("default-features")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true);
        let features: Vec<_> = spec
            .get("features")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .collect();
        let extra = extra_keys(
            spec.as_table(),
            &["version", "default-features", "features"],
        );
        let matches = reviewed.is_some_and(|(_, reviewed_default, reviewed_features)| {
            *reviewed_default == default_features && *reviewed_features == features.as_slice()
        });
        if !matches || !extra.is_empty() {
            findings.push(format!(
                "Cargo.toml's {key} specification changed its compression features or source"
            ));
        }
    }
    let members = workspace["workspace"]["members"].as_array().unwrap();
    for member in members.iter().filter_map(toml::Value::as_str) {
        let manifest = format!("{member}/Cargo.toml");
        let cargo: toml::Value = toml::from_str(&read(&manifest)).unwrap();
        let targets = cargo.get("target").and_then(toml::Value::as_table);
        let tables =
            std::iter::once(&cargo).chain(targets.into_iter().flatten().map(|(_, target)| target));
        for table in tables {
            for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
                for (key, spec) in table
                    .get(kind)
                    .and_then(toml::Value::as_table)
                    .into_iter()
                    .flatten()
                {
                    let name = spec
                        .get("package")
                        .and_then(toml::Value::as_str)
                        .unwrap_or(key);
                    let plain = spec.get("workspace").and_then(toml::Value::as_bool) == Some(true)
                        && extra_keys(spec.as_table(), &["workspace", "optional"]).is_empty();
                    if COMPRESSION_CRATES.contains(&name) && !plain {
                        findings.push(format!(
                            "{manifest} sets {key} other than `workspace = true`, which can change its features"
                        ));
                    }
                }
            }
        }
        let features = cargo.get("features").and_then(toml::Value::as_table);
        for (feature, values) in features.into_iter().flatten() {
            for value in values
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
            {
                let dependency = value
                    .split_once('/')
                    .map(|(dependency, _)| dependency.trim_end_matches('?'));
                if dependency.is_some_and(|dependency| COMPRESSION_CRATES.contains(&dependency)) {
                    findings.push(format!(
                        "{manifest} feature {feature} enables {value}, a compression crate feature"
                    ));
                }
            }
        }
    }
    let dependents: BTreeSet<&str> = packages
        .iter()
        .filter(|package| package.get("source").is_some())
        .filter(|package| {
            package
                .get("dependencies")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
                .any(|dependency| {
                    COMPRESSION_CRATES.contains(&dependency.split(' ').next().unwrap_or_default())
                })
        })
        .filter_map(|package| package["name"].as_str())
        .collect();
    let reviewed: BTreeSet<&str> = COMPRESSION_DEPENDENTS.iter().copied().collect();
    for name in dependents.difference(&reviewed) {
        findings.push(format!(
            "Cargo.lock package {name} depends on a compression crate and can enable its features"
        ));
    }
    for name in reviewed.difference(&dependents) {
        findings.push(format!(
            "Cargo.lock package {name} no longer depends on a compression crate; update COMPRESSION_DEPENDENTS"
        ));
    }
    findings
}

/// `text` without comment lines and inline `#[cfg(test)]` modules. rustfmt
/// indents a module's items, so its first column-0 `}` closes it; a test
/// module that ends earlier only exposes more code to the checks.
fn production_code(text: &str) -> String {
    let mut code = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]" {
            let inline = lines.peek().is_some_and(|next| {
                let next = next.trim();
                let declaration = next
                    .strip_prefix("pub(crate) ")
                    .or_else(|| next.strip_prefix("pub "))
                    .unwrap_or(next);
                declaration.starts_with("mod ") && declaration.ends_with('{')
            });
            if inline {
                for skipped in lines.by_ref() {
                    if skipped == "}" {
                        break;
                    }
                }
                continue;
            }
        }
        if !trimmed.starts_with("//") {
            code.push_str(line);
            code.push('\n');
        }
    }
    code
}

/// The first module under `root::` in each path of `code`, including each
/// member of a `root::{a, b::c}` group.
fn path_modules(code: &str, root: &str) -> BTreeSet<String> {
    let identifier = |text: &str| -> String {
        text.chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect()
    };
    let prefix = format!("{root}::");
    let mut found = BTreeSet::new();
    for (index, _) in code.match_indices(&prefix) {
        let boundary = code[..index].chars().next_back().is_none_or(|before| {
            !(before.is_ascii_alphanumeric() || before == '_' || before == ':')
        });
        if !boundary {
            continue;
        }
        let rest = code[index + prefix.len()..].trim_start();
        if let Some(group) = rest.strip_prefix('{') {
            let mut depth = 1;
            let mut item = String::new();
            let mut items = Vec::new();
            for character in group.chars() {
                match character {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    ',' if depth == 1 => {
                        items.push(std::mem::take(&mut item));
                        continue;
                    }
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                item.push(character);
            }
            items.push(item);
            found.extend(
                items
                    .iter()
                    .map(|item| identifier(item.trim()))
                    .filter(|name| !name.is_empty() && name != "self"),
            );
        } else {
            let name = identifier(rest);
            if !name.is_empty() {
                found.insert(name);
            }
        }
    }
    found
}

/// In-repository crate names (`kuru_*`) used as path roots in `code`.
fn crate_roots(code: &str) -> BTreeSet<String> {
    code.match_indices("kuru_")
        .filter(|(index, _)| {
            code[..*index].chars().next_back().is_none_or(|before| {
                !(before.is_ascii_alphanumeric() || before == '_' || before == ':')
            })
        })
        .filter_map(|(index, _)| {
            let name: String = code[index..]
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            code[index + name.len()..].starts_with("::").then_some(name)
        })
        .collect()
}

/// One `mod name;` declaration: its file, whether only tests compile it, and
/// whether `#[path]` named the file, which makes it own its directory.
struct Module {
    name: String,
    path: String,
    test: bool,
    by_path: bool,
}

/// The file-backed module declarations in `file`, resolved as rustc does: a
/// `#[path]` is relative to the declaring file's directory; a plain
/// declaration in `lib.rs`, `main.rs`, `mod.rs` or a `#[path]`-loaded file is
/// beside it, and otherwise under a directory named for the file.
fn modules(text: &str, file: &str, by_path: bool) -> Vec<Module> {
    let path = Path::new(file);
    let directory = path.parent().unwrap().to_str().unwrap();
    let owns_directory = by_path
        || matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("lib.rs" | "main.rs" | "mod.rs")
        );
    let nested = if owns_directory {
        directory.to_owned()
    } else {
        file.trim_end_matches(".rs").to_owned()
    };
    let mut found = Vec::new();
    let mut attributes: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with("#[") {
            attributes.push(line);
            continue;
        }
        let declaration = line
            .strip_prefix("pub(crate) ")
            .or_else(|| line.strip_prefix("pub "))
            .unwrap_or(line);
        if let Some(name) = declaration
            .strip_prefix("mod ")
            .and_then(|rest| rest.strip_suffix(';'))
        {
            let named = attributes.iter().find_map(|attribute| {
                attribute
                    .strip_prefix("#[path = \"")
                    .and_then(|rest| rest.strip_suffix("\"]"))
            });
            found.push(Module {
                name: name.to_owned(),
                path: named.map_or_else(
                    || format!("{nested}/{name}.rs"),
                    |named| format!("{directory}/{named}"),
                ),
                test: attributes.contains(&"#[cfg(test)]"),
                by_path: named.is_some(),
            });
        }
        attributes.clear();
    }
    found
}

async fn bundle_build_help() -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    command
        .args(["bundle", "build", "--help"])
        .kill_on_drop(true);
    let output = kuru_delivery::command::output(&mut command, std::time::Duration::from_secs(15))
        .await
        .expect("bundle build --help exceeded fixture deadline or failed to execute");
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

/// The repository's tracked files: what a CI checkout holds, without a
/// maintainer's untracked local mise overrides.
async fn tracked_files(root: &Path) -> BTreeSet<String> {
    let mut command = Command::new("git");
    command
        .args(["ls-files", "-z"])
        .current_dir(root)
        .kill_on_drop(true);
    let output = kuru_delivery::command::output(&mut command, std::time::Duration::from_secs(30))
        .await
        .expect("git ls-files exceeded fixture deadline or failed to execute");
    assert!(
        output.status.success(),
        "git ls-files: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A copy of every file the key-coverage check reads, and its file list.
fn key_repository() -> (tempfile::TempDir, BTreeSet<String>) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    let workspace: toml::Value =
        toml::from_str(&std::fs::read_to_string(source.join("Cargo.toml")).unwrap()).unwrap();
    let mut files: BTreeSet<String> = [
        WORKFLOW,
        "Cargo.lock",
        "Cargo.toml",
        ROOT_TASKS,
        "packages/kuru-archive/src/zip.rs",
        "packages/kuru-delivery/src/bundle.rs",
        "packages/kuru-delivery/src/lib.rs",
        "packages/kuru-delivery/src/main.rs",
        "packages/kuru-memory/mise.lock",
        TASKS,
        "packages/kuru-memory/support/dolt-assets.json",
    ]
    .map(str::to_owned)
    .into();
    for member in workspace["workspace"]["members"].as_array().unwrap() {
        files.insert(format!("{}/Cargo.toml", member.as_str().unwrap()));
    }
    for entry in std::fs::read_dir(source.join("packages/kuru-delivery/src/bundle")).unwrap() {
        files.insert(format!(
            "packages/kuru-delivery/src/bundle/{}",
            entry.unwrap().file_name().to_string_lossy()
        ));
    }
    for file in &files {
        let destination = root.path().join(file);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(source.join(file), destination).unwrap();
    }
    (root, files)
}

fn replace(root: &Path, relative: &str, old: &str, new: &str) {
    let path = root.join(relative);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(old), "{relative} lacks {old:?}");
    std::fs::write(path, text.replacen(old, new, 1)).unwrap();
}

/// Writes and tracks a new file in a key repository copy.
fn add(root: &Path, tracked: &mut BTreeSet<String>, relative: &str, text: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    tracked.insert(relative.to_owned());
}

const HELP: &str =
    "      --manifest <MANIFEST>  [default: packages/kuru-memory/support/dolt-assets.json]";

#[tokio::test]
async fn engine_input_cache_key_covers_exactly_the_build_inputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let help = bundle_build_help().await;
    let findings = key_findings(&root, &help, &tracked_files(&root).await);
    assert!(findings.is_empty(), "{findings:#?}");
    let (copy, tracked) = key_repository();
    let findings = key_findings(copy.path(), &help, &tracked);
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn an_unkeyed_build_input_or_a_non_input_in_the_key_is_rejected() {
    const BUILD: &str = "packages/kuru-delivery/src/bundle/build.rs";
    const BEFORE_TESTS: &str = "#[cfg(test)]\n#[path = \"build_tests.rs\"]";
    const DELIVERY: &str = "packages/kuru-delivery/Cargo.toml";
    let expect = |mutate: &dyn Fn(&Path, &mut BTreeSet<String>), help: &str, finding: &str| {
        let (copy, mut tracked) = key_repository();
        mutate(copy.path(), &mut tracked);
        let findings = key_findings(copy.path(), help, &tracked);
        assert!(
            findings.iter().any(|found| found.contains(finding)),
            "expected {finding:?} in {findings:#?}"
        );
    };
    let before_tests = |root: &Path, line: &str| {
        replace(
            root,
            BUILD,
            BEFORE_TESTS,
            &format!("{line}\n\n{BEFORE_TESTS}"),
        );
    };

    // The key: its hashed files, crate list and shape.
    expect(
        &|root, _| replace(root, WORKFLOW, "'packages/kuru-delivery/src/main.rs', ", ""),
        HELP,
        "the build reads packages/kuru-delivery/src/main.rs, which the key does not cover",
    );
    expect(
        &|root, _| {
            replace(
                root,
                WORKFLOW,
                "hashFiles('",
                "hashFiles('packages/kuru-memory/mise.toml', '",
            );
        },
        HELP,
        "the key hashes packages/kuru-memory/mise.toml, which is not a derived engine build input",
    );
    expect(
        &|root, _| replace(root, WORKFLOW, " zlib-rs\n", "\n"),
        HELP,
        "the archive writer uses locked crate zlib-rs, which ENGINE_ARCHIVE_CRATES omits",
    );
    expect(
        &|root, _| replace(root, WORKFLOW, "-${{ steps.crates.outputs.sha256 }}", ""),
        HELP,
        "the key is not the target, the pin, one hashFiles and the ENGINE_ARCHIVE_CRATES digest",
    );
    expect(
        &|root, _| {
            replace(
                root,
                WORKFLOW,
                "}}-${{ steps.crates",
                "}}-${{ hashFiles('README.md') }}-${{ steps.crates",
            );
        },
        HELP,
        "the key is not the target, the pin, one hashFiles and the ENGINE_ARCHIVE_CRATES digest",
    );

    // The bundle:build task.
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                "tools.go = ",
                "tools.cmake = \"3.31.6\"\ntools.go = ",
            );
        },
        HELP,
        "bundle:build tool cmake@3.31.6 is not a pinned manifest toolchain",
    );
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                "-- bundle build\"",
                "-- bundle build --manifest other.json\"",
            );
        },
        HELP,
        "bundle:build runs Some(\"cargo run -p kuru-delivery --features tooling --locked --bin kuru-delivery -- bundle build --manifest other.json\")",
    );
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                &format!("run = \"{BUNDLE_BUILD_RUN}\""),
                &format!("run = \"sh support/tweak.sh && {BUNDLE_BUILD_RUN}\""),
            );
        },
        HELP,
        "bundle:build runs Some(\"sh support/tweak.sh && cargo run",
    );
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                "tools.go = ",
                "depends = [\"patch:recipe\"]\ntools.go = ",
            );
        },
        HELP,
        "bundle:build sets depends",
    );
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                "tools.go = ",
                "env.GOFLAGS = \"-race\"\ntools.go = ",
            );
        },
        HELP,
        "bundle:build sets env GOFLAGS",
    );
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                "platforms = { linux-x64 = { asset_pattern",
                "postinstall = \"make\", platforms = { linux-x64 = { asset_pattern",
            );
        },
        HELP,
        "bundle:build tool github:mstorsjo/llvm-mingw sets options [\"postinstall\"]",
    );
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                "ucrt-ubuntu-22.04-x86_64.tar.xz]",
                "ucrt-ubuntu-20.04-x86_64.tar.xz]",
            );
        },
        HELP,
        "setup:build-tools installs",
    );

    // The mise configuration the task loads.
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                "[env]\n",
                "[env]\n_.path = [\"{{config_root}}/support/bin\"]\n",
            );
        },
        HELP,
        "packages/kuru-memory/mise.toml sets env _",
    );
    expect(
        &|root, _| {
            replace(
                root,
                TASKS,
                "[env]\n",
                "[tools]\npython = \"3.13\"\n\n[env]\n",
            );
        },
        HELP,
        "packages/kuru-memory/mise.toml sets top-level tools",
    );
    expect(
        &|root, tracked| {
            add(
                root,
                tracked,
                "packages/kuru-memory/mise.local.toml",
                "[env]\nCC = \"gcc-13\"\n",
            );
        },
        HELP,
        "packages/kuru-memory/mise.local.toml is mise configuration",
    );
    expect(
        &|root, tracked| add(root, tracked, ".tool-versions", "make 4.4\n"),
        HELP,
        ".tool-versions is mise configuration",
    );
    expect(
        &|root, _| replace(root, ROOT_TASKS, "[env]\n", "[env]\nCC = \"gcc-13\"\n"),
        HELP,
        "mise.toml sets env CC",
    );
    expect(
        &|root, _| replace(root, ROOT_TASKS, "[tools]\n", "[tools]\nmake = \"4.4\"\n"),
        HELP,
        "mise.toml has tool make",
    );
    expect(
        &|root, _| replace(root, ROOT_TASKS, "lockfile = true", "lockfile = false"),
        HELP,
        "mise.toml settings differ from the reviewed values",
    );

    // The input job's environment and steps.
    expect(
        &|root, _| {
            replace(
                root,
                WORKFLOW,
                "  --target \"$BUILT_TARGET\" \\",
                "  --target \"$BUILT_TARGET\" --jobs 1 \\",
            );
        },
        HELP,
        "the input job's build step is not the reviewed invocation",
    );
    expect(
        &|root, _| {
            replace(
                root,
                WORKFLOW,
                "        id: crates\n        run: |\n          set -eu\n",
                "        id: crates\n        run: |\n          set -eu\n          echo \"$RUNNER_TEMP/bin\" >> \"$GITHUB_PATH\"\n",
            );
        },
        HELP,
        "step \"Digest the locked archive writer crates\" writes GITHUB_PATH",
    );
    expect(
        &|root, _| {
            replace(
                root,
                WORKFLOW,
                "      - name: Read the committed pin\n",
                "      - uses: actions/setup-go@0000000000000000000000000000000000000000\n      - name: Read the committed pin\n",
            );
        },
        HELP,
        "windows-arm64-input uses actions/setup-go",
    );
    expect(
        &|root, _| {
            replace(
                root,
                WORKFLOW,
                "    timeout-minutes: 90\n",
                "    timeout-minutes: 90\n    container: ubuntu:20.04\n",
            );
        },
        HELP,
        "windows-arm64-input sets container",
    );
    expect(
        &|root, _| {
            replace(
                root,
                WORKFLOW,
                "  KURU_MBX: \"0\"\n",
                "  KURU_MBX: \"0\"\n  MISE_ENV: ci\n",
            )
        },
        HELP,
        "differs from the reviewed build environment",
    );

    // The helper's modules and what they use.
    expect(
        &|_, _| {},
        "      --manifest <MANIFEST>  [default: packages/kuru-memory/support/other.json]",
        "the build reads packages/kuru-memory/support/other.json, which the key does not cover",
    );
    // A new helper module outside tests must join the key; rustc resolves it
    // beside the `#[path]`-loaded build.rs.
    expect(
        &|root, _| {
            before_tests(root, "mod layout;");
            std::fs::write(root.join("packages/kuru-delivery/src/bundle/layout.rs"), "").unwrap();
        },
        HELP,
        "the build reads packages/kuru-delivery/src/bundle/layout.rs, which the key does not cover",
    );
    expect(
        &|root, _| {
            std::fs::write(
                root.join("packages/kuru-delivery/src/bundle/packing.rs"),
                "",
            )
            .unwrap();
        },
        HELP,
        "packages/kuru-delivery/src/bundle/packing.rs is neither keyed nor a test module",
    );
    // Recipe constants moved to a new library module stay covered.
    expect(
        &|root, _| {
            replace(
                root,
                "packages/kuru-delivery/src/lib.rs",
                "mod staging;",
                "mod staging;\npub mod recipe;",
            );
            before_tests(root, "const _: &str = crate::recipe::GO_LDFLAGS;");
        },
        HELP,
        "uses module recipe of packages/kuru-delivery, which is neither keyed nor exempt",
    );
    expect(
        &|root, _| before_tests(root, "use super::super::targets;"),
        HELP,
        "uses super::super",
    );
    expect(
        &|root, _| {
            before_tests(
                root,
                "const RECIPE: &[u8] = include_bytes!(\"recipe.txt\");",
            )
        },
        HELP,
        "uses include_bytes!",
    );
    expect(
        &|root, _| before_tests(root, "use kuru_core::recipe;"),
        HELP,
        "uses in-repository crate kuru_core",
    );

    // Cargo features and dependents of the compression crates.
    expect(
        &|root, _| {
            replace(
                root,
                DELIVERY,
                "flate2.workspace = true",
                "flate2 = { workspace = true, features = [\"zlib-rs\"] }",
            );
        },
        HELP,
        "packages/kuru-delivery/Cargo.toml sets flate2 other than `workspace = true`",
    );
    expect(
        &|root, _| {
            replace(
                root,
                DELIVERY,
                "  \"dep:toml\",\n",
                "  \"dep:toml\",\n  \"flate2/zlib-rs\",\n",
            )
        },
        HELP,
        "packages/kuru-delivery/Cargo.toml feature tooling enables flate2/zlib-rs",
    );
    expect(
        &|root, _| {
            replace(
                root,
                "Cargo.toml",
                "flate2 = \"=1.1.10\"",
                "flate2 = { version = \"=1.1.10\", features = [\"zlib-rs\"] }",
            );
        },
        HELP,
        "Cargo.toml's flate2 specification changed",
    );
    expect(
        &|root, _| {
            let path = root.join("Cargo.toml");
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(
                path,
                format!("{text}\n[patch.crates-io]\nflate2 = {{ path = \"x\" }}\n"),
            )
            .unwrap();
        },
        HELP,
        "Cargo.toml sets patch",
    );
    expect(
        &|root, _| {
            let path = root.join("Cargo.lock");
            let text = std::fs::read_to_string(&path).unwrap();
            let start = text.find("name = \"tar\"").unwrap();
            let list = start + text[start..].find("dependencies = [\n").unwrap();
            let at = list + "dependencies = [\n".len();
            std::fs::write(
                path,
                format!("{} \"flate2\",\n{}", &text[..at], &text[at..]),
            )
            .unwrap();
        },
        HELP,
        "Cargo.lock package tar depends on a compression crate",
    );
    // The writer's closure gaining a locked crate must extend the list.
    expect(
        &|root, _| {
            let path = root.join("Cargo.lock");
            let text = std::fs::read_to_string(&path).unwrap();
            let start = text.find("name = \"kuru-archive\"").unwrap();
            let list = start + text[start..].find("dependencies = [\n").unwrap();
            let at = list + "dependencies = [\n".len();
            std::fs::write(path, format!("{} \"tar\",\n{}", &text[..at], &text[at..])).unwrap();
        },
        HELP,
        "the archive writer uses locked crate tar, which ENGINE_ARCHIVE_CRATES omits",
    );
}
