## Why

The native Windows fixture intermittently times out after entering its qualified
stock PowerShell hash command, while its existing traced `-Command` diagnostic
changes both the launch form and environment. A failure-only control must
separate those variables without weakening the authoritative Kuru assertion.

## What Changes

- Extend the Windows CLI fixture with at most three bounded, isolated
  post-failure controls: encoded/inherited-console, command/inherited-console,
  and encoded/private-console launches using the same source and product-shaped
  environment.
- Keep the original Kuru launch and all success assertions authoritative; the
  controls only preserve discriminating failure observations.

## Impact

- Changes `apps/kuru-tui/tests/windows_cli.rs` only; native Windows fixture
  time increases only after its existing failure path.
