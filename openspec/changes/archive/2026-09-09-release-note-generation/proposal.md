## Why

Release 34419735895 generated notes with false default peers, an incorrect Codex
identity, stale animation behavior and copied prompt instructions. Stronger
wording alone did not fix the Haiku output; the run was cancelled before any
publication. Communiqué's native Anthropic parser prevents using Claude 5
because it rejects thinking blocks.

## What Changes

Use Claude Sonnet 5 through Anthropic's documented OpenAI-compatible endpoint
with Communiqué's supported OpenAI provider, keeping the existing scoped secret.
Supply a bounded snapshot of current product documentation from the exact
selected commit, distinguish it from historical changes, and reject generated
notes exceeding the documented word/bullet limits before writing the artifact.
Prove actual Communiqué request/tool-response handling with isolated fixtures;
evaluate real generated prose during the next authorized release.
Run read-only notes generation alongside final validation so generation problems
can surface earlier; publication still requires every build.

## Capabilities

### Modified Capabilities

None. Accurate release notes were already required; the implementation failed.

## Impact

communique.toml, the native notes adapter and integration tests, the notes job's
credential environment mapping, and release operations documentation. No new
dependency, credential, proxy, publication entrypoint, application provider or
storage change. Keep the strategy-only dispatch and final inline Pages jobs.

## Surfaces

- [ ] interactive
- [x] deploy
- [x] integration
- [x] agent-behavior
