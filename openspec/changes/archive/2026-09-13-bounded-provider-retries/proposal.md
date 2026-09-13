## Why

Kuru currently sends a provider request once, except for one subscription 401
rotation, and has no operation-wide accounting for a transient rejected status,
refresh request, or retry delay. A transient provider rejection can therefore
remove a peer, while a future ad-hoc retry could exceed the completion budget or
replay an ambiguous request, accepted stream, or rotating-token POST.

The current refresh owner also needs an explicit, evidenced distinction between
a token POST that never reached HTTP dispatch and every outcome where the token
may have been consumed. A caller timeout must not cancel the owner before it
reconciles the durable pending credential state.

## What Changes

- Add private, operation-wide retry accounting for Responses and ChatGPT
  subscription model catalogs and completions: at most four application sends,
  three provider sends, two retry delays, one logical credential rotation, and
  two refresh sends only when the first token POST is proved undispatched.
- Retry only explicit 429, 500, and 503 provider responses before JSON or SSE
  acceptance; keep quota, access, configuration, model, ambiguous transport,
  malformed, partial, and accepted-stream failures terminal.
- Parse one bounded Retry-After value as delta-seconds or canonical IMF-fixdate,
  apply bounded jitter and the existing absolute operation deadline, and return
  fixed redacted exhaustion diagnostics without replaying when a delay cannot
  fit.
- Keep the detached refresh owner and credential lease through durable
  reconciliation after caller loss. Permit a second refresh POST only for the
  audited auth-local `reqwest::Error::is_connect` connection-acquisition result
  and only with native proof that zero OAuth POSTs reached the fixture; after
  possible dispatch, do not replay and distinguish an exact new generation,
  exact pending state, and unknown publication state.
- Explicitly disable client-library policy retries on the provider and refresh
  clients, while leaving generic MCP, A2A, login, and authentication-management
  behavior unchanged.
- Document the bounded retry policy and its no-replay limits.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `provider-tools`: bounded provider and refresh retry behavior, attempt
  accounting, rejected-status handling, and durable refresh reconciliation.
- `public-documentation`: user-visible retry and no-replay behavior.

## Impact

- `packages/kuru-connectors/src/providers.rs`, `providers/diagnostics.rs`, and
  provider fixture modules: private operation budget, retry schedule, response
  classification, and local HTTP/SSE regressions.
- `packages/kuru-connectors/src/auth.rs`, `auth/http.rs`, and `auth/store.rs`:
  private refresh allowance, pinned pre-dispatch classification, and exact
  pending/new-generation/rollback reconciliation under the existing lease.
- `packages/kuru-connectors` test-only auth and wire fixtures: fake-secret
  native HTTP/1 and proxy observations, including required native Windows
  coverage before merge.
- `Cargo.toml`, `packages/kuru-connectors/Cargo.toml`, and `Cargo.lock`: add the
  already locked transitive `httpdate` 1.0.3 as an exact direct dependency for
  canonical IMF-fixdate parsing, without version drift.
- `docs/protocols.md` and `apps/kuru-docs/reference/configuration.md`: bounded
  retry, retry-delay, and no-replay limitations.
- No public provider trait, completion shape, configuration flag,
  credential-store format, provider routing, MCP/A2A, or tool-effect API change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
