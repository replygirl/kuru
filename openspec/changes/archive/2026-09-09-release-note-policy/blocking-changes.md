# Dependencies

## Blocked by

- [x] `release-note-generation` — exact-source notes context and supported provider transport *(archived 2026-09-09)*

## Soft-blocked by

None.

## Scope and authorization

The user has authorized completing the Release workflow and its necessary fixes.
There is no new dependency or permission decision. The active-change scan in this
worktree found only this repair. The separately prepared direct-release-install
branch consumes the finished release and is not a prerequisite for this repair.

## Operational evidence

Release 34424298823 passed source preparation, both full validation gates and all
four native builds. Attempts 1 and 5 generated drafts but rejected their bullet
counts; attempts 2–4 failed before generation during an observed GitHub
attestation-service outage. No release or docs publication occurred.
