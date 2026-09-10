# Dependencies

## Blocked by

- [x] `native-delivery` — archive packaging, checksum format and native atomic updater *(archived 2026-09-09)*
- [x] `frozen-release-tooling` — reproducible release source preparation *(archived 2026-09-09)*
- [x] `native-build-tooling` — verified native builds on all four release targets *(archived 2026-09-09)*
- [x] `release-note-generation` — supported model protocol, current source context and output limits *(archived 2026-09-09)*
- [x] `release-note-policy` — complete bounded release drafts *(archived 2026-09-09)*
- [x] `release-repository-isolation` — explicitly rooted delivery commands under hooks *(archived 2026-09-09)*
- [x] `fixture-maintenance-isolation` — deterministic sentinel repository setup *(archived 2026-09-09)*

## Soft-blocked by

None.

## Phase gates

The first release remains the priority. Run 34424298823 passed both validation
gates, version preparation and all four native builds at
7c38954f8e088333cef83bc6bfbb9dc60afb0ec1. Attempts 1 and 5 generated notes but
discarded them at an editorial bullet-count gate; attempts 2–4 stopped before
generation during observed GitHub attestation failures. Nothing was published.
PR #9 merged draft preservation and hook repository isolation at
aac594e8838921a2b34ba8adf762d73934a122b8 after all four hosted platform checks
passed. This branch incorporates that repaired base. Main CI then passed, and
Release 34432603265 published v0.1.0 from that exact source with all eleven jobs
successful, including final inline Pages deployment. The tag, four native archives,
SHA256SUMS and live site have been independently observed; completion evidence is
recorded in verification.md.

Independent implementation and local verification may proceed on this branch
while release repairs complete. The initial release and Pages merge prerequisite
is now met. Record actual release, source and live-install evidence before merging
this follow-up. This branch does not alter the immutable
source of the existing release run. Run 34419735895 was cancelled before
publication because its generated notes were inaccurate and is not a passing gate.
The user already authorized the release and this installation follow-up; no
new dependency or permission decision is required. Active/archive scan found
no other unshipped provider.
