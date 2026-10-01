# Proposal

## Why

The Windows source-entrypoint probe tests intermittently hit their 180 s bound
with a single parked `powershell.exe`, ~20 s of CPU and no output; the #137
diagnostics show the process but not which script statement it was resolving or
executing, so the product-side versus test-side question cannot be answered.

## What Changes

- `apps/kuru-tui/tests/windows_cli.rs`: the source-entrypoint probe starts with a
  shared trace prelude. Engine `PreCommandLookupAction`/`PostCommandLookupAction`
  handlers and inline bracketing marks append one line each (UTC time, event,
  command name, milliseconds since process start, process CPU milliseconds) to
  `<tempdir>/probe-trace.log` using only .NET calls, never a cmdlet or function,
  inside `try {} catch {}` so tracing cannot add a command lookup or change the
  outcome. The probe's statements, environment and assertions are otherwise unchanged.
- The two probe tests launch through `launch_traced`: on a launch or timeout
  error it appends, after the existing #137 diagnostics, a one-line
  classification (parked in command lookup of X, executing X after lookup, last
  mark, or no trace file) and the bounded tail of the trace (at most 64 KiB read,
  last 40 lines). Success prints nothing new. The scopes test's assertion
  diagnostic also carries the trace.
- A new Windows-only test runs a scratch probe with the identical prelude that
  parks in a script function reading an anonymous pipe it also holds the writer
  for, under a test-only 30 s bound, and asserts the timeout error names that
  function and classifies it as executing after lookup.

## Impact

Test-only; no shipped script, `kuru-delivery` command timeout arm, deadline,
retry or sleep changes. Windows native test time grows by the scratch probe's
30 s bound plus PowerShell start on each run; per-line trace cost is one small
file append per command lookup (about ten to fifteen per probe launch), measured
only on CI.
