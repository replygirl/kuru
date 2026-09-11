## Why

CI 34572200638 failed before Ubuntu tests because GitHub returned HTTP 500 for
the pinned Windows Dolt archive. Build-input preparation makes one request, so
a transient upstream response aborts an otherwise reproducible build even when
the same immutable asset is immediately available afterward.

## What Changes

- Permit at most three attempts for the same immutable GET after HTTP 500,
  502, 503 or 504, with short bounded delays and one shared 120-second deadline.
- Preserve explicit server Retry-After instructions by returning their error
  without an automatic retry; do not guess or shorten a server's delay.
- Keep permanent HTTP, transport, body, size and digest errors fatal. Retries
  happen before accepting any archive bytes and never change the selected URL,
  version, digest, private staging or publication policy.
- Verify actual HTTP response sequences, bounded failure and cancellation with
  isolated local servers, including a regression observed failing before repair.

## Capabilities

### Modified Capabilities

None. The existing immutable, verified and bounded build-input contract stands.

## Impact

Only the delivery package's build-input downloader and its behavioral tests,
plus the owning development guide, change. No dependency, tool pin, runtime
download path, public configuration, CI scheduling or release graph is added.

## Surfaces

- [x] interactive — build-preparation failure behavior
- [ ] deploy — no execution topology change
- [x] integration — immutable HTTP GET responses
- [ ] agent-behavior — no model or tool-authority change
