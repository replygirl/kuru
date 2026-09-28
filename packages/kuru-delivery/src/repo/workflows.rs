//! Workflow rules for one flake class: a CI job must not fetch what it does
//! not use, or what an earlier job in the same run has already verified.
//!
//! Each rule rejects a shape that has failed a real run on an outage of a
//! download the job never needed:
//! - mise: Windows exe shims run `mise x`, which installs every missing
//!   configured tool unless exec auto-install is off for the whole workflow,
//!   and each job names the tools it installs.
//! - apt: `apt-get update` refreshes every configured list, including the
//!   runner image's third-party repositories, so each apt fetch names its
//!   source list and reads no parts directory.
//! - partitions: a fanned-out job prepares bundle inputs offline, importing
//!   the archives one earlier job of the run fetched and verified.
//!
//! The rules read workflow text only. They cannot see a fetch made inside a
//! mise task a step invokes, a tool a job does select, or whether the job
//! that supplies a partition's inputs really runs first.

use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result};
use serde_yaml_ng::Value;

const EXEC_AUTO_INSTALL: &str = "MISE_EXEC_AUTO_INSTALL";
const BUNDLE_OFFLINE: &str = "KURU_DOLT_BUNDLE_OFFLINE";
const MISE_ACTION: &str = "jdx/mise-action@";
const APT_SOURCE_LIST: &str = "Dir::Etc::sourcelist=/";
const APT_NO_SOURCE_PARTS: &str = "Dir::Etc::sourceparts=/dev/null";
const APT_FETCHES: [&str; 5] = [
    "update",
    "install",
    "upgrade",
    "dist-upgrade",
    "full-upgrade",
];
/// Words that may precede the program in a shell command.
const PREFIXES: [&str; 5] = ["sudo", "env", "exec", "command", "time"];

pub(super) fn check(root: &Path, errors: &mut BTreeSet<String>) -> Result<()> {
    let directory = root.join(".github/workflows");
    if !directory.is_dir() {
        return Ok(());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let path = entry?.path();
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("yml" | "yaml")
        ) {
            files.push(path);
        }
    }
    files.sort();
    for path in files {
        let name = format!(
            ".github/workflows/{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        let text = fs::read_to_string(&path).with_context(|| format!("read {name}"))?;
        let workflow: Value =
            serde_yaml_ng::from_str(&text).with_context(|| format!("parse {name}"))?;
        rules(&name, &workflow, errors);
    }
    Ok(())
}

fn is(value: Option<&Value>, expected: bool) -> bool {
    match value {
        Some(Value::Bool(value)) => *value == expected,
        Some(Value::String(value)) => value == if expected { "true" } else { "false" },
        _ => false,
    }
}

fn names(value: Option<&Value>, key: &str) -> bool {
    value.and_then(|value| value.get(key)).is_some()
}

fn rules(name: &str, workflow: &Value, errors: &mut BTreeSet<String>) {
    let mut uses_mise = false;
    let jobs = workflow.get("jobs").and_then(Value::as_mapping);
    for (id, job) in jobs.into_iter().flatten() {
        let id = id.as_str().unwrap_or_default();
        if names(job.get("env"), EXEC_AUTO_INSTALL) {
            errors.insert(format!(
                "{name}: job {id} overrides {EXEC_AUTO_INSTALL}; set it only in the workflow-level env"
            ));
        }
        let partitioned = job
            .get("strategy")
            .and_then(|strategy| strategy.get("matrix"))
            .is_some_and(|matrix| matrix.get("partition").is_some());
        if partitioned && !is(job.get("env").and_then(|env| env.get(BUNDLE_OFFLINE)), true) {
            errors.insert(format!(
                "{name}: partition job {id} must set {BUNDLE_OFFLINE}: \"true\" in its job env and import the bundle inputs an earlier job of the run verified"
            ));
        }
        let steps = job.get("steps").and_then(Value::as_sequence);
        for (index, step) in steps.into_iter().flatten().enumerate() {
            let uses = step.get("uses").and_then(Value::as_str);
            let label = step
                .get("name")
                .and_then(Value::as_str)
                .or(uses)
                .map_or_else(|| format!("step {}", index + 1), str::to_owned);
            let at = format!("{name}: job {id} step {label}");
            if names(step.get("env"), EXEC_AUTO_INSTALL) {
                errors.insert(format!(
                    "{at} overrides {EXEC_AUTO_INSTALL}; set it only in the workflow-level env"
                ));
            }
            if partitioned && names(step.get("env"), BUNDLE_OFFLINE) {
                errors.insert(format!("{at} overrides the partition's {BUNDLE_OFFLINE}"));
            }
            if uses.is_some_and(|uses| uses.starts_with(MISE_ACTION)) {
                uses_mise = true;
                let selected = step
                    .get("with")
                    .and_then(|with| with.get("install_args"))
                    .and_then(Value::as_str)
                    .is_some_and(|tools| !tools.trim().is_empty());
                if !selected {
                    errors.insert(format!(
                        "{at} uses jdx/mise-action without explicit install_args naming the tools the job uses"
                    ));
                }
            }
            let Some(script) = step.get("run").and_then(Value::as_str) else {
                continue;
            };
            if script.contains(EXEC_AUTO_INSTALL) {
                errors.insert(format!(
                    "{at} overrides {EXEC_AUTO_INSTALL} in its script; set it only in the workflow-level env"
                ));
            }
            if partitioned && script.contains(BUNDLE_OFFLINE) {
                errors.insert(format!(
                    "{at} overrides the partition's {BUNDLE_OFFLINE} in its script"
                ));
            }
            for words in commands(script) {
                let words = program(&words);
                match words.first().map(String::as_str) {
                    Some("mise" | "mise.exe") => {
                        uses_mise = true;
                        if matches!(words.get(1).map(String::as_str), Some("install" | "i"))
                            && words[2..].iter().all(|word| word.starts_with('-'))
                        {
                            errors.insert(format!(
                                "{at} runs `mise install` without naming the tools to install"
                            ));
                        }
                    }
                    Some("apt-get" | "apt")
                        if words
                            .iter()
                            .any(|word| APT_FETCHES.contains(&word.as_str())) =>
                    {
                        let option = |prefix: &str| {
                            words
                                .iter()
                                .any(|word| word.trim_start_matches("-o").starts_with(prefix))
                        };
                        if !option(APT_SOURCE_LIST) || !option(APT_NO_SOURCE_PARTS) {
                            errors.insert(format!(
                                "{at} runs `{}` over every configured apt source; name the needed list with -o {APT_SOURCE_LIST}... and -o {APT_NO_SOURCE_PARTS}",
                                words.join(" ")
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if uses_mise
        && !is(
            workflow
                .get("env")
                .and_then(|env| env.get(EXEC_AUTO_INSTALL)),
            false,
        )
    {
        errors.insert(format!(
            "{name}: uses mise, so its workflow-level env must set {EXEC_AUTO_INSTALL}: \"false\""
        ));
    }
}

/// Splits a shell script into the words of each simple command. It joins
/// continuation lines, drops comments and splits at newlines, `;`, `&&`,
/// `||` and `|`. Quotes around a word are removed; nothing is expanded.
fn commands(script: &str) -> Vec<Vec<String>> {
    let joined = script.replace("\\\r\n", " ").replace("\\\n", " ");
    let mut commands = Vec::new();
    for line in joined.lines() {
        let line = line.replace("&&", "\n").replace("||", "\n");
        for command in line.split(['\n', ';', '|']) {
            let words: Vec<String> = command
                .split_whitespace()
                .take_while(|word| !word.starts_with('#'))
                .map(|word| word.trim_matches(['"', '\'']).to_owned())
                .collect();
            if !words.is_empty() {
                commands.push(words);
            }
        }
    }
    commands
}

/// The program and its arguments, after leading prefixes and assignments.
fn program(words: &[String]) -> &[String] {
    let start = words
        .iter()
        .position(|word| {
            !PREFIXES.contains(&word.as_str())
                && !(word.contains('=') && !word.starts_with('-') && !word.starts_with('/'))
        })
        .unwrap_or(words.len());
    &words[start..]
}
