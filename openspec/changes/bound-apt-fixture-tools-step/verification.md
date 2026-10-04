# Verification

## 1. A healthy Ubuntu partition passes the apt step on attempt 1 within the bound [critical]

- [ ] 1.1 @runtime (agent) GitHub-hosted `ubuntu-latest` runner, each `native-tests (ubuntu-latest) / Coverage partition` job of the PR's CI run, reading the "Install Ubuntu native secret-store fixture tools" step log and its `started_at`/`completed_at` -> every partition prints attempt 1, no attempt 2, no exit status 124, and the step finishes well inside the attempt budget; record job ids and durations (healthy baseline: median 15 s, max 43 s over 200 jobs)
- [~] 1.2 @runtime (agent) GitHub-hosted `ubuntu-latest` runner, a vendor stall during `apt-get update` or `install`, expecting that the attempt ends at its budget with status 124, one retry runs, and a second failure fails the step with that status within the step bound instead of a 45-minute job cancel -> defer: a vendor stall cannot be provoked in CI; reasoned from the shape (coreutils `timeout` on each fetch, at most two attempts, `timeout-minutes` as backstop); record the next natural occurrence in the roadmap notes

## 2. Static workflow and validator checks

- [ ] 2.1 @integration (agent) `mise run //packages/kuru-delivery:test` -> the repo_validation tests pass with the split constant: native-tests.yml accepts the new step text, release.yml keeps the old text and is unchanged, and un-restricting one apt-get inside the new retry loop yields the "over every configured apt source" error naming that command
- [ ] 2.2 @integration (agent) `mise run lint:tooling` -> actionlint, shellcheck and `check:repo` accept the workflows and exit 0
- [ ] 2.3 @integration (agent) `git diff --stat origin/main` -> only `.github/workflows/native-tests.yml`, `packages/kuru-delivery/tests/repo_validation.rs` and this change's artifacts differ; `release.yml` is untouched
