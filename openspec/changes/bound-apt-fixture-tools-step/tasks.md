# Tasks

## 1. Bound the apt fixture-tools step

- [ ] 1.1 Rewrite the "Install Ubuntu native secret-store fixture tools" step in `.github/workflows/native-tests.yml` with the literal update and install fetches under `timeout <attempt budget> sudo apt-get ...`, at most two attempts, the existing guard outside the loop, the attempt number and apt's exit status (124 = timed out) printed on failure, a second failure failing the step with that status, and step `timeout-minutes` = 2 x attempt budget + margin rounded up, and verify each number has its derivation (from the Measured basis in `proposal.md`) in a comment at the site and no `Acquire::Retries` or `dpkg --configure -a` appears
- [ ] 1.2 Split `FIXED_APT` in `packages/kuru-delivery/tests/repo_validation.rs` so native-tests.yml pins the new text and release.yml keeps the old text, update `incident_apt_update_over_every_source_is_rejected`, and add one assertion that un-restricting one apt-get inside the retry loop of native-tests.yml still yields the "over every configured apt source" error naming that command, and verify `mise run //packages/kuru-delivery:test` passes
- [ ] 1.3 Confirm `release.yml` and every other workflow file are unchanged and `docs/development.md` needs no edit, and verify `git diff --stat origin/main` lists only `native-tests.yml`, `repo_validation.rs` and this change's artifacts

## 2. Verification

- [ ] 2.1 Run `mise run lint:tooling` and verify it exits 0
- [ ] 2.2 Run `mise run cospec -- validate bound-apt-fixture-tools-step --strict` and `apply --json` and verify the gate is clear
- [ ] 2.3 On the PR's CI, verify each Ubuntu partition's step log shows attempt 1 succeeding within the attempt budget (record job ids and durations as evidence; if any attempt 2 or a 124 appears, record it rather than passing)
- [ ] 2.4 Record that a vendor stall ending within the step bound with apt's status is reasoned from the shape and not provoked in CI
