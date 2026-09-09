## Context

Release run 34407995805 failed on Linux with `ETXTBSY` while spawning a native
connector fixture. `StdioFixture::new` writes cached executable bytes into a new
file in the multithreaded test process. A concurrent fork may retain that writable
handle until exec, even after the writing thread closes it. Linux refuses to
execute an inode with a writable handle. The observed error matches this race;
the exact scheduler interleaving is not recorded by the CI log.

## Decisions

Compile the peer into an owned temporary directory and wait for the compiler to
exit before publishing the executable. An `Arc` holds the artifact for each
fixture; a mutex-protected weak cache shares it without keeping it alive forever.
Place fixture directories below the artifact directory to ensure hard links stay
on the same filesystem. Each fixture hard-links the immutable executable and
retains its own plan, transcript and completion markers. Only the separate compiler
process opens the executable for writing, before any fixture can use it.

Keep production spawning behavior, protocol assertions and test concurrency
unchanged. No sleep or launch retry conceals the filesystem race. Verify actual
execution and isolation as well as inode sharing; the fixture locates its plan
from the invoked hard-link path through `current_exe`.

## Risks / Trade-offs

The supported Linux and macOS targets must both preserve the invoked hard-link
path. Their hosted tests will execute it. The weak cache can recompile after the
last fixture drops; this bounded cost avoids leaked static temporary artifacts.
An isolated cache in the lifetime regression prevents other tests from obscuring
final-owner cleanup.
