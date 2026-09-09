## Context

The action's `mise install --locked` flag affects that invocation. Root aliases
launch new mise processes, and their normal auto-installs may extend lockfiles.
The failed bump job installed six unneeded root tools before the Rust stamp
command, then rejected unrelated tracked changes. A clean local installation
with the same mise version added Taplo's computed checksum to root mise.lock.

## Decisions

Set `MISE_LOCKED=1` at Release workflow scope so normal lock writers remain frozen
in child mise processes. Set `MISE_TASK_RUN_AUTO_INSTALL=false` only in bump,
build and publish jobs: each explicitly installs locked Rust/hk before its native
task, and these subcommands do not execute the notes tools declared on the
generic release task. Keep automatic preparation for validation, notes and docs,
under inherited frozen locks. The docs setup task installs Node before npm;
disabling all task preparation would risk computing PATH before Node exists.

Check `git diff --exit-code HEAD` after bump setup and before stamping. Preserve
the native Cargo-only commit guard and report offending file names if it fires.
Never restore unexpected changes or add tool lockfiles to allowed release edits.

## Operational surface

Existing hosted runners, permissions, secrets and four native architectures stay
unchanged. The verified mise version remains 2026.9.3. No local services, bind
addresses or release artifacts are added. Docs remain the final release jobs.

## Integration contract

The pinned mise implementation accepts inherited `MISE_LOCKED` and
`MISE_TASK_RUN_AUTO_INSTALL`. Its normal lock writers return without writing when
locked. Task tools are additive across the configuration hierarchy, so
`--include-task-tools` must not be described as a narrow installer even with
positional tool names. Existing full package setup is retained where needed.

## Risks / Trade-offs

A Rust-only job that later needs another external executable must add an explicit
locked installation. Real fresh-cache probes verify frozen file contents and
tool usability; the normal release integration suite retains unrelated-change
rejection. Hosted publication remains necessary evidence after merge.
