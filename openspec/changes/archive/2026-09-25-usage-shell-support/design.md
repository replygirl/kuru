## Context

The shipped `completions` and `man` commands already run before configuration or authority setup and produce a fixed five-file target-paired sidecar. The CLI definition is Clap; the current generators are `clap_complete` and `clap_mangen` plus a local recursive roff writer.

## Goals / Non-Goals

**Goals:** Keep Clap as the only authored command model while using published Usage APIs for shell scripts, completion answers and manual rendering. Keep installed default completions self-contained and preserve the release support inventory.

**Non-Goals:** A parser rewrite, dynamic project-aware completion, a new release envelope, or automatic installation of the Usage executable.

## Decisions

1. Pin compatible `usage-lib`, `usage-cli` and `usage-argv` 6.11.1. Convert `Cli::command()` directly with `usage::Spec::from(&command)` under Usage's `clap` feature. Do not use the older `clap_usage` bridge or maintain a parallel KDL command model.
2. Generate default scripts with public `usage_argv::script::script("kuru", shell)`, whose Bash variant is self-contained. The scripts ask Kuru's hidden `__complete_word__` callback; public `CompletionRequest::parse` handles each shell's line, public `usage_cli::complete_answer` answers from the Clap-derived spec, and public `usage_argv::complete::render_request` writes the native shell protocol. The answer command does not call Usage's process dispatcher. For explicit external mode use `usage_lib::complete` with `usage_bin="usage"`; this generic Bash script requires the optional `bash-completion` package. Render the manual with `ManpageRenderer`.
3. Keep the generated support commands and hidden completion route ahead of all configuration, trust, memory and provider setup. Preserve the existing output/member bounds and release install/update checks.

## Risks / Trade-offs

- Usage's script protocol and candidate formatting may change across releases → exact compatible pins and focused four-shell protocol tests, including spaces, apostrophes, Unicode and descriptions.
- The hidden completion route could appear in public help or perform startup work → keep it hidden in Clap and assert pure early dispatch from a foreign directory.
- A generated script could require an uninstalled Usage executable or Bash shell package → default to Usage's native Kuru-hosted script with no such prerequisite, and document/test the optional external mode separately.
- `usage-cli`, the only path to the default self-hosted answer engine, has no optional Cargo features as published (verified against its Cargo.toml: `rmcp`'s "server" transport, `tera`, `tokio` and `env_logger` are all unconditional) → no narrower dependency set is selectable; measured release-binary impact is a real, modest ~3.16 MiB (≈4.9%) versus the pre-Usage baseline, well inside the 128 MiB shipping cap. A same-process process-launching audit above `native_completion_answer_output` in `apps/kuru-tui/src/cli.rs` confirms none of that crate's own process-spawning code (`usage::sh::sh`, its `exec`/`shell` subcommands, the unix-only `exec` crate) is reachable from the Clap-derived spec this decision uses.

## Operational surface

Generation and completion answers run inside the selected Kuru executable on each existing native target. They open no listener, require no secrets, create no project state, and do not launch another process in the default mode. The optional external mode invokes a separately installed `usage` executable at completion time; it is never required by release installation. Cargo pins compatible Usage library versions in the app binary; existing native release target and archive bounds are unchanged.

## Integration contract

The Clap command tree remains authoritative. The published Usage `From<&clap::Command> for Spec` conversion supplies the answer engine and manual with the same tree; no independent schema, route, mount, or ID mapping is introduced. Default scripts use Usage-argv's documented `__complete_word__` protocol, parsed before Clap or Kuru startup by its public request parser; an optional external script calls Usage's `complete-word` directly with an embedded spec. Focused shell fixtures assert both protocols and keep all release support files paired with the selected executable.
