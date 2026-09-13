## Why

The Windows workspace coverage run exposed native tests whose private wrapper
deadlines, environment setup, or observations no longer match the production
contract they exercise. Those fixtures mask a real shell timeout as a JSON parse
error, terminate a stock PowerShell check before Kuru's own deadline, rely on
environment variables that the minimized shell deliberately excludes, and ask
the non-opening `config` command to report preferences stored in memory. A later
macOS hook run also showed that the Unix terminal fixture gives the child PTY
stdio without making that PTY its controlling terminal, so Crossterm can read
dimensions from the unrelated outer hook terminal and never produce the expected
frame at the requested test size.

The native evidence must distinguish a production regression from a fixture
budget or observation failure. Correcting only these tests preserves the shipped
deadlines, reduced shell authority, real loopback refusal, native process cleanup,
and observable preference persistence.

A subsequent native run reached the real shell but compared lexically preserved
`PATH`, `HOME` and `TEMP` values with paths reconstructed from PowerShell's
canonicalized working directory. Windows can spell the same directory with a
short or long component, so that observation can reject the required lexical
preservation without identifying which condition failed.

The next exact-head Windows run timed out inside the real shell operation with
the PowerShell root still running and zero bytes or EOF on both captured streams.
Because the script emitted output only after every assertion, that evidence does
not distinguish startup from a blocked file write, native command, batch probe,
module autoload or final check.

Fixed stage labels in the following run localized a separate CLI shell timeout
to first-use `Get-FileHash` discovery. That fixture launched a negative stock
PowerShell control and the authoritative Kuru child with the same `APPDATA` and
`LOCALAPPDATA`, allowing the control's deliberately incompatible module manifest
to affect PowerShell's shared module-analysis cache before Kuru ran. The same run
also showed that stock PowerShell appends `.CPL` to an inherited `PATHEXT` during
engine construction, after Kuru has already supplied the exact projected value.

## What Changes

- Give real stock PowerShell and refused-connect fixtures bounded observation
  windows that encompass the production operation being observed.
- Keep hostile and secret environment inputs excluded while supplying the stock
  Windows compatibility variables needed by the outer native shell fixture.
- Embed controlled fixture paths and values directly in authorized PowerShell
  source instead of relying on custom environment variables that Kuru must drop.
- Compare projected Windows shell values with the exact controlled values supplied
  by the fixture, and name failed conditions in the bounded tool receipt.
- Append fixed stage labels to a private fixture file around each shell probe and
  report a bounded label prefix when the real shell operation itself fails.
- Give the deliberately failing stock-PowerShell module control separate private
  cache directories from Kuru's authoritative shell invocation.
- Keep the exact pre-launch `PATHEXT` projection assertion and account for stock
  PowerShell's own `.CPL` addition only in the inner-process expectation.
- Use the checked movable approval-record operation boundary when the trust
  corruption fixture replaces synthetic records under a retained pinned store.
- Make native replay failures name the raw failed receipt and verify saved picker
  choices through the memory-backed behavior that owns them.
- Launch Unix terminal fixtures through the existing safe portable PTY boundary
  so the requested test PTY owns dimensions, input and output even when the test
  runner itself has a differently sized controlling terminal.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only native and cross-platform test code changes: connector authentication and
shell tests, runtime Windows tool replay, Windows CLI/TUI integration tests, and
Unix terminal test support. The already pinned portable PTY development
dependency becomes available to Unix tests; no production API, configuration,
timeout, allowlist, dependency version, or migration changes. PowerShell cache
isolation and post-startup `PATHEXT` expectations remain fixture-only.

## Surfaces

- [x] interactive — native TUI picker persistence and CLI shell behavior
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — real tool receipts and minimized shell authority
