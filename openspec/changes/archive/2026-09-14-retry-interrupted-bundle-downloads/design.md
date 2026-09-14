## Context

The build-input downloader permits three attempts for selected temporary HTTP
statuses, but send and body-stream errors currently escape that loop. Final-head
CI then failed before Ubuntu tests when the pinned archive body timed out. The
log proves a body timeout, but does not establish whether the connection was
idle throughout, so the correction must cover bounded transport interruption
without assuming a more specific upstream cause.

## Goals / Non-Goals

**Goals:**

- Recover a pinned, idempotent archive GET from transient connect, send-timeout,
  and body-chunk network errors.
- Keep all attempts, delays, streaming, and verification within the existing
  120-second total download budget and three-GET maximum.
- Ensure no bytes or digest state from a failed attempt can enter the next one.

**Non-Goals:**

- Retrying permanent HTTP responses, `Retry-After`, integrity failures, or local
  file errors.
- Changing runtime downloads, manifests, dependencies, workflow scheduling,
  publication, locks, or safety limits.

## Decisions

The ordinary HTTPS client gets a 15-second connect timeout and a 30-second
read-idle timeout. The existing outer 120-second timeout remains authoritative,
so steady progress cannot extend the total acquisition budget and retries do
not reset it.

Keep one attempt loop around send, status acceptance, and body streaming. A send
error is retryable only when reqwest classifies it as a timeout or connect
failure. Any network error returned while reading the accepted response's next
chunk is retryable because no completed archive has been accepted. Before each
retry, truncate and rewind the existing private staging file and construct a
fresh SHA-256 state. Use the existing 250-millisecond and one-second delays and
never exceed three identical GETs.

Size declarations, streamed size overflow, final byte-count mismatch, digest
mismatch, local seek/truncate/write failure, permanent status, and every status
carrying `Retry-After` remain terminal. Publication still occurs only after one
complete attempt passes all existing checks.

## Integration contract

Every attempt uses the exact manifest-owned URL and remains a credential-free
HTTPS GET. Bounded loopback fixtures exercise a body interruption followed by a
valid response, persistent retryable failure, byte/digest reset, and terminal
integrity and status errors. They count actual requests and inspect the final
private staged output rather than substituting a mock result.

## Risks / Trade-offs

A very slow server can now consume multiple attempts, but never more than the
unchanged 120-second total. Retrying a chunk error repeats transferred bytes;
the immutable GET, exact size, SHA-256 verification, reset staging, and maximum
three attempts bound that cost and prevent mixed output.
