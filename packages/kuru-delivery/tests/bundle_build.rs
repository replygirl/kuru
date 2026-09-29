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

/// The `bundle:build` task's tools with their versions, `{{vars.*}}` resolved.
fn task_tools(tasks: &toml::Value) -> Vec<(String, String)> {
    let vars = tasks.get("vars").and_then(toml::Value::as_table);
    let resolve = |value: &str| {
        let mut resolved = value.to_owned();
        for (name, var) in vars.into_iter().flatten() {
            if let Some(var) = var.as_str() {
                resolved = resolved.replace(&format!("{{{{vars.{name}}}}}"), var);
            }
        }
        resolved
    };
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
            (tool.clone(), resolve(version))
        })
        .collect()
}

const INPUT_JOB: &str = "windows-arm64-input";
const CRATES_ENV: &str = "ENGINE_ARCHIVE_CRATES";
/// In-repository crates whose modules encode the archive bytes: Rust path,
/// Cargo.lock name and directory. The bundle sources also call
/// `kuru_platform` (checked directories) and `crate::command` and
/// `crate::archive` (process launch, SHA-256). Those move or hash bytes that
/// the recipe fixed, and the pin check rejects any change they could make.
const ENCODERS: [(&str, &str, &str); 1] =
    [("kuru_archive", "kuru-archive", "packages/kuru-archive")];

/// Every input the engine build reads, derived from the `bundle:build` task,
/// the helper's module declarations, its CLI default manifest and Cargo.lock,
/// compared with what the input job's cache key hashes. Returns one finding
/// per difference.
fn key_findings(root: &Path, help: &str) -> Vec<String> {
    let mut findings = Vec::new();
    let read = |relative: &str| {
        std::fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"))
    };
    let workflow: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&read(".github/workflows/bundle-build.yml")).unwrap();
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
    let keyed_files: BTreeSet<String> = key
        .split_once("hashFiles(")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(arguments, _)| {
            arguments
                .split(',')
                .map(|argument| argument.trim().trim_matches('\'').to_owned())
                .collect()
        })
        .unwrap_or_default();
    let keyed_crates: BTreeSet<String> = job["env"][CRATES_ENV]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    // The target and pin stay in the key; the crate digest comes from a step
    // that runs first and reads the listed Cargo.lock records.
    for required in ["${{ env.BUILT_TARGET }}", "${{ steps.pin.outputs.sha256 }}"] {
        if !key.contains(required) {
            findings.push(format!("the key no longer names {required}"));
        }
    }
    let digest = steps.iter().position(|step| {
        step["run"].as_str().is_some_and(|run| {
            run.contains(&format!("${CRATES_ENV}")) && run.contains("Cargo.lock")
        })
    });
    match digest {
        Some(index) if index < restore => {
            let id = steps[index]["id"].as_str().unwrap_or_default();
            if !key.contains(&format!("${{{{ steps.{id}.outputs.sha256 }}}}")) {
                findings.push(format!("the key omits the {CRATES_ENV} digest step {id}"));
            }
        }
        _ => findings.push(format!(
            "no step before the restore digests the {CRATES_ENV} Cargo.lock records"
        )),
    }
    for step in steps {
        let run = step["run"].as_str().unwrap_or_default();
        if run.contains("bundle:build") && run.contains("--manifest") {
            findings.push("the input job's build names a manifest the key does not read".into());
        }
    }

    // The task: which program runs, from where, with which tools and env.
    let tasks: toml::Value = toml::from_str(&read("packages/kuru-memory/mise.toml")).unwrap();
    let task = &tasks["tasks"]["bundle:build"];
    if task.get("dir").and_then(toml::Value::as_str) != Some("{{config_root}}/../..") {
        findings.push("bundle:build no longer runs from the repository root".into());
    }
    let env = task.get("env").and_then(toml::Value::as_table);
    for (name, value) in env.into_iter().flatten() {
        if value.as_bool() != Some(false) {
            findings.push(format!(
                "bundle:build sets env {name}, which the key does not cover"
            ));
        }
    }
    let run = task["run"].as_str().unwrap_or_default();
    let words: Vec<_> = run.split_whitespace().collect();
    let value = |flag: &str| {
        words
            .iter()
            .position(|word| *word == flag)
            .and_then(|index| words.get(index + 1))
            .copied()
    };
    let (Some(package), Some(binary)) = (value("-p"), value("--bin")) else {
        findings.push(format!("bundle:build runs no named package binary: {run}"));
        return findings;
    };
    let arguments = words
        .iter()
        .position(|word| *word == "--")
        .map(|index| &words[index + 1..])
        .unwrap_or_default();
    if arguments != ["bundle", "build"] {
        findings.push(format!(
            "bundle:build passes {arguments:?} to the helper; the key covers only `bundle build`"
        ));
    }

    let mut expected = BTreeSet::new();
    let package_dir = format!("packages/{package}");
    let cargo: toml::Value = toml::from_str(&read(&format!("{package_dir}/Cargo.toml"))).unwrap();
    let bin_path = cargo["bin"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|bin| bin["name"].as_str() == Some(binary))
        .and_then(|bin| bin["path"].as_str());
    match bin_path {
        Some(path) => {
            expected.insert(format!("{package_dir}/{path}"));
        }
        None => findings.push(format!("{package} has no binary {binary}")),
    }

    // The helper's subcommand module and every module it declares outside
    // tests. Test-only modules never reach the binary.
    let entry = arguments.first().copied().unwrap_or_default();
    let lib = format!("{package_dir}/src/lib.rs");
    let mut pending: Vec<(String, bool)> = modules(&read(&lib), &lib, false)
        .into_iter()
        .filter(|module| module.name == entry && !module.test)
        .map(|module| (module.path, module.by_path))
        .collect();
    if pending.is_empty() {
        findings.push(format!("{package} declares no {entry} module"));
    }
    let mut test_only = BTreeSet::new();
    while let Some((file, by_path)) = pending.pop() {
        if !expected.insert(file.clone()) {
            continue;
        }
        let text = read(&file);
        for module in modules(&text, &file, by_path) {
            if module.test {
                test_only.insert(module.path);
            } else {
                pending.push((module.path, module.by_path));
            }
        }
        for (rust_name, _, directory) in ENCODERS {
            let prefix = format!("{rust_name}::");
            for (index, _) in text.match_indices(&prefix) {
                let module: String = text[index + prefix.len()..]
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect();
                if !module.is_empty() {
                    expected.insert(format!("{directory}/src/{module}.rs"));
                }
            }
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

/// A copy of every file the key-coverage check reads.
fn key_repository() -> tempfile::TempDir {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    let mut files: Vec<String> = [
        ".github/workflows/bundle-build.yml",
        "Cargo.lock",
        "packages/kuru-archive/src/zip.rs",
        "packages/kuru-delivery/Cargo.toml",
        "packages/kuru-delivery/src/bundle.rs",
        "packages/kuru-delivery/src/lib.rs",
        "packages/kuru-delivery/src/main.rs",
        "packages/kuru-memory/mise.lock",
        "packages/kuru-memory/mise.toml",
        "packages/kuru-memory/support/dolt-assets.json",
    ]
    .map(str::to_owned)
    .into();
    for entry in std::fs::read_dir(source.join("packages/kuru-delivery/src/bundle")).unwrap() {
        files.push(format!(
            "packages/kuru-delivery/src/bundle/{}",
            entry.unwrap().file_name().to_string_lossy()
        ));
    }
    for file in files {
        let destination = root.path().join(&file);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(source.join(&file), destination).unwrap();
    }
    root
}

fn replace(root: &Path, relative: &str, old: &str, new: &str) {
    let path = root.join(relative);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(old), "{relative} lacks {old:?}");
    std::fs::write(path, text.replacen(old, new, 1)).unwrap();
}

const HELP: &str =
    "      --manifest <MANIFEST>  [default: packages/kuru-memory/support/dolt-assets.json]";

#[tokio::test]
async fn engine_input_cache_key_covers_exactly_the_build_inputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let help = bundle_build_help().await;
    let findings = key_findings(&root, &help);
    assert!(findings.is_empty(), "{findings:#?}");
    let copy = key_repository();
    let findings = key_findings(copy.path(), &help);
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn an_unkeyed_build_input_or_a_non_input_in_the_key_is_rejected() {
    const WORKFLOW: &str = ".github/workflows/bundle-build.yml";
    const TASKS: &str = "packages/kuru-memory/mise.toml";
    let expect = |mutate: &dyn Fn(&Path), help: &str, finding: &str| {
        let copy = key_repository();
        mutate(copy.path());
        let findings = key_findings(copy.path(), help);
        assert!(
            findings.iter().any(|found| found.contains(finding)),
            "expected {finding:?} in {findings:#?}"
        );
    };
    expect(
        &|root| replace(root, WORKFLOW, "'packages/kuru-delivery/src/main.rs', ", ""),
        HELP,
        "the build reads packages/kuru-delivery/src/main.rs, which the key does not cover",
    );
    expect(
        &|root| {
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
        &|root| replace(root, WORKFLOW, " zlib-rs\n", "\n"),
        HELP,
        "the archive writer uses locked crate zlib-rs, which ENGINE_ARCHIVE_CRATES omits",
    );
    expect(
        &|root| replace(root, WORKFLOW, "-${{ steps.crates.outputs.sha256 }}", ""),
        HELP,
        "the key omits the ENGINE_ARCHIVE_CRATES digest step crates",
    );
    expect(
        &|root| {
            replace(
                root,
                TASKS,
                "tools.go = ",
                "tools.cmake = \"3.31.6\"\ntools.go = ",
            )
        },
        HELP,
        "bundle:build tool cmake@3.31.6 is not a pinned manifest toolchain",
    );
    expect(
        &|root| {
            replace(
                root,
                TASKS,
                "-- bundle build\"",
                "-- bundle build --manifest other.json\"",
            );
        },
        HELP,
        "the key covers only `bundle build`",
    );
    expect(
        &|root| {
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
        &|_| {},
        "      --manifest <MANIFEST>  [default: packages/kuru-memory/support/other.json]",
        "the build reads packages/kuru-memory/support/other.json, which the key does not cover",
    );
    // A new helper module outside tests must join the key; rustc resolves it
    // beside the `#[path]`-loaded build.rs.
    expect(
        &|root| {
            replace(
                root,
                "packages/kuru-delivery/src/bundle/build.rs",
                "#[cfg(test)]\n#[path = \"build_tests.rs\"]",
                "mod layout;\n\n#[cfg(test)]\n#[path = \"build_tests.rs\"]",
            );
            std::fs::write(root.join("packages/kuru-delivery/src/bundle/layout.rs"), "").unwrap();
        },
        HELP,
        "the build reads packages/kuru-delivery/src/bundle/layout.rs, which the key does not cover",
    );
    expect(
        &|root| {
            std::fs::write(
                root.join("packages/kuru-delivery/src/bundle/packing.rs"),
                "",
            )
            .unwrap();
        },
        HELP,
        "packages/kuru-delivery/src/bundle/packing.rs is neither keyed nor a test module",
    );
    // The writer's closure gaining a locked crate must extend the list.
    expect(
        &|root| {
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
