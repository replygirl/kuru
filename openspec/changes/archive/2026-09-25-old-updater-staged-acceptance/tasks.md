## 1. Existing staged Windows acceptance

- [x] 1.1 Extend `packages/kuru-delivery/tests/support/mise_acceptance.rs` to authenticate the fixed official v0.4.2 Windows release manifest and ZIP against pinned digests before extracting its executable; reject absent or altered bytes before any old executable launch.
- [x] 1.2 Add the exact old installed `kuru.exe` update subcase against the already verified staged candidate directory, with assertions for installed candidate bytes, absent managed support from the old updater, and exact five-file pure generation from the new executable. Native execution remains a hard prepromotion gate.
- [x] 1.3 Keep `apps/kuru-tui/tests/windows_mise.rs` and its package-owned `verify:staged-windows` task as the single prepromotion acceptance route; record focused local checks and require the exact native Windows release job before publication.

## 2. Release evidence

- [x] 2.1 Update `docs/release.md` with the staged old-executable check and the distinct postpublication public-download proof; verify the local text and Cospec gates without claiming an unrun native result.

## Observed checks and release obligation

- `mise run //apps/kuru-tui:typecheck` passed on macOS after package-owned preparation reused the existing checksum-verified local Dolt archive through the documented offline override. The initial preparation attempt stopped before compilation because GitHub DNS lookup failed. Host typecheck does not execute or typecheck the Windows-only acceptance body.
- `mise run format:rust`, `git diff --check`, strict Cospec validation (0 errors, 0 warnings), and the actual Cospec apply gate (exit 0, clear) passed. The `docs/release.md` edit has no new link or command; final branch documentation checks remain part of normal hooks and CI.
- The native `native_mise_installs_and_activates_exact_staged_windows_archive` case is **unrun** here. The existing `verify-staged-windows` release job runs it on the exact staged candidate; `publish` already depends on that job. Its Windows pass is a hard requirement before promotion. A later public-download check remains distinct because the new release URL does not exist before publication.
