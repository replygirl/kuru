## 1. Rooted subprocess isolation [critical]

- [x] 1.1 @regression (agent) invoke real delivery commands from a child carrying a foreign hook repository environment -> the old implementation altered the disposable caller's config, ref, reflog and objects (/tmp/kuru-repository-isolation-red.log); the fixed implementation selects the requested repository and preserves the entire foreign repository byte snapshot (/tmp/kuru-repository-isolation-green.log). Poisoned paths only target temporary repositories; no process-global environment mutation
- [x] 1.2 @integration (agent) run actual release and Communiqué fixtures -> all 13 notes and 15 workflow tests pass, including actual cog, Git fixture mutation, Communiqué source tools, lost-response temporary-index recovery and preservation of nonlocal fake authentication/global signing settings. Strict Clippy and formatting passed; independent source review found no actionable blocker

## 2. Repository and publication

- [x] 2.1 @integration (agent) run full mise check -> full gate passed in 42.06 seconds with 97.46% workspace line coverage (8993/9227), including behavioral tests, lint, format, docs/link checks and cospec; /tmp/kuru-repository-isolation-check.log
- [~] 2.2 @runtime (agent) observe ordinary pre-push hook, hosted CI and next authorized Release -> defer: final commit and normal push follow local checks and archival; verify caller metadata before/after the real hook, require all platform checks, inspect actual notes and record publication plus final inline Pages separately in the PR
