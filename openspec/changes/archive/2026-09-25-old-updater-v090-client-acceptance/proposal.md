## Why

The archived `old-updater-staged-acceptance` change pins only the published v0.4.2 Windows client against the staged release candidate. v0.4.2 is five `kuru-delivery` releases behind the currently shipped v0.9.0, so that proof does not exercise the actual binary a real user upgrades from today.

## What Changes

- Parameterize `old_updater_accepts_staged_release` in `packages/kuru-delivery/tests/support/mise_acceptance.rs` over `{version, release base, pinned manifest digest, pinned archive digest}` and run it for both the published v0.4.2 and v0.9.0 Windows clients.
- Update `docs/release.md` to describe both pinned clients.
- Track, as an explicit deferred item rather than prose, that neither client proves shell-support continuity across an upgrade: both predate #88 (the five-file shell-support archive), since v0.9.0 was released before #88 shipped.

## Impact

Only `packages/kuru-delivery/tests/support/mise_acceptance.rs` and `docs/release.md` change. The staged Windows acceptance path (`apps/kuru-tui/tests/windows_mise.rs`, run only by the `verify-staged-windows` Release job) now authenticates and exercises two published old clients instead of one: one additional ~50 MB checksum-verified download and one additional real `kuru update` subprocess run per staged Release attempt. No ordinary PR/main CI job is affected.
