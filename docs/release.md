# Releases

Kuru releases use one manually dispatched workflow with a version bump as its
only input. The workflow validates the source, creates a signed version commit
when necessary, builds all four native archives, generates Communiqué notes,
publishes a complete draft release, and deploys documentation from that exact commit.
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

Configure Pages to use GitHub Actions. Documentation builds and publishes in the
final `build-docs` and `deploy-docs` jobs of `.github/workflows/release.yml`.
The build job has `contents: read` and `pages: read`; only the deploy job receives
`pages: write` and `id-token: write`. There is no standalone Pages workflow.

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

1. Check prerequisites and run the full repository gate on the selected main
   revision, including meaningful tests, coverage, docs and cospec checks. This
   original dispatch SHA remains the base when all jobs are rerun.
2. Stamp the workspace and local lockfile entries, then create the signed API
   commit with an expected-head comparison. If an earlier attempt created that
   commit, recover it by verifying its parent, release message and complete Git
   tree against the expected stamp. An unchanged version reuses the checked
   commit when it remains in main's history. Later main changes are excluded.
3. Run the full gate again on that exact version commit. Build native archives
   on Linux x86_64/arm64 and macOS x86_64/arm64, verifying each binary's version.
4. Generate notes in a separate job with a read-only GitHub token, alongside final
   validation of the selected version commit. No release writes are available to
   that job. Its output is an artifact for publication, which still waits for
   validation and all four builds.
5. Verify that all four expected archives exist and match their checksums.
   Create or reuse the immutable annotated tag, stage the notes and all assets
   in a draft, and verify uploaded asset digests before publishing by release ID.
   A matching complete published release is verified and reused without replacing
   its notes or assets, even if rebuilding produced different bytes.
   GitHub selects the latest release by version and date; recovering an older
   draft does not force it to become latest.
6. Run `build-docs` after `bump` and `publish` succeed, checking out the exact
   released commit SHA. Build and validate the site, then publish its artifact
   through `deploy-docs`, the final release stage. These jobs are skipped if
   release publication fails.

The archives retain the `kuru-VERSION-TARGET.tar.gz` naming convention and are
published alongside `SHA256SUMS`. Users install through mise or the package-owned
[shell bootstrap](install.md#install-with-the-shell-bootstrap), which resolves
latest to an explicit version and verifies its checksum before replacement.
[Release directories](install.md#release-archives) also support HTTPS mirrors and
offline installation. The bootstrap checks logical tar members and bounds both
decompression and fixed-member extraction; the native Rust updater retains its
raw-header validation. Neither path runs the candidate to validate it.

Release jobs inherit `MISE_LOCKED=1`, including nested package tasks. This keeps
tool installation from extending lockfiles after source validation. The bump
and publish jobs explicitly install locked Rust/hk and disable automatic task-tool
installation. Native build jobs install only Rust and set `MISE_NO_HOOKS=1` to
omit mise's repository-setup postinstall hook; hk 1.58.1 has no Intel macOS asset,
and archive construction does not create Git commits. Git hooks and full
validation retain hk. These native subcommands do not need the notes toolchain.
Validation, notes and docs jobs retain their package setup, with frozen locks.
Version preparation checks for tracked setup changes before stamping, and
the commit guard permits only Cargo.toml and Cargo.lock changes. An unexpected
file is reported by name and must be fixed in source rather than reset or included
in the version commit.

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
