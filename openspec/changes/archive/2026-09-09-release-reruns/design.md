## Context

GitHub reruns retain the original event SHA, while a successful API version commit moves main. Explicit resume fields moved that reconciliation burden to the maintainer. Cospec exposes only strategy and avoids a commit when version stamping is unchanged.

## Decisions

Keep planning anchored to the original checkout. Reuse its workspace version when that commit is already a release candidate; otherwise calculate with real Cocogitto from that history. Remove both manual resume inputs.

Stamp as usual, then reconcile remote main. If main advanced, a reusable bump must be the direct child of the original base with the expected release headline and identical full Git tree. Compute that tree with a temporary index so the checkout index is untouched. Later main changes are never included. An unchanged stamp can reuse the original commit when it remains an ancestor of main. All new writes retain expectedHeadOid.

Publication recognizes a complete published release only when its immutable tag, source marker and complete asset metadata agree. It downloads the bounded SHA256SUMS by asset ID, verifies that manifest's own digest and reconciles its four archive hashes against GitHub's asset digests. A signed HTTPS storage redirect is followed without forwarding the API token. It returns the existing URL without replacing notes or assets. Partial drafts continue requiring exact verified local digests. Native/notes artifact uploads replace artifacts from prior attempts of the same run; Pages uses an attempt-specific name shared through the successful build's output so deploy-only retries retain that artifact.

## Risks / Trade-offs

Do not adopt moving main as a new release base during a rerun. An unrelated first descendant, non-linear history, mismatched tag or corrupt draft remains a hard error. A published release stays immutable even if a rebuild differs; its existing verified publication is reused. Fresh dispatch targets current main, whereas retrying the original run preserves its original base. Hosted publication remains deferred until authorized.

Use GitHub's `make_latest: legacy` semantic-version/date selection when completing a draft. A late recovery must not unconditionally promote an older release over newer versions.

## Operational surface

Existing Ubuntu 24.04 planning/publication runners and four native build targets remain. Existing scoped app and Communiqué credentials remain. No services, ports, tools or dependencies are added. Pages remains the final pair of jobs inside release.yml.

## Integration contract

GitHub Actions reruns use the original GITHUB_SHA/GITHUB_REF. REST commit/compare responses provide SHA, parent SHA, message and tree SHA; compare is bounded to the first descendant and must report the original base as merge base. GraphQL createCommitOnBranch still uses expectedHeadOid. Release listing includes drafts; asset names/digests and the source marker bind publication to the immutable tag. Tests exercise these contracts through an isolated Axum server and real temporary Git repositories, including lost responses.
