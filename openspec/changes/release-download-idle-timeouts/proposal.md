# Proposal

## Why

Self-update (`kuru --update`: Unix `archive::install`, Windows
`update::replace_running` through `archive::read_asset`) and the CI release
verifier (`published::PublicGitHub`) build their HTTP clients with a flat
60-second whole-request timeout. A healthy but slow link that needs more than
60 seconds to deliver a release archive is cut off part way through even though
bytes keep arriving, and the error carries no context naming the phase that
failed, so the user sees only "update failed". This is fixed-wait audit unit U2
(rank 2): the total timeout measures link speed, not liveness.

## What Changes

- The release-asset client (`archive.rs`) and the published-release client
  (`published.rs`) bound connection setup (`CONNECT_TIMEOUT`, 15 s) and each
  idle read (`READ_IDLE_TIMEOUT`, 30 s) instead of the whole request, matching
  the bundle download client (`bundle.rs`) and `bundle/build.rs`.
- No flat total is added. The transfer is still bounded: connection setup is
  bounded by `CONNECT_TIMEOUT`; in reqwest 0.13.5 the read timeout also arms a
  timer over `send()` itself (`PendingRequest::poll`), so a server that accepts
  and never answers fails within `READ_IDLE_TIMEOUT`; each body read must make
  progress within `READ_IDLE_TIMEOUT`; and every caller passes a byte `limit`
  (`MAX_ARCHIVE_BYTES`, the 64 KiB manifest limit, `METADATA_LIMIT`) that the
  reader enforces, so the number of reads is finite. A slow-but-progressing
  link therefore completes, and a dead one fails within one idle bound.
- The two named constants move from `bundle.rs` to `archive.rs` as
  `pub(crate)`, and `bundle.rs` imports them (a two-line touch outside the two
  files). The direction is forced: `bundle` is compiled only with the
  `tooling` feature, while `archive` is compiled into the shipped `kuru`
  binary without it, so `archive` cannot import from `bundle`. There is still
  one definition. `bundle/build.rs` keeps its literals (out of scope).
- Each site's builder is factored: `archive::release_client()` and
  `published::published_client()` build the production clients
  (`https_only`, both bounds; published keeps `no_proxy`,
  `redirect(Policy::limited(5))` and the `kuru-published-release-client` user
  agent). `archive::download(client, url, limit)` and
  `published::send(request)` + `bounded_body` take the client (or a request
  built from it) by injection, and `PublicGitHub::with_client` lets tests
  construct the reader with their own client. Production never builds a client
  without `https_only`, and there is no `cfg(test)` flag in a production builder.
- Errors name the phase: "send release asset request" / "read release asset
  body" in `archive.rs`, and "send published release request" / "read published
  release response body" in `published.rs`. An unsuccessful status keeps its
  own message.
- No change to verification, installation, replacement, URL checks, size
  limits or redirect policy.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-delivery/src/archive.rs`: constants, `release_client`,
  `download`, phase contexts; test-only `archive/paced_http.rs` fixture
  module (shared with `published.rs` tests) and new tests in `archive/tests.rs`.
- `packages/kuru-delivery/src/published.rs`: `published_client`,
  `with_client`, `send`, read context in `bounded_body`, new tests.
- `packages/kuru-delivery/src/bundle.rs`: imports the two constants instead of
  defining them.
- Public signatures `archive::read_asset(base, name, limit)` and
  `PublicGitHub::new(token)` are unchanged; no new dependencies.
- `apps/kuru-tui/src/cli.rs:2019` still prints "update failed"; whether it
  surfaces the context chain is out of scope.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
