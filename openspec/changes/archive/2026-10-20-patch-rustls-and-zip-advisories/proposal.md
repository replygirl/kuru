# Proposal

## Why

The required `quality / RustSec advisories` job fails on every pull request opened after 2026-10-14, because the RustSec database now reports seven advisories against the locked workspace: RUSTSEC-2026-0346 through 0349 against the transitive `rustls` 0.23.45, and RUSTSEC-2026-0367, 0368 and 0369 against the directly pinned `zip` 8.6.0. The `quality-gate` and `ci-gate` checks depend on that job, so no change can merge green until the lockfile resolves patched releases, and main's own next CI run would fail the same way.

The `zip` advisories concern its extraction path, which Kuru never calls (the strict codec in `kuru-archive` parses archives itself and uses only the upstream writer), and the `rustls` advisories concern client debug output, ECH, brotli certificate decompression and a key-exchange downgrade. None change Kuru's behavior, but the audit gate is a repository invariant and the patched releases are the current stable ones.

## What Changes

- Pin `zip` to the patched major release 9.0.5 in the workspace manifest. Kuru keeps `default-features = false` with only `deflate-flate2`, which the 9.x line still provides, and the writer API it uses (`ZipWriter`, `SimpleFileOptions`, `System`, `DateTime`, `CompressionMethod`) is unchanged.
- Resolve the transitive `rustls` entry to 0.23.46 in `Cargo.lock`. No manifest constraint changes because `rustls` is reached only through `reqwest`, `hyper-rustls`, `tokio-rustls` and `rustls-platform-verifier`.
- Extend the engine build cache key's `ENGINE_ARCHIVE_CRATES` list with the four crates `zip` 9 adds to kuru-archive's locked closure (`nix`, `bitflags`, `cfg_aliases`, `libc`), which the delivery key-coverage test requires exactly.
- Record the patch in the dependency audit page and the cache-key description in the development docs.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`Cargo.toml`, `Cargo.lock`, the `ENGINE_ARCHIVE_CRATES` list in `.github/workflows/bundle-build.yml`, `docs/dependencies.md` and `docs/development.md` only. The lockfile diff is limited to the two package entries plus the `nix` crate that `zip` 9 requires for Unix permission handling, which is already locked at 0.31.3. No application source, public behavior, workflow or threshold changes. The strict ZIP codec's round-trip and malformed-record tests in `kuru-archive`, and the delivery and memory consumers that write archives through it, are the behavioral evidence that the writer bytes remain within Kuru's accepted subset.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
