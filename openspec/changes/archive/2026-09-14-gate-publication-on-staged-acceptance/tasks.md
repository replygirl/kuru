## 1. Complete candidate boundary

- [x] 1.1 Add `release:tool -- assemble --version VERSION --directory dist --notes RELEASE_NOTES.md` as a delivery-owned non-publishing candidate command that reuses `release::assets`, requires the exact five archives and checksum sidecars plus bounded notes, generates `SHA256SUMS`, and verifies the complete output without a GitHub write
- [x] 1.2 Add focused candidate inventory and failure-path regressions, and retain the existing draft digest, immutable asset, conflicting-tag, and already-published recovery behavior when `publish()` consumes the validated candidate

## 2. Staged Windows acceptance

- [x] 2.1 Add `//apps/kuru-tui:verify:staged-windows` with a required absolute `KURU_STAGED_WINDOWS_ARCHIVE`, and change the existing native mise acceptance fixture to consume that exact ZIP rather than packaging the test binary while preserving its simulated metadata, checksum, negative, isolation, runtime, persistence, bound, and cleanup checks
- [x] 2.2 Add a native regression that fails on the old rebuilt-input path and proves the installed executable and served ZIP come from the selected candidate artifact without calling the public release endpoint

## 3. Publication-last workflow

- [x] 3.1 Assemble and upload one attempt-scoped candidate artifact after all five builds and notes, then run staged Windows acceptance and documentation build from that selected commit and candidate
- [x] 3.2 Make documentation deployment depend on both staged Windows acceptance and documentation build, make `publish` the sole final job consuming and revalidating the same candidate, and remove the published-Windows verifier from the required workflow graph while keeping its package-owned task available
- [x] 3.3 Extend the checked-in Release workflow structural regression to require the exact candidate, staged Windows, documentation build, deployment, and final publication dependencies with no condition override, publication child, publication step, or release-publication authority outside the final job

## 4. Documentation and verification

- [x] 4.1 Update `AGENTS.md`, release operations, and curated installation guidance for staged exact-ZIP acceptance, pre-publication Pages deployment, sole-final publication, optional public diagnosis, and non-atomic Pages/GitHub recovery
- [x] 4.2 Run locally available focused candidate, publication recovery, workflow-graph, documentation, formatting, lint, typecheck, strict Cospec, and diff checks; record native Windows staged acceptance and the complete hosted Release run as deferred without describing loopback acceptance as a public download

## Evidence

- The package-owned `release_workflow` target passed 20/20 through mise with its pinned Cocogitto tool, covering assembly, exact inventory failures, remote-write exclusion, publication recovery, and the checked-in workflow structure.
- The new workflow regression failed against the exact pre-fix workflow with `missing release job assemble-candidate`; after restoring the current workflow byte-for-byte, the same focused test passed 1/1.
- Candidate controls cover a missing or corrupt archive, an archive replaced by a directory, a sparse archive over 256 MiB, an extra sidecar, stale `SHA256SUMS`, and empty notes; each prevents remote writes.
- Delivery and application typechecks, Rust formatting, actionlint, documentation build/lint/format/link/content checks, strict Cospec validation, and diff checks passed.
- Native execution of the staged Windows test and the complete corrected Release DAG remain deferred to required hosted Windows and post-merge Release jobs.
