## 1. Prepare hosted repository conventions

- [x] 1.1 Review cospec GitHub settings and Kuru publication contents; verify clean history review and record intended private/default-branch settings.
- [x] 1.2 Add repository metadata, authenticated source-clone instructions, contributor/security guidance and branch workflow documentation; verify links and source-install instructions.
- [x] 1.3 Add a stable aggregate CI status and conventional PR-title workflow; verify workflow lint and title-check positive/negative cases.

## 2. Verify source for publication

- [x] 2.1 Run the full mise gate, record evidence, validate/archive this source preparation and make the final local commit before the initial push.

## Evidence

- Read cospec live merge settings and default-branch rules; adapted the independent
  Rust CI gate and preserved private visibility. Existing history: 192 blobs
  reviewed, no credential-shaped content or tracked local state artifacts found.
- `replygirl/kuru` created; GitHub reports PRIVATE and ADMIN access. Squash-only
  merges, branch deletion, auto-merge and focused repository features confirmed.
- Full `mise run check` exits 0: 147 Rust tests, 11 installer tests, 97.64% coverage.
  PR-title positive/negative probes and actionlint pass. Repository metadata and
  private source/release install instructions are prepared; release tags are absent.
- Source is ready for its final archive/commit and initial push. Hosted CI and
  default-branch safeguard activation are publication follow-through after that push.
