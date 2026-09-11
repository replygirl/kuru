## 1. Recover bounded immutable downloads

- [x] 1.1 Add and run a real HTTP 500-to-success regression against the unchanged downloader; retain its observed failure before implementation.
- [x] 1.2 Add at most two delayed retries before body acceptance under the existing 120-second total budget; verify eligible/permanent/status/body/Retry-After and cancellation boundaries with actual local HTTP fixtures.
- [x] 1.3 Update owning developer guidance and run granular checks plus normal hooks/native CI; record actual results, strictly validate and archive the completed fix through cospec.

Final integration passed at a1547ed; see verification's final evidence. The
following RED/GREEN and pending-status account is retained as historical evidence.

The unmodified downloader failed the actual loopback HTTP 500-to-200 regression
with Cargo exit 101: zero cases passed, one failed in 0.01 seconds after
3.85 seconds of compilation. The failure reports the fixture's HTTP 500 at the
real preparation call, before expected publication. Root ran the owning mise
task; no production change was present. Evidence: `/tmp/kuru-bundle-retry-red.log`.

After repair, all nine focused bundle tests passed in 6.42 seconds after
3.54 seconds of compilation. Strict delivery Clippy, formatting and strict
cospec validation passed. Independent review found no material issue; the
development guide now distinguishes error responses carrying Retry-After from
successful responses. Full hook and native integration remain pending.
Evidence: `/tmp/kuru-bundle-retry-green.log`,
`/tmp/kuru-bundle-retry-clippy.log`, `/tmp/kuru-bundle-retry-format.log`,
`/tmp/kuru-bundle-retry-final-validate.log` and
`/tmp/kuru-bundle-retry-independent-review.md`.
