# Proposal

## Why

Dolt v2.3.5 (release commit `ad65af6c`, published 2026-09-16) is the current
upstream release, so the bundled engine moves from v2.3.4 to v2.3.5. Its
release notes contain nothing aimed at the storage path. The macOS "memory
service write outcome is uncertain" failure already recurred on v2.3.4 (PR #115
head 0c468e4e, macOS partition 2, job 108615291670), so this bump is not
claimed to fix it; it exists so any further work on that issue starts from
up-to-date dependencies.

## What Changes

- `packages/kuru-memory/support/dolt-assets.json` (schema v2 unchanged):
  `version` 2.3.5 and `upstream_commit` ad65af6c; the four upstream entries
  (darwin arm64, linux arm64, linux x64, windows x64 zip) re-pinned to the
  v2.3.5 release assets by URL, sizes and SHA-256. The LICENSES payload is
  byte-identical to v2.3.4, so its pin is unchanged.
- The built `aarch64-pc-windows-msvc` entry moves `build.sources.dolt` to the
  v2.3.5 Go module pseudo-version and `h1:` sum. Its archive and executable
  pins return to the round-one `"unpinned"`/`null` sentinels until the
  linux-x64 `Bundle build` job reports byte-identical builds; the pins it
  prints are then committed (round two). The ICU, LLVM and mingw-w64 notice
  pins, ICU source and Go/llvm-mingw toolchain pins do not depend on the Dolt
  version and stay pinned and verified.
- Tests and fixtures that compare against the committed manifest or the
  engine version string move to v2.3.5. The catalog and delivery built-entry
  tests already derive the built target's expectation from its committed pin
  state; the memory committed-manifest test asserts the exact round-one state
  until round two restores its observed-pin assertions.
- `docs/dependencies.md` records the engine bump and its reason.

## Impact

Every ordinary Kuru executable embeds the v2.3.5 engine for its target, and
provisioning extracts it into a new `2.3.5` cache version directory. Until
round two lands, the Windows arm64 target cannot be prepared or embedded and
fails with the documented build instruction; no workflow builds that target
from the committed manifest in the meantime. The isolation guarantees and the
uncertain-write fence are unchanged. The existing native install jobs verify
the packaged cold offline conversation on every supported OS.
