## 1. Reviewable generated prose [critical]

- [x] 1.1 @regression (agent) run real Communiqué fixture with 17 bullets and 451 words -> actual count-policy failures before repair in /tmp/kuru-notes-policy-red.log; all 12 notes tests pass afterward in /tmp/kuru-notes-policy-green.log, including byte-for-byte complete Markdown preservation and alternate list markers. The earlier sandbox bind error was not regression evidence
- [x] 1.2 @integration (agent) run existing source, invalid output, provider error and overwrite tests -> all 12 notes tests pass; inclusive 100000-byte Unicode output is preserved and 100001 bytes rejected, existing output causes no second API request or overwrite, and source/error protections remain intact. Independent review found no actionable blockers

## 2. Repository and live release

- [x] 2.1 @integration (agent) run full mise check -> full gate passed in 46.47 seconds with 97.46% workspace line coverage (8974/9208), including formatting, lint, tests, docs and cospec; /tmp/kuru-note-policy-check-recheck.log. The first run failed when two unchanged connector fixture children received SIGKILL; cause remains unproven. The isolated 33-test connector suite passed before the full successful rerun; /tmp/kuru-note-policy-connectors-recheck.log
- [~] 2.2 @runtime (agent) observe hosted PR/main CI -> defer: requires branch commit and hosted runs; require all four platform jobs and record observed results in the PR before dispatch
- [~] 2.3 @eval (agent) inspect the next actual release-notes artifact against its exact source -> defer: requires merged source and Actions-scoped credentials; inspect actual prose promptly before publication, with source traces in PR evidence. Fixtures establish preservation, not model factuality
- [~] 2.4 @runtime (agent) observe release publication and final inline Pages jobs -> defer: requires successful hosted gates and the next authorized Release; no release, tag or deployed site exists at this local checkpoint. Verify actual assets, exact commit and final inline deployment and record results in the PR
