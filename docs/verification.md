# Verification record

## Visual update

The 2026-09-09 design update passed `mise run check` with **127 Rust tests**,
**11 installer tests** and **97.62% workspace line coverage** (5997/6143), including
Clippy, formatting, tooling checks and strict cospec validation. No dependencies
changed. Seven visual integration tests exercise all frameworks, real peer events,
relationship membership, route animation, reduced motion, Markdown, long selectors,
compact status and Unicode input. The real PTY test checks RGB output, F6, both
motion startup modes, normal chat and terminal restoration.

Actual Ratatui cell exports were visually inspected, and the OpenAI-backed build
was opened in the cmux `Kuru · new design` tab with isolated preview state.
`KURU_VISUAL_ARTIFACTS=/tmp/kuru-visual mise exec -- cargo test -p kuru --test visual`
regenerates HTML/text review artifacts from the actual renderer.

## Initial harness baseline

Validated locally on macOS arm64 on 2026-09-09 with the versions recorded in
[the dependency audit](dependencies.md).

- `mise run check` passed: **115 Rust behavioral tests**, **11 installer tests**,
  Clippy with warnings denied, Rust/TOML/Python formatting, ShellCheck, Ruff,
  Actionlint, repository invariants and cospec validation/drift checks.
- Whole-workspace LLVM line coverage: **97.66%** (4999/5119). No application
  modules are excluded. A separate cross-check omitting trailing `cfg(test)`
  modules and the test-support file measured **97.25%** (3469/3567) for production
  code. The enforced standard LLVM threshold is 90%.
- Real subprocess/HTTP fixtures cover Codex JSON-RPC, Responses native function
  replay, MCP stdio/Streamable HTTP, and A2A 1.0. These are deterministic protocol
  tests, not a claim that every external server/provider has been exercised.
- Native Codex 0.153.4 discovered eight account-visible models and current effort
  strings. A live GPT-6 Astra request completed under the restricted inference
  permission profile. A second live test ran the whole Freudian peer pool,
  executed Kuru's `file_read`, and returned the exact random file marker.
- Real PTY tests exercised chat, selectors, commands, resize, paste and clean
  terminal restoration. Actor cancellation tests verify provider-future teardown
  and permit recovery. The normal OpenAI app was also launched in a cmux pane.
- Source installation through mise and direct Cargo installation both succeeded
  in temporary prefixes. Installed binaries ran all four frameworks and resumed
  sessions. Release packaging, checksum installation and CLI self-update were
  exercised; corruption was rejected while retaining the prior executable.

Fresh interactive browser login was not repeated; live checks used the existing
supported Codex login. Login/device-login/status/logout command routing is tested
with a subprocess fixture. Public GitHub CI and release publication have not run
because this is a local repository; the workflows are statically validated.

The [protocol documentation](protocols.md) lists the supported MCP/A2A subset.
Jungian collective memory is project-scoped. Framework profiles are computational
interpretations; these checks establish software behavior, not psychological or
clinical validity.
