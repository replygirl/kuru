## Context

The preparer receives the final HTTP response before it accepts archive bytes.
Its current 120-second request timeout does not define an overall retry
budget. Extending acquisition must preserve that bound and stable staging/lock
ownership, including when a caller cancels during a delay.

## Decisions

Keep recovery in the existing byte downloader, using the same immutable GET.
After HTTP 500/502/503/504 without Retry-After, drop the unsuccessful response
and await delays of 250 ms then one second before the remaining attempts. Stop
after three attempts. One 120-second timeout covers headers, delays and body streaming;
it does not restart for another request. Keep the pinned reqwest transport's
existing protocol behavior; add no retry middleware or dependency.

Do not retry transport/body errors or accept bytes from an unsuccessful status.
Existing content-length, streamed-size, digest, privacy, identity and publication
checks remain unchanged. A Retry-After header is explicit server advice; return
the error without inventing a shorter delay or an HTTP-date parser here.

## Operational surface

This affects source/CI build-input preparation on the existing native targets.
Production remains outbound HTTPS with no listener, new credential or setting.
Tests use bounded loopback TCP servers and the existing private HTTP-client seam.
There is no runtime engine download or workflow retry. The current hosted run
finishes before a subsequent ordinary branch push starts another run.

## Integration contract

The memory manifest continues to own exact target, release URL, byte counts and
digests. Delivery acquires only those compressed bytes, before Cargo's separate
local verification. Error responses carrying Retry-After and permanent failures remain
errors. Fixtures count actual requests and compare URLs, output bytes, stable
lock identity and staging cleanup; no mock supplies a successful archive result.

## Risks / Trade-offs

Recovery adds at most two HTTP attempts and 1.25 seconds of delay within the same
overall budget. Persistent failures still fail the build; the policy does not
promise availability. A shorter injected private test budget may exercise the
deadline without waiting 120 seconds, while ordinary prepare uses the fixed budget.
Cancellation keeps all work inside the owning future and leaves no retry worker.
