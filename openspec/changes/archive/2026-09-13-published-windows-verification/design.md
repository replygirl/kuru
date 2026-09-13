## Context

The Release workflow validates packages before publication. Native tests also
exercise mise against simulated release metadata, but neither observes the
public tag, assets, ordinary GitHub backend, or installed package after
publication. The existing manual Windows procedure cannot run from a maintainer's
Mac and does not gate documentation deployment.

## Goals / Non-Goals

**Goals:** add one delivery-owned native Windows verification path that composes
existing archive, process, and acceptance primitives; verify both publication
provenance and cold persistent behavior; retain a bounded reviewable receipt.

**Non-Goals:** create another release workflow, mutate or replay publication,
add application crate dependencies to delivery tooling, replace candidate
package tests, or change Kuru's machine JSON format.

## Decisions

1. A tooling-only delivery subcommand owns publication queries, independent
   digest checks, runtime observations, and receipt writing. A small shared
   Windows mise-isolation helper is also used by the existing native acceptance
   fixture. A public verification framework was rejected because only these two
   callers need the same environment boundary.
2. The workflow resolves the pinned native mise executable before the verifier
   clears its environment. The verifier then uses an absolute path and isolated
   config, data, cache, state, system, temporary, and GitHub roots. Custom URL or
   API replacement was rejected because it would stop testing the public route.
3. Mise installation and independent provenance checks are separate evidence.
   The verifier resolves the exact public tag to the expected commit, validates
   the complete release inventory and SHA256SUMS, inspects the ZIP's selected
   executable, and compares it with the installed executable.
4. Runtime acceptance invokes only the installed executable through mise. It
   uses an empty offline engine cache and demo provider, then verifies session
   continuity and extracted Dolt/license bytes against the checked-out manifest.
5. Every child is run through existing bounded owned-command support. The JSON
   receipt contains fixed observations and hashes, not raw child streams, and is
   written only after isolated roots close successfully. Stdout remains the
   machine channel; ordinary informational stderr is permitted.
6. One post-publication `windows-2025` job depends on plan, bump, and publish.
   Documentation additionally depends on this job. A separate workflow or new
   dispatch input was rejected because it would weaken exact-run recovery.

## Risks / Trade-offs

- [A release may publish before a verifier defect is observed] → publication
  stays immutable; documentation is blocked and the same job is rerunnable.
- [GitHub metadata or downloads can stall or grow] → use HTTPS-only canonical
  endpoints, fixed inventory checks, bounded bodies, timeouts, and owned cleanup.
- [An inherited environment can select credentials or configuration] → clear
  it and reconstruct only required Windows and isolated mise variables.
- [A future informational notice changes stderr] → parse stable JSON stdout and
  retain stderr only as bounded failure context.

## Operational surface

The verifier runs directly on the GitHub-hosted `windows-2025` runner after the
existing publish job. It binds no port and starts no container. Setup uses the
workflow's existing read token only while mise itself is installed; the verifier
step and its descendants receive no GitHub token, release credential, proxy, or
provider secret. The workflow pins mise 2026.9.4, installs the repository's
pinned Rust toolchain for the delivery helper, and selects the x86_64 Windows
archive for the exact planned version. All network bodies, process output, and
child lifetimes are bounded by the delivery helper.

## Integration contract

The external route is the public GitHub repository: release metadata and tag
objects from the GitHub HTTPS API, and immutable assets below
`github.com/replygirl/kuru/releases/download/vVERSION`. The expected version and
commit come from the existing plan and bump jobs, and the checkout uses that same
commit. The verifier requires one release with the exact tag, resolved commit,
and authoritative five-target archive plus checksum inventory. It reconciles
GitHub asset names and hashes, the checksum manifest, the selected ZIP member,
the mise-installed executable, and `packages/kuru-memory/support/dolt-assets.json`
by exact version, target, size, and SHA-256 values. Fixture metadata remains local
to unit and candidate-install tests and cannot replace this route.
