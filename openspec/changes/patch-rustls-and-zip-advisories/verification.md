# Verification

## 1. Advisory gate clears on the locked workspace [critical]

- [x] 1.1 @regression (agent) run `mise run //packages/kuru-delivery:audit:advisories` after a fresh `audit:advisories:refresh` against the unpatched lockfile, then against the patched one -> the unpatched scan fails with RUSTSEC-2026-0346, 0347, 0348, 0349 (`rustls` 0.23.45) and 0367, 0368, 0369 (`zip` 8.6.0); the patched scan passes with zero vulnerabilities on the same database revision. Observed on database `b19b2c83c2493961ccb12f9fc4699a00fb2c6c44`: the unpatched b1339bdc scan listed exactly those seven advisories and exited 1; the patched scan listed none and exited 0.
- [ ] 1.2 @integration (agent) push the branch and read the `quality / RustSec advisories`, `quality-gate` and `ci-gate` checks -> all succeed on the final head, with the unchanged-lock diff step passing.

## 2. Strict ZIP codec accepts the upgraded writer

- [x] 2.1 @integration (agent) run `mise run //packages/kuru-archive:test` -> round-trip, stored/deflated, directory, Unix-mode and malformed-record tests pass against `zip` 9.0.5's writer output. Observed: the package suite passed with 0 failures in the typecheck-and-test run.
- [x] 2.2 @integration (agent) run `mise run //packages/kuru-delivery:test` -> the bootstrap, Windows archive and shell-support fixtures that write archives through the codec pass. Observed: 503 passed, 0 failed after the `ENGINE_ARCHIVE_CRATES` extension; the first run's single failure was that key-coverage test naming the four omitted crates.
- [ ] 2.3 @integration (agent) run workspace typecheck, `lint:rust`, `lint:windows` and `docs:check` -> all pass with the new releases; CI native partitions on every supported platform pass with the memory package's archive consumers. Observed locally: `typecheck`, `format:check`, `lint:rust`, `lint:windows` and `docs:check` all exited 0; CI evidence pending.
- [ ] 2.4 @equivalence (agent) write the same directory, executable, text and empty members through `kuru_archive::zip::write` with `zip` 8.6.0 (main) and 9.0.5 (this branch) and compare -> byte-identical archives, so the Windows arm64 engine pin and its rebuilt archive agree. Observed: both probes produced 38455 identical bytes (`cmp` clean); the CI engine rebuild under the rotated `ENGINE_ARCHIVE_CRATES` key is the live confirmation.
