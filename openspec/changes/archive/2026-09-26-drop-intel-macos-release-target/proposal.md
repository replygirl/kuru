## Why

The `runner-images` change removed the Intel macOS (`x86_64-apple-darwin`)
build from CI and the Release workflow, and dropped the target from the
release catalog, the shell installer and the bundled Dolt manifest. It was a
`ci` record with `skip_specs: true`, so the durable capability specs still
required five release archives and the product now enforces four. The
installer also gained a user-visible refusal that no spec recorded. This
change brings the canonical specs into line with the shipped behavior and
marks the platform removal as a breaking release change.

## What Changes

- **BREAKING**: future releases publish one core archive and one paired
  shell-support archive for each target in the release catalog, which no
  longer includes `x86_64-apple-darwin`. Past releases keep their Intel assets.
- **BREAKING**: the shell bootstrap refuses Intel Macs, detected through
  `uname` or selected with `--target x86_64-apple-darwin`, before any download,
  naming v0.9.0 as the last release that supported them.
- `release-automation` and `repository-delivery` requirements that said "five"
  archives, inventories or targets now refer to the release catalog.
- `docs/install.md` and `apps/kuru-docs/guide/installation.md` state that
  Intel Macs are unsupported after v0.9.0 and point to the v0.9.0 tag's own
  bootstrap with `--version 0.9.0`.
- Record correction: the archived `2026-09-26-runner-images` proposal lists
  `docs/verification.md` among the edited docs. That dated record was left
  unchanged, as the PR describes; it is a historical verification record.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `release-automation`: candidate assembly and support-inventory verification
  follow the release catalog instead of a fixed count of five.
- `repository-delivery`: the Windows artifact requirement and its recovery
  scenario follow the release catalog, and the shell bootstrap refuses Intel
  Macs before downloading.

## Impact

- Already implemented in the same PR by `runner-images`:
  `packages/kuru-delivery/src/targets.rs`, `src/release.rs`,
  `support/install.sh`, `packages/kuru-memory/support/dolt-assets.json`,
  and their tests. This change adds only the spec deltas and the two docs
  paragraphs.
- The PR title and squash commit carry `feat(release)!:` so an `auto` Release
  selects a breaking bump and the notes flag the removal.
- Intel Mac users on v0.9.0 who run `kuru update` will not find an asset for
  their target in later releases; the docs name the v0.9.0 installation path.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
