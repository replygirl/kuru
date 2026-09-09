## Context

Kuru has four native release targets, exact dependency pins, a full validation gate, and checksum-verified installers. Main requires signed commits and pull requests. The user is provisioning a dedicated app with Contents write permission and a narrow exception to the PR rule; the signature and force-push/deletion protections remain enforced.

## Decisions

Use Cocogitto only for dry-run version calculation. Native Rust package helpers validate strict versions, stamp only local package versions, create bounded GitHub API payloads, verify tag identity, and validate the exact expected asset set. Commit via createCommitOnBranch with expectedHeadOid. Build and check the returned commit SHA rather than a moving branch. Treat an unchanged first-release version as a valid no-op commit.

Separate notes generation under read permissions from publication. Use Communiqué 1.3.5's actual top-level context/system_extra and defaults schema. Include root-commit inventory for the first release because its automatic ROOT..TAG range excludes that commit. Tests drive the real tool against local transport fixtures where feasible; missing live credentials are recorded honestly.

Create the annotated tag only after validation, artifacts and notes succeed. An existing tag is reusable only at the exact candidate SHA. Create a complete draft and publish it by numeric release ID. Never delete or retarget tags, overwrite published release assets, bypass signature rules, or force-push. Explicit retry uses the existing tag/version, so it does not calculate another bump.

## Risks / Trade-offs

GitHub and the model API remain external dependencies. Failures before publication preserve completed version commits and any immutable tag/draft for an explicit retry. Concurrent main changes fail the compare-and-swap. Generated notes can be imperfect; instructions require factual commit-backed changes and the draft body remains reviewable. Tests prove orchestration and failure behavior without cutting a real release.

## Operational surface

The workflow runs on the existing macOS/Linux native runners and uses RELEASE_APP_ID, RELEASE_APP_PRIVATE_KEY and the scoped ANTHROPIC_API_KEY_COMMUNIQUE secret. Only notes generation exposes that secret as ANTHROPIC_API_KEY. Only the app may bypass the PR/check requirement; the separate integrity rules have no bypass. The reusable Pages workflow receives the exact release SHA. Private release downloads continue through authenticated gh download followed by the local checksum installer.

## Integration contract

GitHub GraphQL createCommitOnBranch accepts the repository, branch and full expectedHeadOid, returns commit.oid, and signs the resulting commit. REST tag references are peeled to commits before comparison. Release IDs are numeric and drafts are addressed by ID. Action inputs reach the native Rust Clap CLI through environment variables and quoted argv values, never interpolated shell code. The delivery package owns release tool pins and mise tasks. Communiqué 1.3.5 uses Anthropic Messages text/tool blocks; the current Haiku family is selected because newer thinking-on-by-default responses are unsupported by that parser. Local HTTP fixtures exercise the real binary with fake credentials only.
