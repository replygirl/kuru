## Why

Kuru's ChatGPT login and inference currently invoke the Codex CLI/app-server,
so installing Kuru alone does not provide OpenAI access. That dependency is
incorrect: Kuru is the harness and should authenticate and send requests directly.

This is the final OpenAI authentication correction within the embedded-Dolt and
Windows/macOS/Linux PR. The abandoned Codex bundle and broader native-auth feature
proposals are cancelled, not delivered. Runtime behavior changes and runtime evals
are outside scope; authentication and transport support must nevertheless be complete.

## What Changes

- Implement native ChatGPT browser OAuth with PKCE, optional device login,
  private Kuru credentials, refresh, status and logout.
- Replace the subscription adapter's subprocess dependency with direct OpenAI
  HTTP requests so authenticated Kuru can discover models and complete requests.
  Preserve the existing Responses API path using OPENAI_API_KEY.
- Keep existing provider selections: codex means direct ChatGPT subscription
  access, responses means API-key access, and demo remains offline. Add no new
  auth-selection configuration hierarchy, automatic route selection or billing fallback.
- Remove codex_command and the Codex tool prerequisite with useful migration
  guidance. Verify the complete auth lifecycle and real request protocol through
  ordinary functional tests, then record a live account smoke check separately.

## Capabilities

### Modified Capabilities

- `provider-tools`: correct the requirement that delegated OpenAI authentication
  and inference to an external Codex harness.

## Impact

Changes belong to packages/kuru-connectors, the existing configuration/CLI
integration, their tests, and owning authentication/install documentation. Reuse
the current Responses transport and native private filesystem primitives. The
runtime, peer orchestration, memory design, archive limits and Release/Pages
topology remain outside this correction. Completing this correction and the
remaining native CI checks finishes the PR's intended scope.

## Surfaces

- [x] interactive — existing login, auth status and logout commands
- [x] deploy — native callback listener and removal of the external client prerequisite
- [x] integration — OAuth and direct OpenAI HTTP request protocol
- [ ] agent-behavior — no runtime behavior or evaluation work
