# Design

## Context

`zip` moves across a major version, so the fix is not a lockfile-only patch like the earlier `patch-rustls-security-advisory` change. Kuru reaches `zip` from one place, the `kuru-archive` writer in `packages/kuru-archive/src/zip.rs`, plus the test fixture generator in its unit tests; every reader path is Kuru's own strict parser. The 9.0 changelog's breaking entries remove deprecated reader fields and methods and rewrite extra-field and ZIP64 handling; the writer entry points Kuru calls are retained, and the `deflate-flate2` feature still selects `flate2` 1.1 without choosing a backend.

## Goals / Non-Goals

**Goals:**
- Clear every reported advisory with current stable releases and the smallest lockfile diff.
- Prove the upstream writer still emits records that Kuru's strict codec accepts, through the existing round-trip and malformed-record tests rather than new upstream-shaped tests.

**Non-Goals:**
- Adopting new `zip` 9 features, changing the accepted ZIP subset, or enabling any additional feature.
- Loosening the audit task, ignoring advisories, or widening any pin.

## Decisions

- Pin `=9.0.5`, the newest release, instead of the first patched `9.0.0`: the 9.0.1 to 9.0.5 fixes are reader and extraction corrections with no writer impact, and the repository policy is current releases with exact pins.
- Update the lockfile with one targeted `cargo update -p rustls --precise 0.23.46` after editing the manifest, which also resolves the manifest's new `zip` pin. A separate `cargo update -p zip` re-resolved unrelated wide-range dependencies (`syn` for `cssparser-macros`, `getrandom` for `tempfile`), so it was discarded to keep the diff to the two advisories.
- Keep `rustls` transitive. Adding a direct pin would duplicate the constraint that `reqwest` already carries.

## Integration contract

- `zip` 9.0.5 from crates.io, `default-features = false`, feature `deflate-flate2` only, which depends on `flate2` 1.1 (Kuru pins 1.1.10). Writer entry points used: `ZipWriter::new`, `add_directory`, `start_file`, `finish`; options `SimpleFileOptions::default().system(System::Unix).last_modified_time(DateTime::default()).unix_permissions(..).compression_method(Stored | Deflated)`. Reader entry point used only by unit tests: `ZipArchive::new`. The 9.0.5 changelog (https://github.com/zip-rs/zip2/blob/v9.0.5/CHANGELOG.md) marks as breaking only the removal of deprecated reader elements, the `*_zip64_comment` methods and the unread `ZipFileData::extra_data_start` field, none of which Kuru calls.
- `rustls` 0.23.46 is reached through `reqwest` 0.13.5 (feature `rustls`), `hyper-rustls` 0.27.10, `tokio-rustls` 0.26.6 and `rustls-platform-verifier` 0.7.1, all of which accept any 0.23.x release; the advisories' fixed version is >=0.23.46 and the resolved entry changes only its version and checksum.
- The advisory database used for the local before/after scans is the package task's isolated clone, refreshed to the revision recorded in verification; CI refreshes its own clone on the runner.

## Risks / Trade-offs

- [Windows arm64 engine rebuild] → the bundle-build workflow keys its cached, pin-verified engine archive on the `Cargo.lock` records of kuru-archive's locked closure, and `bundle_build.rs` requires `ENGINE_ARCHIVE_CRATES` to equal that closure. `zip` 9 adds `nix` with `bitflags`, `cfg_aliases` and `libc`, so the list grows by those four names and the key rotates once; the rebuilt archive must still match the committed pin. The same members written through Kuru's writer by `zip` 8.6.0 and 9.0.5 produced byte-identical archives in a local probe, so the pin is expected to hold.
- [Writer bytes change under `zip` 9] → `kuru-archive`'s writer re-opens its own output through the strict parser before returning, and its tests cover stored and deflated members, directory entries, Unix modes and malformed records; delivery and memory archive consumers run the same codec in their suites and in CI on every platform.
- [New `nix` edge in the dependency graph] → `nix` 0.31.3 is already a locked workspace dependency; no new crate source enters the lockfile.
