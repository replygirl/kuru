# Spec Delta

## MODIFIED Requirements

### Requirement: Owner open timeline

Only when the environment variable `KURU_OPEN_TIMELINE` is exactly `1` in the memory service owner process, the owner SHALL record named open milestones as nanosecond offsets from one monotonic anchor per process, in a bounded in-memory log sealed at endpoint publication. It SHALL write that log once, after it has released its owner lock in close, as a non-durable owner-private file named for its service generation beside its endpoint, created through the checked private-file handle so that it never follows a link, truncates or replaces an existing file. A gated owner started with a starter token SHALL also stream each stamp, starting with the stamps already logged, to a non-durable owner-private file named from that token's activity tag beside its endpoint, created through the same checked create-only handle and written one unsynced line per stamp; when that name is already taken it SHALL leave streaming off, and it SHALL NOT append to or remove a stream file it did not create. It SHALL remove its own stream file while it holds its owner lock, after endpoint publication and before it serves, or on a failed open, and never in close. The close-time file SHALL carry only event names, offsets, one wall-clock anchor, the service generation, the binary version and counts; each stream line SHALL carry only an event name, its offset and its wall-clock time; neither SHALL carry a path, scope, SQL text, store identity, connection secret, credential or content. Recording, streaming or writing MUST NOT fail, cancel or reorder the open or serve, MUST NOT change the close's result or the order of its steps, and MUST NOT lengthen any successor's owner-lock wait. A gated open adds at most one unsynced write of at most 62 bytes per stamp plus one removal before it serves, with no sync, wait or retry; after the lock release a gated close adds exactly one create-only, unsynced write of at most 8 KiB before it returns; and a failed open leaves no timeline file it created, except a stream file whose removal a concurrent reader made uncertain on Windows, which is not retried. A starter whose own environment has the variable exactly `1` MAY read only the stream file named from the token it passed to its owner, once, at its readiness deadline, without waiting or retrying; it SHALL then place an owner timeline clause before its client phase split that names the streamed events in file order and the last event, or reports the file absent, empty, stale (its first stamp precedes the starter's spawn) or unreadable, and that never carries a path, token, scope or tag. With the variable unset nothing SHALL change, including the supervisor protocol, the client's open stages and markers, and the owner-private directory contents. No other product code SHALL read, enumerate or remove these files. The variable MUST be documented only in the developer documentation. A Windows starter SHALL forward the variable to its owner only when it is exactly `1`.

#### Scenario: Unset changes nothing
- **WHEN** an owner opens, serves and closes without `KURU_OPEN_TIMELINE`, and a starter without it reaches its readiness deadline
- **THEN** the owner creates no `open-timeline-*` or `open-stream-*` file, the client's observed stage sequence is unchanged, no supervisor frame or marker differs, and the readiness deadline error carries no owner timeline clause

#### Scenario: Gated owner writes after releasing its lock
- **WHEN** an owner with `KURU_OPEN_TIMELINE=1` opens an existing project, serves and closes
- **THEN** no `open-timeline-*` file exists while the owner lock is held at any close step, and after the lock is released exactly one file named for the service generation exists with the canonical events in non-decreasing order, one wall-clock anchor, the binary version and the usage row count, and none of the data directory, project path, scope, scope hash, connection secret or store instance

#### Scenario: Write failure leaves the close unchanged
- **WHEN** the timeline file cannot be created because its name is occupied or its directory is unusable
- **THEN** the close still succeeds, the endpoint stays retired, the owner lock is free, and the occupying entry is untouched

#### Scenario: Successor owner during predecessor's write
- **WHEN** a successor owner starts after the predecessor released its owner lock but before the predecessor's timeline write finishes
- **THEN** the successor's open and its own generation-named file are unaffected, because each file's name is distinct and the write is exclusive-create

#### Scenario: Deadline names the held event
- **WHEN** a gated starter's gated owner is held after streaming `create-start` until the starter's readiness deadline passes
- **THEN** the deadline error keeps its leading text and carries `owner timeline:` with the owner's events in file order, `owner-main`, `owner-lock`, `startup-lock` and `create-start` among them, `last=create-start` and no later event, before a `client phases:` split that still parses

#### Scenario: Ungated starter reads nothing
- **WHEN** a starter without the variable reaches its readiness deadline while its owner's stream file exists
- **THEN** it reads no stream file and its error carries no owner timeline clause

#### Scenario: Stream absent after publication
- **WHEN** a gated owner started with a starter token publishes its endpoint and serves
- **THEN** its stream file was removed before it began serving, while it still held its owner lock, and no `open-stream-*` file remains

#### Scenario: Foreign tag never read
- **WHEN** a gated starter reaches its deadline while another token's stream file is present beside the endpoint
- **THEN** its clause is built only from the file named from its own token, or reports that file absent

#### Scenario: Taken stream name
- **WHEN** a gated owner's stream name is already taken by a file it did not create
- **THEN** the owner opens with streaming off and its in-memory log unchanged, the occupying file's bytes are unchanged and it is not removed, and a gated starter whose spawn followed that file's first stamp reports `owner timeline: stale`
