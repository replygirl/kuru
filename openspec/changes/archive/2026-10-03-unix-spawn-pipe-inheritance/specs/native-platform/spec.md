# Spec Delta

## MODIFIED Requirements

### Requirement: Complete one-shot Unix child pipes

The retained Unix fresh-process-group owner SHALL accept the caller's declared
policy for standard input, output, and error (pipe, null, or inherit) and SHALL
itself create every declared pipe; it MUST NOT reconstruct stdio intent through
`Command` introspection, and std MUST NOT create a stdio pipe for an owned
spawn. The owner SHALL permit a safe caller to take each declared pipe exactly
once without exposing the standard child or its numeric process identity.
Taking pipes MUST NOT transfer root observation, signal, reap, or post-reap
authority away from that owner. The child's ends MUST be closed in the parent
before spawn returns.

#### Scenario: Session takes all configured pipes

- **WHEN** a connector creates a retained Unix child with piped stdin, stdout, and stderr
- **THEN** it can take each pipe once for asynchronous framing and draining while the original owner remains solely responsible for ordered process-group cleanup

#### Scenario: Undeclared pipe is not available

- **WHEN** a caller declares a stream null or inherited and later takes that stream
- **THEN** the owner reports that the stream was not piped and its cleanup authority is unchanged

## ADDED Requirements

### Requirement: Concurrent owned Unix spawns keep pipes private

The fresh-process-group owner SHALL create its stdio pipes close-on-exec and
start the child under one process-wide platform spawn lock, and the bounded
process snapshot SHALL start its child under the same lock, so its std pipes
are created under the lock; no other platform spawn can then copy a
descriptor table that holds a pipe end without close-on-exec. Where the OS
creates pipes close-on-exec atomically the owner MUST use that call. The lock
MUST be private to the platform and, in product builds, held only across pipe
creation and child creation, never across caller code, waiting, or I/O on the
child; only test builds may run their own seams under it.
The platform MUST NOT claim isolation from spawns or descriptor creation that
bypass it: an unrelated legacy spawn can still inherit an owned pipe end in its
own window, and an owned child can still inherit descriptors that other code
creates without atomic close-on-exec. On std's fork path a legacy spawn that
inherits an owned spawn's exec-error pipe stalls that spawn, and with it every
later platform spawn, until the legacy child exits.

#### Scenario: Concurrent owned children reach end of file independently

- **WHEN** two owned children that read standard input to end of file start concurrently on separate threads and each parent drops its input end at once
- **THEN** each child exits on its own end of file within the existing bound while the other child is still running

#### Scenario: Forced window between pipe creation and close-on-exec

- **WHEN** one owned spawn is paused after creating a pipe and before marking it close-on-exec, and a second owned spawn starts on another thread
- **THEN** the second spawn waits for the platform spawn lock, the first child reaches end of file once its parent drops its input end, and the second child holds none of the first child's pipe ends
