# Releases

Kuru releases use one manually dispatched workflow with a version bump as its
only input. The workflow validates the source, creates a signed version commit
when necessary, builds all five native archives, generates Communiqué notes,
assembles and tests the complete candidate, deploys documentation from that exact
commit, and publishes the release only after those gates succeed. A separate
post-publication Windows job then verifies the immutable public download.
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

The notes step exposes the scoped Anthropic secret to Communiqué as
`OPENAI_API_KEY` for its OpenAI-compatible wire adapter, which calls Anthropic.
Keep the repository secret named `ANTHROPIC_API_KEY_COMMUNIQUE`; no generic
`OPENAI_API_KEY` or `ANTHROPIC_API_KEY` repository secret is needed. Do not place
credentials in configuration files.

The app needs a narrow exception to the rule requiring a pull request before a
release version commit can reach main. Keep the separate rules requiring signed
commits and linear history and prohibiting deletion and force-push in place,
without an app exception. GitHub's `createCommitOnBranch` API signs the commit;
`expectedHeadOid` rejects a race if main moved after the checked revision.
The app token also allows normal push CI to run for the version commit.

Configure Pages to use GitHub Actions. Documentation builds and publishes through
the `build-docs` and `deploy-docs` jobs of `.github/workflows/release.yml` before
the public-release job.
The build job has `contents: read` and `pages: read`; only the deploy job receives
`pages: write` and `id-token: write`. There is no standalone Pages workflow.

Release notes run on Ubuntu with the delivery package's task-scoped Cocogitto
7.0.0 and Communiqué 1.3.5 pins. The latter has no Intel macOS release binary;
native archive build jobs use only the Rust packaging task, so all five Kuru
targets remain buildable. Full maintainer tests and notes generation run on
Linux, Apple Silicon macOS or Windows x86_64. App installation does not require these tools.

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
invalid version output and conflicting release identities.

If main is already at its tagged release commit or prepared version commit,
dispatch reuses that version regardless of the selected strategy. New commits
after that point cause the chosen strategy to calculate a new version. Recover
an interrupted release by rerunning its existing workflow, especially when main
has since advanced; a fresh dispatch starts from the current main revision.

The dispatch plan can be reviewed from a clean checkout. Stamping then makes the
local version change:

```sh
mise run release:tool -- plan --bump auto
mise run release:set-version -- 0.2.0
```

The second command changes the checkout's workspace version and only the local
workspace package entries in Cargo.lock. It does not create a commit or tag.
Both commands run native Rust tooling from `packages/kuru-delivery`; no Python
runtime is involved.

## Validation and publication order

1. Run the reusable quality jobs and native coverage workflow concurrently on
   the selected main revision. Format, lint, typecheck, tooling, docs and cospec
   have independent jobs; coverage executes the behavioral suite once. Both
   workflows must pass before planning and checking publication prerequisites.
   This original dispatch SHA remains the base when all jobs are rerun.
2. Stamp the workspace and local lockfile entries, then create the signed API
   commit with an expected-head comparison. If an earlier attempt created that
   commit, recover it by verifying its parent, release message and complete Git
   tree against the expected stamp. An unchanged version reuses the checked
   commit when it remains in main's history. Later main changes are excluded.
3. Run the same independent validation jobs on that exact version commit. Both
   quality and coverage must pass before building native archives
   on Linux x86_64/arm64, macOS x86_64/arm64 and Windows x86_64 MSVC, verifying
   each binary's version and bundled offline engine. The Windows build uses a
   static CRT and validates its PE imports against the allowed system DLLs.
4. Generate notes in a separate job with a read-only GitHub token, alongside final
   validation of the selected version commit. No release writes are available to
   that job. Its output is an artifact for publication, which still waits for
   validation and all five builds.
5. Assemble one attempt-scoped candidate artifact without a release-write token.
   The delivery tool requires exactly five archives and their checksum sidecars,
   generates `SHA256SUMS`, validates the bounded notes, and retains `dist/` plus
   `RELEASE_NOTES.md` for the remaining jobs. Invalid or incomplete inputs stop
   here without creating a tag, draft, or public release.
6. On native Windows, run the ordinary mise installation route against simulated
   GitHub metadata that serves the exact staged Windows ZIP. In parallel,
   `build-docs` checks out the selected commit and builds and validates the site.
   The Windows check verifies candidate checksums and bytes, installation,
   activation, bundled Dolt, an offline conversation and durable reopen. It
   then repeats
   [previous-release update acceptance](#previous-release-update-acceptance)
   against the exact staged candidate, requested at its real version, as a
   release-time sanity re-run of the check ordinary CI runs on every supported
   platform. The loopback mise fixture and the local release base given to the
   previous updater are not public downloads of the new release. A failure in
   either path blocks `publish`.
7. Run `deploy-docs` only after both staged Windows acceptance and `build-docs`
   succeed. Pages deployment and GitHub release promotion are separate service
   operations; this ordering does not claim they update atomically.
8. Run `publish` after the acceptance gates. It consumes the same candidate, rechecks
   the five archives and existing `SHA256SUMS`, creates or reuses the immutable
   annotated tag, stages notes and release assets in a draft, verifies uploaded
   digests, and only then promotes the release. A matching complete published
   release is verified and reused without replacing notes or assets after a lost
   response. GitHub selects the latest release by version and date; recovering
   an older draft does not force it to become latest.

The four Unix archives retain `kuru-VERSION-TARGET.tar.gz`; Windows uses
`kuru-VERSION-x86_64-pc-windows-msvc.zip` with exactly `kuru.exe`, `LICENSE` and
`README.md`. All five are published alongside `SHA256SUMS`. Users install through
mise or the package-owned
[shell](install.md#install-with-the-shell-bootstrap) and
[PowerShell](install.md#install-with-powershell) bootstraps, which resolve
latest to an explicit version and verify its checksum before replacement.
[Release directories](install.md#release-archives) also support HTTPS mirrors and
offline installation. The shell bootstrap checks logical tar members and bounds
decompression and fixed-member extraction; the native Rust updater retains its
raw-header validation. Windows ZIP paths require exactly three regular members.
No installer runs the candidate to validate it.

Release jobs inherit `MISE_LOCKED=1`, including nested package tasks. This keeps
tool installation from extending lockfiles after source validation. The bump
and publish jobs explicitly install locked Rust/hk and disable automatic task-tool
installation. Native build jobs install only Rust and set `MISE_NO_HOOKS=1` to
omit mise's repository-setup postinstall hook; hk 1.58.1 has no Intel macOS asset,
and archive construction does not create Git commits. Local Git hooks and the
release bump, publish, notes and docs jobs retain hk. Native archive subcommands
do not need the notes toolchain. Reusable quality and native-test workflows skip
hook setup and install only their scoped tools, with frozen locks.
Version preparation checks for tracked setup changes before stamping, and
the commit guard permits only Cargo.toml and Cargo.lock changes. An unexpected
file is reported by name and must be fixed in source rather than reset or included
in the version commit.

## Verify the staged Windows candidate

Before publication, the Release workflow downloads its complete candidate on
`windows-2025` and invokes `//apps/kuru-tui:verify:staged-windows` with the exact
staged Windows ZIP. The task resolves the pinned native mise executable before
clearing its child environment, then routes the ordinary
`github:replygirl/kuru@VERSION` backend through isolated loopback release metadata
in fresh user, project, configuration, cache, data, state and temporary roots.

The fixture serves the candidate's real ZIP bytes and checksum through the
simulated metadata, verifies what mise installs, and then runs and resumes an
offline demo conversation from empty Kuru and engine caches. It checks memory
status, history and sessions and compares the extracted Dolt executable and
licenses with `packages/kuru-memory/support/dolt-assets.json` at the selected
commit. The fixture, mise and Kuru children receive no token, provider
credential, proxy or public endpoint. Informational application stderr is
allowed; machine results remain JSON on stdout. This proves the exact staged
package through the native mise path, not availability or download behavior
from the public GitHub release.

The task then runs
[previous-release update acceptance](#previous-release-update-acceptance)
against the staged ZIP and its sidecar. Only its release resolver contacts
public GitHub, as described there; the workflow step currently passes no
`GITHUB_TOKEN`, so that listing request is anonymous.

## Previous-release update acceptance

Updating Kuru must always be possible and must succeed; migrations exist so that
it does. As a floor under that policy, not a replacement for it, the previous
published release's own updater must install every candidate. Ordinary PR and
main CI runs `//packages/kuru-delivery:test:previous-release-update` natively on
every supported platform. The task requires `KURU_UPDATE_CANDIDATE_BINARY`, the
absolute path of the release-profile `kuru` built from the tree under test, and
outbound HTTPS to GitHub. It has no bundle-preparation dependency, because the
binary already embeds its engine.

The test packages the binary, and the five shell-support files it generates in
an isolated environment, as a local release directory for the host target.
Main's workspace version equals its latest published tag, so the candidate is
packaged under the next patch version. The previous updater is asked for
`X.Y.(Z+1)` while the installed binary still reports its built version `X.Y.Z`.
The updater binds the archive name and checksums, not the reported version, so
both versions are checked separately and the exact installed bytes are the
discriminating check. A branch whose workspace version is older than the latest
published tag fails release selection and must be rebased.

The previous release is resolved at run time from the public GitHub releases
list: the greatest stable `vX.Y.Z` release other than the candidate, failing if
none is older or one is newer. The resolver downloads that release's own
`SHA256SUMS`, the host target's archive (`.tar.gz` on Linux and macOS, `.zip` on
Windows) and, when the release publishes one, that target's shell-support
envelope. It verifies each file against that manifest and GitHub's asset digests
before running anything. An optional `GITHUB_TOKEN` is sent as a bearer token
only on the single release-listing request to `api.github.com`, which avoids the
anonymous API rate limit on shared runners. Asset downloads are anonymous, and no
Kuru or mise child process receives the token.

The previous executable is installed into fresh isolated user, configuration,
cache, data, state and temporary roots. A support-aware previous release also
gets its own versioned support tree, and on Linux and macOS its stable man page,
as the installers lay them out. That release's own `kuru update` then runs
against the candidate directory. The check requires the exact replacement bytes
and the built version. An executable-only previous updater must leave no managed
support. A support-aware one must install the candidate's exact versioned
support snapshot, leave the previous tree unchanged and, on Linux and macOS,
publish the candidate's stable man page. The upgraded binary regenerates the
five support files, which must match the candidate's. Nothing is pinned, so each
change is checked against the release users actually have. The Release workflow
repeats the check on Windows against the exact staged candidate before
publication.

`deploy-docs` depends on this native check and the independent docs build. The
`publish` job depends on successful deployment, so a
candidate, Windows, docs-build or docs-deployment failure leaves no public Kuru
release. If Pages deploys and final publication then fails, rerun the failed
publication work in the same Release run; the workflow does not claim that Pages
and GitHub Releases commit atomically.

After `publish` succeeds, `verify-published-windows` runs on Windows from the
same selected release commit. It invokes the delivery package's existing
`verify:published-windows` task with the exact version, expected commit and run
URL. It resolves the public tag and asset inventory, verifies checksums, installs
through the unmodified public mise route, and exercises a cold offline
conversation and durable reopen with the bundled engine. Its cleanup-confirmed
receipt is retained as the `published-windows-<version>-<attempt>` Actions artifact.

This job has read-only repository permissions and no publication credentials.
A failed download or runtime check makes the release run fail visibly, while the
already published release remains unchanged. Publication and public-download
verification are separate results: staged acceptance cannot prove public
availability, and a later availability failure does not undo publication.
Rerun the failed job on the same Release run to retain its original version and
commit; do not dispatch another release or replace immutable assets to repeat
the check. The package task also remains available for manual diagnostics.

## Notes model and configuration

`communique.toml` uses top-level `context` and `system_extra` plus `[defaults]`.
The pinned tool is Communiqué 1.3.5. It uses `claude-sonnet-5` through
Anthropic's official OpenAI-compatible endpoint. `provider = "openai"` selects
the wire format; requests go directly to `https://api.anthropic.com/v1`, and the
model and credentials remain Anthropic's. Only the notes step maps the existing
`ANTHROPIC_API_KEY_COMMUNIQUE` secret to the adapter's `OPENAI_API_KEY` environment
variable. No additional secret is needed.

Communiqué's native Anthropic parser cannot yet deserialize Claude 5 thinking
blocks. The compatibility response omits those blocks while retaining tool calls.
Return to the native adapter when upstream supports the response shape and the
integration checks pass. This compatibility route cannot configure an
`anthropic-workspace-id` header; use a key scoped to the intended workspace.

Generation includes current product documentation from the exact selected commit.
That snapshot determines current behavior; historical commits may describe
earlier designs. Generation requires a clean checkout, including no nonignored
untracked files, because repository search can read those too. Ignored build
outputs may remain. The first release also includes the root commit inventory.
This matters because Communiqué's automatic first-release log uses `ROOT..HEAD`,
which omits the root commit itself. Notes are generated against the selected
commit before the remote tag is created; the context supplies the target version.
The native wrapper preserves complete generated Markdown in the notes artifact
after checking that it is a nonempty regular UTF-8 file of at most 100 KB.
Word and bullet targets guide the writing; exceeding them does not discard the
draft or fail publication. Real-tool fixtures exercise compatible API requests,
tool-result replay, failure cleanup and complete output preservation. These
checks establish the delivery contract; generated prose remains fallible. Inspect actual notes against
the selected source, including defaults, provider identities and configuration
persistence. A successful notes job is not evidence that every claim is accurate.

Upstream contracts: [Cocogitto versioning](https://docs.cocogitto.io/guide/bump.html),
[Communiqué configuration](https://github.com/jdx/communique/blob/v1.3.5/src/config.rs),
[Communiqué OpenAI adapter](https://github.com/jdx/communique/blob/v1.3.5/src/providers/openai.rs),
[Anthropic API compatibility](https://platform.claude.com/docs/en/cli-sdks-libraries/libraries/openai-sdk),
[Claude Sonnet 5 response changes](https://platform.claude.com/docs/en/models/sonnet-5/whats-new-sonnet-5).

## Recover an interrupted run

A failed run never deletes or retargets a tag, force-pushes a commit, or replaces
a published release. It can leave a checked version commit, an immutable tag,
or a partial draft. Recovery does not require copying a version or commit SHA.

In the existing Release run, prefer **Re-run failed jobs**. **Re-run all jobs** is
also supported: [GitHub preserves the original dispatch SHA](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs),
the version commit is recovered even if its API response was lost, and repeated build/notes uploads
replace only that run's temporary workflow artifacts. Later commits on main do
not enter the release. A completed publication returns its existing URL after
verifying the tag, source marker and published checksums; it remains unchanged.

A tag pointing to another commit, an unrelated first commit after the original
base, an unrelated draft, or a draft asset with a mismatched digest stops the run.
Matching draft assets are retained, missing assets are uploaded, and publication
occurs only after the draft is complete. A corrupted draft needs maintainer
inspection; the workflow will not delete it. For a partial draft, rerunning only
the failed jobs also retains the original build artifacts rather than rebuilding.

If publication succeeded and only documentation failed, rerun `build-docs` and
`deploy-docs` on that existing Release run in the Actions UI. They reuse the
released commit SHA. Do not dispatch a separate docs workflow or cut another
release just to redeploy documentation. The initial site also waits for the first
authorized release.
Each docs build uses an artifact name for that run attempt. The deploy job uses
the successful build's recorded name, so rerunning only deployment uses the same
artifact. If that artifact has expired, rerun the docs build and deployment jobs.
The workflows themselves are implemented and tested without dispatching a live
release during development.
