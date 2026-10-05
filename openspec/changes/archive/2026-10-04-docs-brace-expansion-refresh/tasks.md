# Tasks

## 1. Dependency acceptance

- [x] 1.1 Review the upstream 5.0.9-to-5.0.12 implementation and confirm the existing Dependabot-generated lockfile changes only the intended package entry.
- [x] 1.2 Verify the regenerated lockfile through the app-owned clean install and confirm dependency manifests and locks remain unchanged afterward.
- [x] 1.3 Run documentation formatting, lint, build and content/link checks with the pinned app toolchain.
- [x] 1.4 Run strict Cospec validation and managed-file drift checks, then record observed local results and the hosted CI merge requirement.

Archive this record through Cospec before the final branch commit. Require fresh
green PR CI and green main after merge; the historical Windows failures on the
old PR head are not acceptance evidence for this updated branch.

## Observed evidence

On 2026-10-04, the branch was refreshed onto main `c4331452`. The upstream
[5.0.9-to-5.0.12 comparison](https://github.com/juliangruber/brace-expansion/compare/v5.0.9...v5.0.12)
was reviewed, including iterative comma parsing, nesting and rewrite bounds,
and corresponding regression tests. Kuru's lockfile diff contains only the
package version, registry URL and integrity hash.

`MISE_LOCKED=1 mise run format:fix` passed, including the app-owned clean
`npm ci --no-audit --no-fund` install with Node 26.10.0 and npm 12.1.0.
`MISE_LOCKED=1 mise run docs:check` passed formatting, lint, VitePress build,
and public artifact/local link/anchor checks. No dependency manifest or lockfile
changed during those commands. Strict change validation reported zero errors
and warnings; `cospec:managed:check` reported no drift. The apply gate exited 0.

Hosted native tests and installation/update acceptance are not claimed as local
passes. They remain required checks on the refreshed PR before merging.
