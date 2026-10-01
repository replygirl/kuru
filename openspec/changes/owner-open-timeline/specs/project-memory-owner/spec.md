# Spec Delta

## ADDED Requirements

### Requirement: Owner open timeline

Only when the environment variable `KURU_OPEN_TIMELINE` is exactly `1` in the memory service owner process, the owner SHALL record named open milestones as nanosecond offsets from one monotonic anchor per process, in a bounded in-memory log sealed at endpoint publication. It SHALL write that log once, after it has released its owner lock in close, as a non-durable owner-private file named for its service generation beside its endpoint, created through the checked private-file handle so that it never follows a link, truncates or replaces an existing file. The file SHALL carry only event names, offsets, one wall-clock anchor, the service generation, the binary version and counts, and never a path, scope, SQL text, store identity, connection secret, credential or content. Recording or writing MUST NOT fail, cancel, delay or reorder the open, serve or close, and MUST NOT lengthen any successor's owner-lock wait; a failed open writes nothing. With the variable unset nothing SHALL change, including the supervisor protocol, the client's open stages and markers, and the owner-private directory contents. No product code SHALL read, enumerate or remove these files. The variable MUST be documented only in the developer documentation. Support is not claimed on Windows, whose owner does not inherit the variable.

#### Scenario: Unset changes nothing
- **WHEN** an owner opens, serves and closes without `KURU_OPEN_TIMELINE`
- **THEN** it creates no `open-timeline-*` file, the client's observed stage sequence is unchanged, and no supervisor frame or marker differs

#### Scenario: Gated owner writes after releasing its lock
- **WHEN** an owner with `KURU_OPEN_TIMELINE=1` opens an existing project, serves and closes
- **THEN** no `open-timeline-*` file exists while the owner lock is held at any close step, and after the lock is released exactly one file named for the service generation exists with the canonical events in non-decreasing order, one wall-clock anchor, the binary version and the usage row count, and none of the data directory, project path, scope, scope hash, connection secret or store instance

#### Scenario: Write failure leaves the close unchanged
- **WHEN** the timeline file cannot be created because its name is occupied or its directory is unusable
- **THEN** the close still succeeds, the endpoint stays retired, the owner lock is free, and the occupying entry is untouched

#### Scenario: Successor owner during predecessor's write
- **WHEN** a successor owner starts after the predecessor released its owner lock but before the predecessor's timeline write finishes
- **THEN** the successor's open and its own generation-named file are unaffected, because each file's name is distinct and the write is exclusive-create
