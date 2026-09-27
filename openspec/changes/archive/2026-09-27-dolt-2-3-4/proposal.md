# Proposal

## Why

Dolt v2.3.4 fixes a `journal_writer` bootstrap race on cancelled contexts
(dolthub/dolt#11796) and preserves the manifest root across automatic garbage
collection (dolthub/dolt#11747). Both match the pattern of the macOS
uncertain-write failures, so the bundled engine moves from v2.3.3 to v2.3.4
with no other behavior change.

## What Changes

- `packages/kuru-memory/support/dolt-assets.json` (schema v2 unchanged):
  `version` 2.3.4 and `upstream_commit` 373301a5; the four upstream entries
  (darwin arm64, linux arm64, linux x64, windows x64) re-pinned to the v2.3.4
  release assets by URL, sizes and SHA-256. The LICENSES payload is
  byte-identical to v2.3.3, so its pin is unchanged.
- The built `aarch64-pc-windows-msvc` entry moves `build.sources.dolt` to the
  v2.3.4 Go module pseudo-version and `h1:` sum. Its archive and executable
  pins return to the round-one `"unpinned"`/`null` sentinels until the
  linux-x64 `Bundle build` job reports byte-identical builds; the pins it
  prints are then committed (round two). The ICU, LLVM and mingw-w64 notice
  pins, ICU source and Go/llvm-mingw toolchain pins do not depend on the Dolt
  version and stay pinned and verified.
- Tests and fixtures that compare against the committed manifest or the
  engine version string move to v2.3.4. The catalog and built-entry tests
  derive the built target's expectation from its committed pin state instead
  of assuming it is pinned.
- `docs/dependencies.md` records the engine bump and the reasons.

## Impact

Every ordinary Kuru executable embeds the v2.3.4 engine for its target, and
provisioning extracts it into a new `2.3.4` cache version directory. Until
round two lands, the Windows arm64 target cannot be prepared or embedded and
fails with the documented build instruction; no workflow builds that target
from the committed manifest in the meantime. The isolation guarantees and the
uncertain-write fence are unchanged. The existing native install jobs verify
the packaged cold offline conversation on every supported OS.
