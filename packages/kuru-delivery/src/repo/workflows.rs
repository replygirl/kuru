//! Workflow rules for one flake class: a CI job must not fetch what it does
//! not use, or what an earlier job in the same run has already verified.
//!
//! Each rule rejects a shape that has failed a real run on an outage of a
//! download the job never needed:
//! - mise: Windows exe shims run `mise x`, and `mise run` prepares a task's
//!   tools; each installs every missing configured tool unless its automatic
//!   installation is off for the whole workflow. Each job names the tools it
//!   installs.
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
/// apt configuration keys, which apt compares without regard to case
/// (apt.conf(5)).
const APT_SOURCE_LIST: &str = "dir::etc::sourcelist";
const APT_SOURCE_PARTS: &str = "dir::etc::sourceparts";
/// apt reads this file of configuration after its defaults (apt.conf(5)).
const APT_CONFIG: &str = "APT_CONFIG";
const APT_FETCHES: [&str; 5] = [
    "update",
    "install",
    "upgrade",
    "dist-upgrade",
    "full-upgrade",
];
/// A word that may precede the program in a shell command, with its options
/// that take the next word as their value: short letters, then long names.
/// sudo(8), env(1), timeout(1), nice(1), time(1) and bash's `exec -a`, on
/// Ubuntu 24.04. timeout also takes its duration before the program.
type Prefix = (&'static str, &'static [char], &'static [&'static str]);
const PREFIXES: [Prefix; 8] = [
    (
        "sudo",
        &['u', 'g', 'h', 'p', 'C', 'D', 'r', 't', 'U', 'T', 'R'],
        &[
            "--user",
            "--group",
            "--host",
            "--prompt",
            "--close-from",
            "--chdir",
            "--role",
            "--type",
            "--other-user",
            "--command-timeout",
            "--chroot",
        ],
    ),
    // `-S` is not listed: its string is the command, and [`commands`] has
    // already split it into words.
    ("env", &['u', 'C'], &["--unset", "--chdir"]),
    ("exec", &['a'], &[]),
    ("command", &[], &[]),
    ("time", &['f', 'o'], &["--format", "--output"]),
    ("timeout", &['k', 's'], &["--kill-after", "--signal"]),
    ("nice", &['n'], &["--adjustment"]),
    ("nohup", &[], &[]),
];
/// Shells whose `-c` string is itself a command.
const SHELLS: [&str; 4] = ["sh", "bash", "dash", "zsh"];
/// mise options that take the next word as their value. Global options, which
/// mise also accepts after the subcommand: `-C/--cd`, `-E/--env`, `-j/--jobs`
/// (`docs/cli/index.md` at v2026.9.4). `install`: `-j/--jobs`,
/// `--minimum-release-age`, `--shared` (`docs/cli/install.md`). `upgrade`:
/// `-j/--jobs`, `-x/--exclude`, `--minimum-release-age`
/// (`docs/cli/upgrade.md`). `bootstrap`: `--from`, `--adopt`, `--from-dir`,
/// `--only`, `--skip` (`docs/cli/bootstrap.md`). Every other option of these
/// is a flag.
const MISE_VALUE_SHORT: [char; 4] = ['C', 'E', 'j', 'x'];
const MISE_VALUE_LONG: [&str; 11] = [
    "--cd",
    "--env",
    "--jobs",
    "--minimum-release-age",
    "--shared",
    "--exclude",
    "--from",
    "--adopt",
    "--from-dir",
    "--only",
    "--skip",
];
/// mise subcommands that install every configured tool when given no tool:
/// `install` (`i`), `upgrade` (`up`) and `bootstrap` (`bs`), whose phase 6
/// installs the versioned tools (`docs/cli/*.md` at v2026.9.4).
const MISE_INSTALLS: [(&str, &str); 3] = [("install", "i"), ("upgrade", "up"), ("bootstrap", "bs")];

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
    if names(workflow_env, APT_CONFIG) {
        errors.insert(format!(
            "{name}: sets {APT_CONFIG} at workflow level; apt must read only the lists each fetch names"
        ));
    }
    for variable in AUTO_INSTALL {
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
    for (id, job, facts) in &jobs {
        job_rules(name, id, job, facts, workflow_env, errors);
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
    if names(job_env, APT_CONFIG) {
        errors.insert(format!(
            "{name}: job {id} sets {APT_CONFIG}; apt must read only the lists each fetch names"
        ));
    }
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
        if names(step.get("env"), APT_CONFIG) || step_script(step).contains(APT_CONFIG) {
            errors.insert(format!(
                "{at} sets {APT_CONFIG}; apt must read only the lists each fetch names"
            ));
        }
        if partitioned && names(step.get("env"), BUNDLE_OFFLINE) {
            errors.insert(format!("{at} overrides the partition's {BUNDLE_OFFLINE}"));
        }
        if uses.is_some_and(|uses| uses.starts_with(MISE_ACTION)) {
            // mise-action runs `mise install <install_args>`, so its
            // arguments must name a tool, as a run step's must.
            let selected = step
                .get("with")
                .and_then(|with| with.get("install_args"))
                .and_then(Value::as_str)
                .is_some_and(|arguments| {
                    let words: Vec<String> =
                        arguments.split_whitespace().map(str::to_owned).collect();
                    !operands(&words).is_empty()
                });
            if !selected {
                errors.insert(format!(
                    "{at} uses jdx/mise-action without explicit install_args naming the tools the job uses"
                ));
            }
        }
        let script = step_script(step);
        if script.is_empty() {
            continue;
        }
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

fn step_script(step: &Value) -> &str {
    step.get("run").and_then(Value::as_str).unwrap_or_default()
}

fn command_rules(at: &str, words: &[String], errors: &mut BTreeSet<String>) {
    let Some(first) = words.first() else {
        return;
    };
    match basename(first) {
        "mise" | "mise.exe" => {
            let operands = operands(&words[1..]);
            if let [subcommand] = operands.as_slice()
                && let Some((name, _)) = MISE_INSTALLS
                    .iter()
                    .find(|(name, alias)| subcommand == name || subcommand == alias)
            {
                errors.insert(format!(
                    "{at} runs `mise {name}` without naming the tools to install"
                ));
            }
        }
        "apt-get" | "apt"
            if words
                .iter()
                .any(|word| APT_FETCHES.contains(&word.as_str())) =>
        {
            if !apt_restricted(&words[1..]) {
                errors.insert(format!(
                    "{at} runs `{}` over every configured apt source; name the needed list with one -o Dir::Etc::sourcelist=/..., make the last -o Dir::Etc::sourceparts=/dev/null, and pass no -c",
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

/// Whether an apt command reads only the list it names. apt applies `-o`
/// settings in order, so the last parts setting wins, and a `-c` file is
/// read where it appears (apt-get(8), apt.conf(5)). Each word of a short
/// cluster before `o` or `c` is a flag.
fn apt_restricted(arguments: &[String]) -> bool {
    let mut settings = Vec::new();
    let mut config_file = false;
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        let (option, attached) = if let Some(long) = word.strip_prefix("--") {
            match long.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (long, None),
            }
        } else if let Some(short) = word.strip_prefix('-') {
            match short.find(['o', 'c']) {
                Some(position) => {
                    let value = &short[position + 1..];
                    (
                        if short[position..].starts_with('o') {
                            "option"
                        } else {
                            "config-file"
                        },
                        (!value.is_empty()).then_some(value),
                    )
                }
                None => continue,
            }
        } else {
            continue;
        };
        let value = |words: &mut std::slice::Iter<'_, String>| {
            attached
                .map(str::to_owned)
                .or_else(|| words.next().cloned())
        };
        match option {
            "option" => {
                if let Some(setting) = value(&mut words) {
                    settings.push(setting);
                }
            }
            "config-file" => config_file = true,
            _ => {}
        }
    }
    let values = |key: &str| -> Vec<String> {
        settings
            .iter()
            .filter_map(|setting| setting.split_once('='))
            .filter(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.to_owned())
            .collect()
    };
    let lists = values(APT_SOURCE_LIST);
    !config_file
        && matches!(lists.as_slice(), [list] if list.starts_with('/'))
        && values(APT_SOURCE_PARTS)
            .last()
            .is_some_and(|parts| parts == "/dev/null")
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

/// Whether a prefix option takes the next word as its value. In a short
/// cluster the first value option takes the rest of the cluster, or the next
/// word when it ends the cluster.
fn prefix_value(word: &str, (_, short, long): &Prefix) -> bool {
    if word.starts_with("--") {
        return long.contains(&word);
    }
    let flags: Vec<char> = word.chars().skip(1).collect();
    flags
        .iter()
        .position(|flag| short.contains(flag))
        .is_some_and(|position| position + 1 == flags.len())
}

/// The program and its arguments, after leading prefixes, their options
/// (such as `sudo -E` or `sudo -u runner`), timeout's duration and
/// assignments. A shell's `-c` string is read as the command it runs;
/// [`commands`] has already removed its quotes.
fn program(words: &[String]) -> &[String] {
    let mut start = 0;
    let mut prefix: Option<&Prefix> = None;
    let mut duration = false;
    while let Some(word) = words.get(start) {
        start += if let Some(found) = PREFIXES.iter().find(|(name, _, _)| *name == basename(word)) {
            prefix = Some(found);
            duration = found.0 == "timeout";
            1
        } else if word.starts_with('-') {
            if prefix.is_some_and(|prefix| prefix_value(word, prefix)) {
                2
            } else {
                1
            }
        } else if word.contains('=') && !word.starts_with('/') {
            1
        } else if duration {
            duration = false;
            1
        } else {
            break;
        };
    }
    let words = &words[start.min(words.len())..];
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
