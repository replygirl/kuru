## Context

SQLx `Pool::close` marks the pool closed, waits for checked-out connections, then gracefully closes idle MySQL connections one by one. MySQL close sends QUIT and shuts down the stream. A hosted Intel macOS run showed the fixed eight-second deadline ending with `closed=true,size=1,idle=1`: query teardown had already been observed, two pool connections drained, and the last idle close did not finish before Dolt was reaped. The current shutdown then returns that first timeout even though owner reap has changed the connection state.

## Decisions

Keep the existing graceful drain and deadline. On timeout, run the existing exact-owner `finish_owner` while retaining lifecycle authority, then retry `Pool::close` for the same already-closed pool handles under another explicit deadline. SQLx documents repeated `close` calls as safe. Both the main shutdown and installed-guard shutdown follow this order; branch-only retirement remains unchanged because it does not own an engine to reap.

The regression fixture uses real Dolt and holds one checked-out pool connection through the first deadline. A test-only one-shot fires only after the retained child has actually reaped; the fixture then releases the dead client connection and requires explicit close to complete the second drain before returning. The lifecycle lease protects the store through exact child reap, and may become available while a closed pool finishes releasing its old client sockets. It does not replace SQLx with a fake pool or infer reap from elapsed time.

## Risks / Trade-offs

- **Extra bounded shutdown latency on a genuinely stalled pool:** only the exceptional path adds a second deadline; it still fails if pool cleanup remains unproved.
- **Cancellation during shutdown:** the retained owner and startup guard continue their existing drop/reap behavior; no timeout authorizes releasing lifecycle authority before exact child reap or killing an unrelated process.
