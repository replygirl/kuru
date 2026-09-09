## 1. Automatic recovery [critical]

- [x] 1.1 @regression (agent) Retry a matching published release through the HTTP fixture -> before implementation the retry failed with "release already published"; afterward interrupted_draft_resumes_missing_assets_then_publishes_exact_commit passes and asserts only GET requests on retry
- [x] 1.2 @integration (agent) Exercise real Git histories and a lost commit response -> all 13 release-workflow tests pass, including one accepted API bump with a lost response, reuse after later main work, original index preservation and rejected wrong parents, trees, messages and divergence; published recovery verifies downloaded SHA256SUMS against all remote asset digests and rejects damage or unsafe redirects
- [x] 1.3 @integration (agent) Plan from original and release-candidate checkouts -> real Cocogitto histories preserve every strategy's original result despite descendant release tags; prepared and tagged heads reuse their version, clean-checkout enforcement and downgrade rejection pass
- [x] 1.4 @manual (agent) Inspect dispatch, CLI, artifacts and Pages graph -> bump is the only dispatch input, no manual resume flags remain, native/notes artifact uploads overwrite prior attempts, and final Pages jobs share the successful build's attempt-specific artifact name and released SHA; Actionlint passes
- [x] 1.5 @integration (agent) Run full mise check -> final exit 0 on 2026-09-09 with 213 passing Rust tests and 97.54% line coverage (8681/8900), plus formatting, Clippy, Actionlint, docs and cospec validation/managed drift checks; publication fixture also verifies GitHub's legacy latest-selection policy; threshold remains 90%

## 2. Hosted verification

- [~] 2.1 @runtime (agent) Observe pull request CI -> defer: this record is archived before the branch commit; required macOS/Ubuntu checks run on the submitted PR and their actual result is recorded by GitHub and reported separately
- [~] 2.2 @runtime (agent) Publish and rerun a live release -> defer: no release dispatch is authorized; real Git and isolated GitHub HTTP fixtures exercise recovery locally
