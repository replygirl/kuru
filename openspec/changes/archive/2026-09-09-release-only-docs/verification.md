## 1. Release-only documentation [critical]

- [x] 1.1 @integration (agent) Compare workflow dependencies and checkout against cospec -> build-docs and deploy-docs now live directly in release.yml; build-docs needs bump and publish and checks out needs.bump.outputs.sha; deploy-docs needs build-docs; the separate pages.yml is removed and permissions remain job-scoped
- [x] 1.2 @integration (agent) Run Actionlint and the full mise gate -> mise run check exited 0 on 2026-09-09 with Actionlint, docs build/content checks, 209 Rust tests, managed drift checks and 97.51% line coverage (8554/8772), above the unchanged 90% threshold
- [x] 1.3 @manual (agent) Review operational instructions -> docs/release.md and AGENTS.md describe only release-owned docs jobs, the exact released SHA, recovery on the original release run and initial publication at the first authorized release

## 2. Runtime boundary

- [x] 2.1 @runtime (agent) Inspect the unintended GitHub Actions Documentation run -> run 34397071952 is canceled, both build/deploy jobs are canceled, and the repository deployments API returns zero deployments
- [~] 2.2 @runtime (agent) Publish a release and observe its final Pages jobs on Ubuntu 24.04 -> defer: no release publication is authorized for this correction; this will be exercised by the next authorized Release run, not by a standalone docs dispatch
