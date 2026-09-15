## Context

The current Release DAG publishes a complete GitHub draft and then runs the native Windows installation verifier. Documentation build and deployment are children of that post-publication verifier. The v0.3.0 and v0.3.1 runs demonstrated that a failure in the verifier can occur after the release is already public, so the workflow cannot enforce the required all-green publication boundary.

The existing release tooling already owns the five-target archive inventory, archive sidecar checks, generated `SHA256SUMS`, release-note validation, draft digest comparison, and immutable recovery. The existing native mise fixture already serves release-shaped metadata and package bytes through an isolated loopback server. The correction changes when those existing boundaries run and which exact ZIP the native fixture consumes.

## Goals / Non-Goals

**Goals:**

- Make public GitHub release promotion the sole final Release job and require every staged candidate, native Windows, and documentation gate before it.
- Bind native mise acceptance and final publication to the same complete candidate artifact from the selected version commit.
- Preserve recoverable drafts, exact published digests, immutable assets, release identity, and the existing five-target catalog.
- Keep documentation in the Release workflow and built from the exact selected commit.

**Non-Goals:**

- Claim that isolated loopback delivery is an actual public GitHub download.
- Add a server, package backend, release input, credential, archive format, or runtime behavior.
- Remove the package-owned published-Windows verifier; it remains an optional post-publication diagnostic.
- Make Pages deployment and GitHub release promotion atomic across services.

## Decisions

### Assemble one validated candidate before native acceptance

A read-only `release:tool -- assemble --version VERSION --directory dist --notes RELEASE_NOTES.md` command will consume the five build artifacts and generated release notes. It will reuse `release::assets` to require exactly five bounded regular archives, verify their checksum sidecars, generate `SHA256SUMS`, validate bounded nonempty notes, and emit one attempt-scoped workflow artifact retaining `dist/` plus the notes file. It has no GitHub write token and performs no tag, release, or upload request.

The staged Windows job and final publication job will select that named candidate artifact. Final publication will call the existing publication boundary on the downloaded directory, requiring the already-generated matching `SHA256SUMS` and repeating inventory and digest validation before a fresh or draft tag, upload, or promotion action. Exact already-published recovery retains its existing early identity/digest check and succeeds without requiring local candidate files.

### Exercise the exact staged Windows ZIP through the existing mise fixture

The `//apps/kuru-tui:verify:staged-windows` task will require the candidate's absolute `KURU_STAGED_WINDOWS_ARCHIVE` ZIP path as an explicit read-only input instead of packaging its Cargo test binary. The existing loopback GitHub metadata and download routes, pinned native mise executable, checksum enforcement, installation, activation, offline conversation, persistent reopen, embedded engine/license checks, negative cases, bounds, and cleanup remain. The fixture will verify that the bytes it serves and installs are the exact staged candidate bytes.

This is pre-publication acceptance of release-shaped bytes. The separate published-Windows verifier continues to resolve real public GitHub metadata when a maintainer invokes it, but it is removed from the required Release DAG.

### Put documentation before the only public-release job

After candidate assembly, staged Windows acceptance and documentation build may run in parallel from the exact selected commit. `deploy-docs` will require both successful jobs. The `publish` job will require candidate assembly and successful Pages deployment and will have no dependent child jobs. Therefore candidate, Windows, documentation build, or documentation deployment failure prevents public release promotion.

The final job retains the existing `publish()` behavior: it can safely complete a matching private draft, verify and reuse an exact already-published release after a lost response, and refuses conflicting tags, notes markers, assets, or digests. A rerun remains anchored to the original dispatch and selected commit.

## Operational surface

Runner selection and tool pins remain unchanged: five native build runners produce the candidate, `windows-2025` performs staged acceptance with pinned mise, Ubuntu builds the docs and assembles/publishes the candidate, and the existing Pages environment deploys the site. Candidate assembly and staged acceptance receive no release-write token or provider credentials. Notes retain only their scoped generation credential, Pages retains its current deployment permissions, and only the sole final job receives authority to create or promote a GitHub release. The earlier bump job retains its narrowly scoped GitHub App authority to create or recover the signed version commit.

The workflow still has one manual dispatch input and the existing release concurrency group. All artifacts, documentation, and final release identity remain bound to the signed selected commit.

## Integration contract

The candidate artifact is the job boundary: it contains exactly five native archives, their five checksum sidecars, one generated `SHA256SUMS`, and one bounded nonempty notes file. Its attempt-scoped name is emitted by candidate assembly and consumed by both staged Windows acceptance and final publication, preventing either consumer from selecting an unrelated build artifact.

The mise fixture continues to emulate only the public metadata/download protocol on loopback. It uses the exact candidate ZIP and checksum material as inputs and must not rebuild or silently substitute a package. The final publisher continues to use GitHub's release/tag APIs and digest fields through the existing `GitHub` client; no API schema, route ownership, credential, or ID type changes.

## Risks / Trade-offs

- **Pages can deploy before final GitHub publication and publication can then fail.** The workflow deliberately prioritizes preventing an unverified public release; the two services are not claimed to update atomically. Rerunning the final job revalidates the same candidate and safely resumes publication.
- **Passing loopback acceptance does not prove public CDN or GitHub availability.** Documentation states the boundary, and the existing published verifier remains available for explicit post-publication diagnosis.
- **Cross-job artifacts could otherwise drift between validation and publication.** Both consumers select the candidate artifact name produced by assembly, staged acceptance uses its exact ZIP, and final publication repeats inventory/digest validation.
- **Moving documentation earlier lengthens the critical path to publication.** Staged Windows acceptance and docs build run in parallel, while correctness takes precedence over publishing before required gates settle.
