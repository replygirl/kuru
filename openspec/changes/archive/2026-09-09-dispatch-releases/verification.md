## 1. Conventional immutable preparation [critical]

- [x] 1.1 @integration (agent) invoke real Cocogitto in disposable git histories -> observe initial, feature, patch, breaking, explicit and no-op versions without writes
- [x] 1.2 @integration (agent) stamp disposable Cargo workspaces and exercise GitHub API fixtures -> observe scoped lock changes, no-op stamps, expected-head failures and conflicting immutable tags

Observed: the installed Cocogitto 7.0.0 ran against disposable conventional histories in `packages/kuru-delivery/tests/release_workflow.rs`. Tests cover the initial version, automatic patch/minor, pre-1.0 breaking behavior, explicit major/minor/patch, post-1.0 breaking behavior and HEAD already tagged. Rust stamping changes only workspace package versions, rejects inconsistent lockfiles without writing the manifest and remains idempotent. HTTP fixtures inspect base64 commit contents and expectedHeadOid, reject a moved main, and preserve conflicting tags.

## 2. Complete recoverable publication [critical]

- [x] 2.1 @integration (agent) validate native archive/checksum fixtures and drive publication fixtures -> observe missing/corrupt assets rejected, complete draft publication and published-release preservation
- [x] 2.2 @integration (agent) validate the actual release workflow with Actionlint and inspect exact SHA dependencies -> observe typed environment inputs, least job permissions and reusable Pages SHA binding

Observed: local HTTP fixtures interrupt an asset upload, verify no publication occurs, then resume the same draft and upload only missing assets. They reject missing/corrupt local assets, empty notes, unowned or corrupt drafts, conflicting tags, malformed API responses, invalid tag object chains and corrupted uploaded digests. Published releases remain untouched. Archive extraction and native installer behavior are covered by the delivery package's separate archive suite. Actionlint accepted `release.yml`; inspection confirms every verification/build/notes/publish job checks out the selected full SHA and Pages receives that same SHA only after publication. No remote release or tag was created during these checks.

## 3. Release notes and operational documentation

- [x] 3.1 @integration (agent) exercise installed Communiqué against a local API fixture and inspect initial context -> observe supported model request, output notes file and root-commit inventory
- [x] 3.2 @integration (agent) run release tests, formatting and repository gates -> mise run check exited 0 on 2026-09-09, including 205 Rust tests and 97.55% workspace line coverage (8435/8647), preserving the 90% gate
- [x] 3.3 @manual (agent) review release instructions against implemented commands and credential names -> observe no presumed secret contents, no automatic release dispatch and working authenticated installation instructions
- [x] 3.4 @eval (agent) inspect notes from the real-tool fixture against seeded release history and root context -> observe factual user-facing fixture changes without fabricated publication claims

Observed: `mise -C packages/kuru-delivery exec -- cargo test --manifest-path ../../Cargo.toml --locked -p kuru-delivery --features tooling --test release_workflow --test release_notes` passed all 14 tests (9 release, 5 notes). Strict Clippy passed for the release binary and these tests. The actual Communiqué 1.3.5 binary sends `claude-haiku-4-5-20251001`, includes root inventory and writes a `v0.1.0` heading; fake API authentication and unsupported thinking-block responses fail without creating output or tags. Existing reviewed notes are preserved. The fixture's seeded text matches its supplied history/context; this checks the notes integration contract, not the quality of live model inference. Documentation names `ANTHROPIC_API_KEY_COMMUNIQUE`, the app variable/key, native mise commands and private authenticated downloads. The combined repository gate passed. The app was verified with Contents write/Metadata read and no events, scoped access to replygirl/kuru, and bypass only on the PR/check ruleset. No release/tag was dispatched.
