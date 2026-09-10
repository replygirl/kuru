## 1. Supported generation contract [critical]

- [x] 1.1 @regression (agent) run actual Communiqué against native thinking and compatible multi-turn tool-call fixtures -> actual binary reached native /v1/messages and rejected thinking; compatible /v1/chat/completions replayed read_file result then submitted notes with Sonnet 5 and the fake Bearer key (11 notes tests passed in /tmp/kuru-notes-generation-after.log; no live-model claim)
- [x] 1.2 @integration (agent) alter a product document without committing and build notes context; attempt generation with tracked edits or nonignored untracked documents -> selected-SHA config and source excluded both uncommitted markers; oversized context rejected; dirty source reached no API. A status.showUntrackedFiles=no probe failed before the explicit flag and passed after it (/tmp/kuru-notes-untracked-before.log and -after.log); ignored build output remained usable
- [x] 1.3 @regression (agent) submit notes with the observed seventeen-bullet shape and excessive word count -> both cases were accepted before the guard (/tmp/kuru-notes-limits-before.log, two unexpected-success failures); after the guard both reject without output/tag, reviewed output remains unchanged, inclusive 450-word/10-item cases pass (/tmp/kuru-notes-generation-after.log)

## 2. Real generated prose and release [critical]

- [~] 2.1 @eval (agent) review next authorized run's actual generated notes against selected source -> defer: requires the merged selected commit and existing Actions-scoped credential; inspect the next authorized Release artifact before publication and retain source traces in the PR evidence, rather than treating local fixtures as model-quality proof
- [~] 2.2 @runtime (agent) observe authorized Release auto -> defer: normal branch merge and hosted gates precede dispatch; the prior run 34419735895 was cancelled before publication, so no successful release or Pages deployment is claimed

## 3. Repository checks

- [x] 3.1 @integration (agent) run notes tests and full mise check -> package tests passed, full gate exited 0 in 56.23 seconds with 97.46% line coverage (8995/9229); /tmp/kuru-notes-full-check.log. Formatting, Clippy, docs/link checks and cospec checks passed. Independent review found no remaining actionable issue
- [~] 3.2 @runtime (agent) observe hosted CI before merge -> defer: branch commit/PR follows this local gate; require both full platform checks and both native build jobs, then record actual outcomes in the PR
