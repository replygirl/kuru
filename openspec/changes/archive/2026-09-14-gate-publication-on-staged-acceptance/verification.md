## 1. Candidate assembly and publication boundary [critical]

- [x] 1.1 @regression (agent) assemble a real five-target candidate directory with archive sidecars and notes through the delivery command -> the package-owned `release_workflow` target passed 20/20 and the real CLI assembled six verified publication inputs without GitHub credentials
- [x] 1.2 @unit (agent) exercise missing, extra, nonregular, oversized, checksum-invalid, and invalid-notes candidates -> missing, corrupt, directory-substituted, sparse-oversized, extra-sidecar, stale-manifest, and empty-notes controls all failed before remote writes
- [x] 1.3 @integration (agent) run the existing draft, digest, conflicting-tag, and already-published recovery fixtures with the assembled candidate -> the 20/20 integration target preserved private draft recovery and exact published-release reuse

## 2. Exact staged Windows acceptance [critical]

- [~] 2.1 @regression (agent) invoke the native Windows mise acceptance with an explicit prebuilt candidate ZIP -> defer: the ignored exact staged-archive test and required mise task are implemented and source-reviewed, but actual execution requires the hosted Windows Release job
- [~] 2.2 @integration (agent) run the fixture's checksum, corrupt, missing, fallback, isolation, offline persistence, and cleanup cases against staged input -> defer: application typecheck and review passed; full behavior remains required on native Windows before publication

## 3. Publication-last Release DAG [critical]

- [x] 3.1 @regression (agent) validate the real Release workflow dependency graph -> the focused test failed on the exact pre-fix workflow with `missing release job assemble-candidate`, the current workflow was restored byte-for-byte, and the same test then passed 1/1
- [x] 3.2 @integration (agent) inspect the checked-in workflow's exact `needs` chain, default success conditions, permissions, and publication commands -> the 20/20 workflow target and actionlint passed with one final publication command and no dependency-condition override or publication child
- [~] 3.3 @e2e (agent) observe the corrected Release workflow on its exact selected commit -> defer: the complete Release DAG can execute only after merge and remains required before the workflow may promote a public release

## 4. Documentation and repository checks

- [x] 4.1 @unit (agent) run documentation content, link, and formatting checks -> `mise run //apps/kuru-docs:check` passed its build, lint, format, link, and content checks with the revised release ordering
- [x] 4.2 @integration (agent) run repository format, lint, typecheck, workflow/action validation, delivery tests, and strict Cospec validation -> delivery/application typechecks, Rust formatting, actionlint, focused delivery tests, strict Cospec validation with zero errors or warnings, and diff checks passed
