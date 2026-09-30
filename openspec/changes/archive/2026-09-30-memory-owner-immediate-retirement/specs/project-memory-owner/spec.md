## MODIFIED Requirements

### Requirement: Attachment-bound idle cleanup

The owner SHALL retain the engine while any authenticated attachment remains or any accepted operation is unsettled. A newly started owner SHALL NOT retire for lack of attachments until the client that started it has attached, bounded by the startup budget measured from endpoint publication; it SHALL serve other clients meanwhile, and if its starter has not attached by then it SHALL retire once no attachment remains. After its starter has attached, once the last attachment has released and accepted work has settled, the owner SHALL begin shutdown immediately, with no idle interval. It SHALL stop accepting and retire its endpoint, then close the store and reap Dolt, and only then release its owner lock. Because no idle interval remains, a command that follows another pays a fresh owner start: measured on a macOS arm64 release build (10 samples; Ubuntu figure pending from CI), the median reopen right after a close was 1.207 s, a reopen after the predecessor had fully exited took 1.050 s median (the whole command: process start, owner spawn, existing-project open and demo turn), and the median close was 71 ms. A running writable client SHALL keep at least one authenticated attachment across a cancelled or failed request, without resending that request, except in three cases: the owner itself closes that client's only connection; the owner refuses the replacement connection and then ends the abandoned connection at its operation timeout before the client's next request; or the cancelled request is dropped outside an async runtime, so no replacement is opened. In those cases the owner MAY retire, and the client's next request SHALL fail with a truthful connect error and SHALL NOT be retried to hide it.

#### Scenario: Last client disconnects
- **WHEN** the last attached client disconnects after the starter has attached and no accepted operation remains
- **THEN** the owner begins shutdown without waiting, retires its endpoint, and releases its owner lock only after owned Dolt cleanup

#### Scenario: Owner starts before its starter attaches
- **WHEN** a new owner has published its endpoint and another client attaches and detaches before the starter
- **THEN** the owner keeps serving until its starter attaches within the startup budget, and the starter attaches to that same generation

#### Scenario: Client work outlasts other clients
- **WHEN** one attachment still holds the dream lease or has an accepted operation in flight and every other attachment has released
- **THEN** the owner keeps its engine until that attachment releases and the operation settles

#### Scenario: Cancelled call on a running client
- **WHEN** a running writable client's request is cancelled and it was that client's only connection
- **THEN** the owner remains for that client and its next request succeeds on the same generation
