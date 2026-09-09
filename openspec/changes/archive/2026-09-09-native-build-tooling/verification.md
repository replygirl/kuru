## 1. Native build environments [critical]

- [x] 1.1 @integration (agent) run pinned mise with isolated tool/config/cache paths, Rust only and native-build settings -> mise 2026.9.3 installed only locked Rust, built aarch64-apple-darwin in 55.01 seconds, ran kuru 0.1.0 and packaged exactly kuru/LICENSE/README.md; no hk, setup hooks or tracked source drift. Evidence: /tmp/kuru-native-rust-only-probe.log and /tmp/kuru-native-rust-only-probe.sh.
- [~] 1.2 @runtime (agent) run new PR build/package jobs on macos-15-intel and ubuntu-24.04-arm -> defer: the archived workflow must be committed and pushed first; both new hosted jobs are required by ci-gate before merge, with evidence recorded in the PR.
- [x] 1.3 @integration (agent) run full mise check -> MISE_LOCKED=1 mise run check exited 0 in 50.46 seconds; 219 tests, 97.50% line coverage (8955/9185), format, lint, docs, Actionlint and cospec passed. Log: /tmp/kuru-native-build-check.log.

## 2. Generated release notes

- [~] 2.1 @eval (agent) evaluate regenerated Communiqué notes from the corrected release run against source and the observed failure examples -> defer: the real notes service runs with the scoped Actions secret after this correction merges; inspect that artifact against source before publication. Local delivery tests verify configuration propagation, not model factuality.

## Observed failure

Release run 34416373766 passed plan, bump and verify, then job 102686524005
failed installing hk 1.58.1 on macos-x64: no lockfile URL exists. GitHub's actual
v1.58.1 asset list confirms no Intel macOS archive is published. The job requires
Rust for app/archive construction; hk is only invoked by mise's repository setup
postinstall hook. No git write or commit-hook execution occurs in this build job.
Log: /tmp/kuru-release-third-intel.log.

Local Intel execution is not claimed: `arch -x86_64 /usr/bin/true` reports Bad CPU
type on this host. The real Intel runner is therefore required acceptance evidence.

The first real notes artifact at /tmp/kuru-release-third-notes/RELEASE_NOTES.md
invented a v0.2 storage promise, shell subcommand restrictions, loopback demo and
message anonymization. It also described measured coverage as the coverage gate.
Tighten tool instructions and inspect the regenerated artifact before publication.

Completed run 34416373766 built Linux x86_64/arm64 and macOS arm64 successfully;
Intel alone failed setup. Publish and both Pages jobs were skipped. No release
or tag was created by that run.
