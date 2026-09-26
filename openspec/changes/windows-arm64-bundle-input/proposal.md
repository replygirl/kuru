## Why

Upstream Dolt publishes engine archives for five targets but has never shipped
windows/arm64, and Dolt v2.3.3 cannot be built for it without cgo (go-mysql-server's
ICU regex and gozstd need a C/C++ toolchain). Windows on Arm support (the follow-on
change) therefore needs Kuru's bundled build-input pipeline to produce a pinned,
reproducible engine archive from pinned sources instead of downloading one, while
every existing target keeps its verified upstream archive unchanged.

Landing the pipeline separately keeps the platform addition reviewable: this change
proves Linux-to-Linux determinism and pins the built archive before any consumer
selects it.

## What Changes

- Dolt asset manifest schema v2: every asset carries an explicit `provenance`
  (`upstream` | `built`). Upstream entries keep every existing field and value; built
  entries carry a `build` object (recipe, `linux-x64` build host, Dolt Go module
  version and `h1:` sum, ICU 78.3 source tarball URL/size/SHA-256, Go 1.26.2 and
  llvm-mingw 20260922 UCRT toolchain pins) and required third-party `notices` (ICU,
  LLVM runtime, mingw-w64 runtime) packaged beside Dolt's `LICENSES`. Field names are
  fixed in design.md.
- An `aarch64-pc-windows-msvc` built entry is added as data only. No Rust target,
  installer, updater or release selects it in this change.
- Both manifest parsers (`kuru-memory` build policy and the independent `kuru-delivery`
  preparer) branch on provenance. `bundle prepare` never fetches or requires a built
  entry when preparing another target, and refuses an unpinned built entry with an
  actionable message. Runtime extraction accepts exactly the manifest-declared notice
  members (none for upstream assets).
- New `kuru-delivery bundle build` subcommand: on linux-x64 only, fetches the pinned
  Dolt module and ICU sources, cross-compiles Dolt with cgo for windows/arm64 against
  static ICU with stub data, and writes the archive in exactly the manifest layout;
  `--print-pins` reports observed pins.
- New `kuru-memory` mise task `bundle:build` and `setup:build-tools`, with Go and
  llvm-mingw pinned as task-scoped tools in `packages/kuru-memory/mise.toml` and a new
  `packages/kuru-memory/mise.lock`. The root `mise.lock` is untouched.
- New workflow file whose `ubuntu-latest` job builds the archive twice into fresh private
  directories,
  proves byte identity, prints and uploads the SHA-256 and size, and verifies the
  committed pin once present. `ci.yml` is not edited.
- `docs/development.md` bundled build inputs: built-from-source inputs, linux-x64 as
  the only build host, and the lockfile-refresh re-pin rule.
- Dolt stays at v2.3.3. No user-facing change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `embedded-runtime`: build inputs gain an explicit upstream/built provenance, a
  pinned reproducible source-build path with required third-party notices, and
  notice-aware local extraction.

## Impact

- `packages/kuru-memory/support/dolt-assets.json` (schema v2, sixth asset),
  `packages/kuru-memory/support/bundle.rs`, `packages/kuru-memory/build.rs`,
  `packages/kuru-memory/src/catalog.rs`, `packages/kuru-memory/src/provision.rs`
  (notice-aware zip extraction), related tests.
- `packages/kuru-delivery/src/bundle.rs`, `packages/kuru-delivery/src/main.rs`
  (new `bundle build`), new build module and fixture tests.
- `packages/kuru-memory/mise.toml`, new `packages/kuru-memory/mise.lock`.
- New `.github/workflows/bundle-build.yml`.
- `packages/kuru-delivery/src/published_windows.rs`: its lenient manifest reader accepts
  schema v2 and decodes only the Windows x64 entry, so an unpinned built entry cannot
  break post-publication verification.
- `docs/development.md`.
- The built asset's `archive_sha256` is coupled to the exact `zip`/`flate2` crate pins;
  a lockfile refresh that moves them must re-pin it.
- Not breaking: the five upstream targets' archives, digests, preparation, embedding
  and extraction are unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
