//! Workflow rules for one flake class: a CI job must not fetch what it does
//! not use, or what an earlier job in the same run has already verified.
//!
//! Each rule rejects a shape that has failed a real run on an outage of a
//! download the job never needed:
//! - mise: Windows exe shims run `mise x`, and `mise run` prepares a task's
//!   tools; each installs every missing configured tool unless its automatic
//!   installation is off for the whole workflow. Each job names the tools it
//!   installs. [`EXEMPTIONS`] names the jobs that still do not.
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

/// mise 2026.9.4 `settings.toml`: `exec_auto_install` (`mise x`, which
/// Windows shims run) and `task.run_auto_install` (`mise run`), both on by
/// default.
const EXEC_AUTO_INSTALL: &str = "MISE_EXEC_AUTO_INSTALL";
const TASK_AUTO_INSTALL: &str = "MISE_TASK_RUN_AUTO_INSTALL";
const AUTO_INSTALL: [&str; 2] = [EXEC_AUTO_INSTALL, TASK_AUTO_INSTALL];
const BUNDLE_OFFLINE: &str = "KURU_DOLT_BUNDLE_OFFLINE";
const MISE_ACTION: &str = "jdx/mise-action@";
/// The kuru-delivery tasks that run one partition (`coverage shard`). A test
/// rejects a fan-out of every such task the package defines.
const PARTITION_TASKS: [&str; 2] = ["coverage:shard", "test:partition"];
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
/// Shells whose `-c` string is itself a command.
const SHELLS: [&str; 4] = ["sh", "bash", "dash", "zsh"];
/// mise options that take the next word as their value. Global options, which
/// mise also accepts after the subcommand: `-C/--cd`, `-E/--env`, `-j/--jobs`
/// (`docs/cli/index.md` at v2026.9.4). `install`: `-j/--jobs`,
/// `--minimum-release-age`, `--shared` (`docs/cli/install.md` at v2026.9.4).
/// Every other option of both is a flag.
const MISE_VALUE_SHORT: [char; 3] = ['C', 'E', 'j'];
const MISE_VALUE_LONG: [&str; 5] = [
    "--cd",
    "--env",
    "--jobs",
    "--minimum-release-age",
    "--shared",
];

/// Jobs that may still leave one automatic installation on, exempted by name
/// in their reviewed shape. The exempted workflow does not set the variable
/// at workflow level, so every other job that uses mise sets it to "false".
struct Exemption {
    workflow: &'static str,
    variable: &'static str,
    /// Each job and its steps: `uses <action>` (with ` install_args <tools>`
    /// for mise-action) or `run <script>`.
    jobs: &'static [(&'static str, &'static [&'static str])],
    reason: &'static str,
}

const EXEMPTIONS: [Exemption; 1] = [Exemption {
    workflow: ".github/workflows/release.yml",
    variable: TASK_AUTO_INSTALL,
    jobs: &[
        (
            "notes",
            &[
                "uses actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
                "uses jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c install_args rust aqua:jdx/hk",
                "run mise run //packages/kuru-delivery:setup",
                "run mise run release:tool -- notes --sha \"$RELEASE_SHA\" --version \"$RELEASE_VERSION\" --output RELEASE_NOTES.md",
                "uses actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
            ],
        ),
        (
            "build-docs",
            &[
                "uses actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
                "uses jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c install_args rust aqua:jdx/hk",
                "run mise run //apps/kuru-docs:setup",
                "uses actions/configure-pages@45bfe0192ca1faeb007ade9deae92b16b8254a0d",
                "run mise run docs:build\nmise run //packages/kuru-delivery:tool -- docs --base \"$KURU_DOCS_BASE\"\n",
                "run printf 'name=github-pages-%s\\n' \"$RUN_ATTEMPT\" >> \"$GITHUB_OUTPUT\"",
                "uses actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9",
            ],
        ),
    ],
    reason: "these release jobs still install every missing configured tool through `mise run`, and notes' setup task runs a bare `mise install --include-task-tools`; follow-up release-notes-docs-tool-scope scopes their tool installation, a release workflow change for the maintainer to decide",
}];

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

fn basename(word: &str) -> &str {
    word.rsplit(['/', '\\']).next().unwrap_or(word)
}

/// A step as an exemption pins it.
fn shape(step: &Value) -> String {
    if let Some(uses) = step.get("uses").and_then(Value::as_str) {
        match step
            .get("with")
            .and_then(|with| with.get("install_args"))
            .and_then(Value::as_str)
        {
            Some(tools) => format!("uses {uses} install_args {tools}"),
            None => format!("uses {uses}"),
        }
    } else {
        let run = step.get("run").and_then(Value::as_str).unwrap_or_default();
        format!("run {run}")
    }
}

/// What one job's steps do, read before any rule applies.
#[derive(Default)]
struct Job {
    uses_mise: bool,
    runs_partition: bool,
}

fn survey(job: &Value) -> Job {
    let mut facts = Job::default();
    let steps = job.get("steps").and_then(Value::as_sequence);
    for step in steps.into_iter().flatten() {
        if step
            .get("uses")
            .and_then(Value::as_str)
            .is_some_and(|uses| uses.starts_with(MISE_ACTION))
        {
            facts.uses_mise = true;
        }
        let script = step.get("run").and_then(Value::as_str).unwrap_or_default();
        for words in commands(script) {
            let words = program(&words);
            if words.first().is_some_and(|word| is_mise(word)) {
                facts.uses_mise = true;
                facts.runs_partition |= words[1..].iter().any(|word| {
                    PARTITION_TASKS
                        .iter()
                        .any(|task| word == task || word.ends_with(&format!(":{task}")))
                });
            }
        }
    }
    facts
}

fn is_mise(word: &str) -> bool {
    matches!(basename(word), "mise" | "mise.exe")
}

fn rules(name: &str, workflow: &Value, errors: &mut BTreeSet<String>) {
    let jobs: Vec<(&str, &Value, Job)> = workflow
        .get("jobs")
        .and_then(Value::as_mapping)
        .into_iter()
        .flatten()
        .map(|(id, job)| (id.as_str().unwrap_or_default(), job, survey(job)))
        .collect();
    let uses_mise = jobs.iter().any(|(_, _, facts)| facts.uses_mise);
    let workflow_env = workflow.get("env");
    for variable in AUTO_INSTALL {
        match EXEMPTIONS
            .iter()
            .find(|exemption| exemption.workflow == name && exemption.variable == variable)
        {
            None => {
                for (id, job, _) in &jobs {
                    if names(job.get("env"), variable) {
                        errors.insert(format!(
                            "{name}: job {id} overrides {variable}; set it only in the workflow-level env"
                        ));
                    }
                }
                if uses_mise && !is(workflow_env.and_then(|env| env.get(variable)), false) {
                    errors.insert(format!(
                        "{name}: uses mise, so its workflow-level env must set {variable}: \"false\""
                    ));
                }
            }
            Some(exemption) => exempted(name, workflow_env, &jobs, exemption, errors),
        }
    }
    for (id, job, facts) in &jobs {
        job_rules(name, id, job, facts, workflow_env, errors);
    }
}

/// An exempted workflow: the named jobs keep their reviewed shape and every
/// other job that uses mise opts out itself.
fn exempted(
    name: &str,
    workflow_env: Option<&Value>,
    jobs: &[(&str, &Value, Job)],
    exemption: &Exemption,
    errors: &mut BTreeSet<String>,
) {
    let variable = exemption.variable;
    let reason = exemption.reason;
    let exempt: Vec<&str> = exemption.jobs.iter().map(|(id, _)| *id).collect();
    if names(workflow_env, variable) {
        errors.insert(format!(
            "{name}: sets {variable} at workflow level, so the exemption for jobs {} is stale; remove it ({reason})",
            exempt.join(", ")
        ));
    }
    for (id, steps) in exemption.jobs {
        let found = jobs.iter().find(|(job_id, _, _)| job_id == id);
        let reviewed = found.is_some_and(|(_, job, _)| {
            let actual: Vec<String> = job
                .get("steps")
                .and_then(Value::as_sequence)
                .into_iter()
                .flatten()
                .map(shape)
                .collect();
            !names(job.get("env"), variable) && actual == *steps
        });
        if !reviewed {
            errors.insert(format!(
                "{name}: job {id} is exempted from {variable} only in its reviewed shape and no longer has it; scope its tool installation and remove the exemption, or review the new shape ({reason})"
            ));
        }
    }
    for (id, job, facts) in jobs {
        if exempt.contains(id) || !facts.uses_mise {
            continue;
        }
        if !is(job.get("env").and_then(|env| env.get(variable)), false) {
            errors.insert(format!(
                "{name}: job {id} uses mise and is not exempted, so its job env must set {variable}: \"false\" while the workflow level does not"
            ));
        }
    }
}

fn job_rules(
    name: &str,
    id: &str,
    job: &Value,
    facts: &Job,
    workflow_env: Option<&Value>,
    errors: &mut BTreeSet<String>,
) {
    // Any fan-out that runs a partition task, or has a partition axis, reads
    // the run's verified inputs, whatever its axis is called.
    let partitioned = job
        .get("strategy")
        .and_then(|strategy| strategy.get("matrix"))
        .is_some_and(|matrix| facts.runs_partition || matrix.get("partition").is_some());
    let job_env = job.get("env");
    let offline = if names(job_env, BUNDLE_OFFLINE) {
        is(job_env.and_then(|env| env.get(BUNDLE_OFFLINE)), true)
    } else {
        is(workflow_env.and_then(|env| env.get(BUNDLE_OFFLINE)), true)
    };
    if partitioned && !offline {
        errors.insert(format!(
            "{name}: partition job {id} must run with {BUNDLE_OFFLINE}: \"true\" from its job env, or the workflow env without a job override, and import the bundle inputs an earlier job of the run verified"
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
        for variable in AUTO_INSTALL {
            if names(step.get("env"), variable) {
                errors.insert(format!(
                    "{at} overrides {variable}; set it only in the workflow-level env"
                ));
            }
        }
        if partitioned && names(step.get("env"), BUNDLE_OFFLINE) {
            errors.insert(format!("{at} overrides the partition's {BUNDLE_OFFLINE}"));
        }
        if uses.is_some_and(|uses| uses.starts_with(MISE_ACTION)) {
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
        for variable in AUTO_INSTALL {
            if script.contains(variable) {
                errors.insert(format!(
                    "{at} overrides {variable} in its script; set it only in the workflow-level env"
                ));
            }
        }
        if partitioned && script.contains(BUNDLE_OFFLINE) {
            errors.insert(format!(
                "{at} overrides the partition's {BUNDLE_OFFLINE} in its script"
            ));
        }
        for words in commands(script) {
            command_rules(&at, program(&words), errors);
        }
    }
}

fn command_rules(at: &str, words: &[String], errors: &mut BTreeSet<String>) {
    let Some(first) = words.first() else {
        return;
    };
    match basename(first) {
        "mise" | "mise.exe" => {
            let operands = operands(&words[1..]);
            if matches!(operands.as_slice(), ["install" | "i"]) {
                errors.insert(format!(
                    "{at} runs `mise install` without naming the tools to install"
                ));
            }
        }
        "apt-get" | "apt"
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
        // add-apt-repository(1) on noble: it updates the package cache, over
        // every source, unless `-n, --no-update` is given.
        "add-apt-repository" | "apt-add-repository" => {
            let no_update = words[1..].iter().any(|word| {
                word == "--no-update"
                    || word.strip_prefix('-').is_some_and(|short| {
                        !short.starts_with('-')
                            && short.chars().all(|flag| flag.is_ascii_alphabetic())
                            && short.contains('n')
                    })
            });
            if !no_update {
                errors.insert(format!(
                    "{at} runs `{}`, which refreshes every configured apt source; pass -n (--no-update) and update only the needed list",
                    words.join(" ")
                ));
            }
        }
        _ => {}
    }
}

/// Whether a mise option takes the next word as its value.
fn takes_value(word: &str) -> bool {
    if word.starts_with("--") {
        return MISE_VALUE_LONG.contains(&word);
    }
    // In a short cluster such as `-yj`, the first value option takes the rest
    // of the cluster, or the next word when it ends the cluster.
    let flags: Vec<char> = word.chars().skip(1).collect();
    flags
        .iter()
        .position(|flag| MISE_VALUE_SHORT.contains(flag))
        .is_some_and(|position| position + 1 == flags.len())
}

/// A mise command's words that are neither options nor option values:
/// its subcommand, then that subcommand's arguments.
fn operands(arguments: &[String]) -> Vec<&str> {
    let mut operands = Vec::new();
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            operands.extend(words.map(String::as_str));
            break;
        }
        if word.len() > 1 && word.starts_with('-') {
            if takes_value(word) {
                words.next();
            }
        } else {
            operands.push(word.as_str());
        }
    }
    operands
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

/// The program and its arguments, after leading prefixes, their options
/// (such as `sudo -E`) and assignments. A shell's `-c` string is read as the
/// command it runs; [`commands`] has already removed its quotes.
fn program(words: &[String]) -> &[String] {
    let start = words
        .iter()
        .position(|word| {
            !PREFIXES.contains(&basename(word))
                && !word.starts_with('-')
                && !(word.contains('=') && !word.starts_with('/'))
        })
        .unwrap_or(words.len());
    let words = &words[start..];
    if words
        .first()
        .is_some_and(|shell| SHELLS.contains(&basename(shell)))
    {
        let mut index = 1;
        let mut string = false;
        while let Some(word) = words.get(index) {
            if word == "-o" || word == "+o" {
                index += 2;
            } else if word.starts_with("--") && word.len() > 2 {
                index += 1;
            } else if word.len() > 1 && word.starts_with(['-', '+']) {
                string |= word.contains('c');
                index += 1;
            } else {
                break;
            }
        }
        if string {
            return program(&words[index.min(words.len())..]);
        }
    }
    words
}
