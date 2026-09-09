# Releases

Kuru releases use one manually dispatched workflow. Maintainers select a version
bump; the workflow validates the source, creates a signed version commit when
necessary, builds all four native archives, generates Communiqué notes, publishes
a complete draft release, and deploys documentation from that exact commit.
Pushes and tags do not start release publication or deploy documentation.

## One-time setup

Create a dedicated `kuru-release` GitHub App with repository **Contents: read and
write** and the automatically included **Metadata: read** permission. Disable
webhooks, request no organization or account permissions, and install the app
only on `replygirl/kuru`. Configure these repository Actions values:

| Name | Kind | Purpose |
| --- | --- | --- |
| `RELEASE_APP_ID` | Variable | Numeric ID of the dedicated app |
| `RELEASE_APP_PRIVATE_KEY` | Secret | App key used to mint a short-lived installation token |
| `ANTHROPIC_API_KEY_COMMUNIQUE` | Secret | Dedicated API key used only by release-notes generation |

The notes job exposes the scoped Anthropic secret to Communiqué as
`ANTHROPIC_API_KEY`. Do not create a generic repository secret with that name or
place credentials in configuration files.

The app needs a narrow exception to the rule requiring a pull request before a
release version commit can reach main. Keep the separate rules requiring signed
commits and linear history and prohibiting deletion and force-push in place,
without an app exception. GitHub's `createCommitOnBranch` API signs the commit;
`expectedHeadOid` rejects a race if main moved after the checked revision.
The app token also allows normal push CI to run for the version commit.

Configure Pages to use GitHub Actions. Documentation builds and publishes in the
final `build-docs` and `deploy-docs` jobs of `.github/workflows/release.yml`.
The build job has `contents: read` and `pages: read`; only the deploy job receives
`pages: write` and `id-token: write`. There is no standalone Pages workflow.
Deployment of public documentation does not change repository visibility.

Release notes run on Ubuntu with the delivery package's task-scoped Cocogitto
7.0.0 and Communiqué 1.3.5 pins. The latter has no Intel macOS release binary;
native archive build jobs use only the Rust packaging task, so all four Kuru
targets remain buildable. Full maintainer tests and notes generation run on
Linux or Apple Silicon macOS. App installation does not require these tools.

## Choose and release a version

After the change is reviewed, merged, and main's checks are green, open Actions →
Release → Run workflow, select `main`, and choose:

| Bump | Behavior |
| --- | --- |
| `auto` | Derive the version from conventional commits since the last release |
| `major` | Explicitly advance the major version |
| `minor` | Advance the minor version |
| `patch` | Advance the patch version |

The repository's `cog.toml` makes `feat` a minor bump and other recognized
conventional types a patch bump. Breaking changes advance the major version
once it is nonzero. Cocogitto deliberately keeps automatic bumps within `0.y.z`;
use `major` when intentionally declaring `1.0.0`.

A release with no prior version tag starts from `0.0.0`. The initial feature
history therefore selects `0.1.0`. If that already matches the manifest, no
artificial version commit is created. The workflow rejects version downgrades,
invalid version output, and automatic releases when HEAD is already tagged.

Version calculation can be reviewed from a clean checkout; Cocogitto rejects
untracked or uncommitted changes. Stamping then makes the local version change:

```sh
mise run release:version -- auto
mise run release:set-version -- 0.2.0
```

The second command changes the checkout's workspace version and only the local
workspace package entries in Cargo.lock. It does not create a commit or tag.
Both commands run native Rust tooling from `packages/kuru-delivery`; no Python
runtime is involved.

## Validation and publication order

1. Check prerequisites and run the full repository gate on the selected main
   revision, including meaningful tests, coverage, docs and cospec checks.
2. Stamp the workspace and local lockfile entries, then create the signed API
   commit with an expected-head comparison. An unchanged version reuses the
   checked commit after confirming main has not moved.
3. Run the full gate again on that exact version commit. Build native archives
   on Linux x86_64/arm64 and macOS x86_64/arm64, verifying each binary's version.
4. Generate notes in a separate job with a read-only GitHub token. No release
   writes are available to that job. Its output is an artifact for publication.
5. Verify that all four expected archives exist and match their checksums.
   Create or reuse the immutable annotated tag, stage the notes and all assets
   in a draft, and verify uploaded asset digests before publishing by release ID.
6. Run `build-docs` after `bump` and `publish` succeed, checking out the exact
   released commit SHA. Build and validate the site, then publish its artifact
   through `deploy-docs`, the final release stage. These jobs are skipped if
   release publication fails.

The archives retain the `kuru-VERSION-TARGET.tar.gz` naming convention and include
`SHA256SUMS`. [Authenticated installation](install.md#release-archives) continues
to work while the repository is private: download with `gh release download`,
then install from the local directory. A release command requires a version that
has actually been published.

## Notes model and configuration

`communique.toml` uses top-level `context` and `system_extra` plus `[defaults]`.
The pinned tool is Communiqué 1.3.5. The model is the current Haiku family,
`claude-haiku-4-5-20251001`, which produces response blocks supported by that tool.
Communiqué's current Anthropic parser cannot deserialize the thinking blocks
returned by default by Claude Sonnet 5 and Opus 5. A local integration fixture
exercises both the supported response and that failure shape. Update the model
when compatibility has been verified; do not silently switch to an incompatible
model merely because its name is newer.

The first release includes the root commit inventory as additional context.
This matters because Communiqué's automatic first-release log uses `ROOT..HEAD`,
which omits the root commit itself. Notes are generated against the selected
commit before the remote tag is created; the context supplies the target version.
Generated notes remain fallible prose, so inspect the release notes artifact when
reviewing a run. The prompt requires factual changes and prohibits invented test
results or deployment claims.

Upstream contracts: [Cocogitto versioning](https://docs.cocogitto.io/guide/bump.html),
[Communiqué configuration](https://github.com/jdx/communique/blob/v1.3.5/src/config.rs),
[Communiqué Anthropic parser](https://github.com/jdx/communique/blob/v1.3.5/src/providers/anthropic.rs),
[Claude Sonnet 5 response changes](https://platform.claude.com/docs/en/models/sonnet-5/whats-new-sonnet-5).

## Recover an interrupted run

A failed run never deletes or retargets a tag, force-pushes a commit, or replaces
a published release. It can leave a checked version commit, an immutable tag,
or a partial draft. Preserve the `bump` job's exact commit SHA and selected version.

Run Release again on `main`, supplying **both** `resume_version` and `resume_sha`.
The workflow requires the SHA to be part of main's history and its workspace
version to match. It reuses that identity instead of calculating another bump.
A tag pointing to another commit, an unrelated draft, or a draft asset with a
mismatched digest stops the run. Matching existing assets are retained, missing
assets are uploaded, and publication occurs only after the draft is complete.
A corrupted draft needs maintainer inspection; the workflow will not delete it.

If publication succeeded and only documentation failed, rerun `build-docs` and
`deploy-docs` on that existing Release run in the Actions UI. They reuse the
released commit SHA. Do not dispatch a separate docs workflow or cut another
release just to redeploy documentation. The initial site also waits for the first
authorized release.
The workflows themselves are implemented and tested without dispatching a live
release during development.
