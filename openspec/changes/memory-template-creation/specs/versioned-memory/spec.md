# Spec Delta

## ADDED Requirements

### Requirement: Store creation path selection

When a writable open finds the active store absent and recovery has found no
completed stage to reuse, Kuru MUST choose how to create the store before
starting any engine for it, and MUST NOT run more than one creation attempt
from the template in that open.

Kuru MUST take the cold staged build (initialization, an optional legacy
import, every schema step, validation and `ready.json` in a private staging
directory), never reading, building or copying a template, when a legacy
import is present, when the open is configured with an explicit engine binary,
or when a test fixture explicitly requests the cold path. For every other new
store Kuru MUST create the store from the per-machine store template of its
engine cache and compiled key:

- When a template is published for that key, Kuru MUST copy it into a new
  staging directory under the key's shared lock, write the stage's identity
  record last, and run the stage's one engine start, which adopts the copy,
  validates it, checks the template shape and publishes `ready.json`, before
  the unchanged quiescence, move onto the active path and active start: two
  engine starts in all, with no schema step run for this project.
- When no template is published for that key and the key's exclusive lock is
  free without waiting, Kuru MUST first build and publish the template on one
  engine, running the schema chain once, and then copy this project's stage
  from the template it built, or from the build's verified stage when
  publication failed, before the same adoption start and active start: three
  engine starts in all, and the schema chain runs once for the machine and
  key.
- The cold staged build keeps its own engine starts.

A creation worker MUST perform the copy, the build and the stage's engine
start. It MUST own the project's startup lock for that work, and the
template key's lock while a copy or build runs (the build's exclusive lock as
the build engine's reap guard, and every copy's lock held by the thread that
writes it), in the ownership shape of the migration worker: the opening frame
MUST NOT hold either lock while that work is in flight, a cancelled open MUST
leave the worker to finish, and the startup lock MUST return only after every
engine the worker started has been reaped. A later opener of the same project
waits for the startup lock within its own startup deadline, then reuses the
stage the worker left ready or finds it preserved.

Kuru MUST create the store cold instead, in a new staging directory and
without surfacing an error, when the template cannot be used now: the key lock
is held by another process, the template root or the key's lock file cannot be
opened, locked or verified, the published template fails its structural check
(it is then quarantined, identity-bound and best-effort), or the copy fails
with a verdict against the template's bytes or an I/O error. A copy that wrote
anything MUST first be preserved under the interrupted-stage protocol without
an engine start; only a verdict quarantines the template. A different project
opened while a build for the same key is in progress MUST take the cold path
at once and MUST NOT read any unpublished build or capture stage.

Any failure of a template build the open started (its engine, its own
validation and shape assertions, its capture and byte scan, or its
publication when no verified stage remains to copy from), and any failure of
the copied stage's own engine start, MUST fail the open with its error and
MUST NOT be retried in that open, on the template path or the cold path, so
the schema chain never runs twice in one open. When that
failure is a verdict against the template's bytes (adoption's placeholder,
working-set or rewrite comparison, or the template shape), Kuru MUST also
quarantine the published template the copy was taken from, bound to the
identity it had when it was judged; every other failure, including a
mismatched compiled template key, MUST leave every template untouched. The
unready stage is preserved: by the next open's recovery without an engine
start when the failure came before the stage's engine was serving, and by the
staging job itself after.

#### Scenario: An ordinary new project copies a warm template in two starts

- **WHEN** a new project is opened on a machine whose engine cache holds a
  published template for the process's compiled key
- **THEN** the project's stage is copied from that template, adopted,
  validated and marked ready on one engine start, then activated with the
  active start, so the open makes two engine starts and runs no schema step.

#### Scenario: The first project for a key builds the template once, in three starts

- **WHEN** a new project is opened with an engine cache that holds no
  published template for the process's compiled key, and no legacy import,
  configured engine binary or cold request applies
- **THEN** Kuru builds and publishes the template on one engine before
  copying this project's stage from it, the open makes three engine starts,
  the schema chain runs once, and a later new project copies the published
  template in two.

#### Scenario: Legacy import and a configured engine binary stay cold

- **WHEN** a new project's open carries a legacy import, or the open is
  configured with an explicit engine binary
- **THEN** Kuru takes the cold staged build without reading, building or
  copying any template.

#### Scenario: A busy, unreadable or damaged template sends the opener cold

- **WHEN** a new project's open meets a key lock another process holds, a
  lock or manifest error, a template that fails its structural check, or a
  verdict or I/O error while copying
- **THEN** the open completes on the cold staged build without a
  template-specific error, any partial copy is preserved without an engine
  start, and only a verdict against the template's bytes quarantines it.

#### Scenario: A verdict on the copy's own engine fails the open and quarantines

- **WHEN** a copied stage's engine start refuses the copy's placeholder
  identity or template shape
- **THEN** the open fails with the typed template verdict and no retry, no
  store appears at the project's active path, the unready stage is preserved,
  and the template it was copied from is quarantined; an engine failure of
  that start instead leaves the template untouched.

#### Scenario: A failed template build fails the open without a cold retry

- **WHEN** a new project's open builds the template for its key and that
  build's validation, shape assertion or byte scan refuses its result
- **THEN** the open fails with that error after the build engine's single
  start, makes no other engine start, runs no cold staged build, publishes
  and quarantines nothing, and leaves no store at the project's active path.

#### Scenario: A concurrent new project never waits on another project's template build

- **WHEN** one project's open is building and has not yet published a
  template for a key, and a second, different project is opened at the same
  time
- **THEN** the second project's open does not wait for the build: it takes
  the cold staged build at once and completes with its own engine starts,
  independently of when, or whether, the first project's build publishes.

## MODIFIED Requirements

### Requirement: Per-machine store template cache

A machine's engine cache MAY hold, under its current engine version
directory, a private `templates/` area keyed by content: a permanent lock
file per key, a published template directory per key holding only a manifest
and a captured `data/` tree, and transient build, capture-stage and
quarantine directories distinguished from a published template by name. A
published template directory MUST hold only its manifest and `data/`, and its
`data/` MUST hold only schema, migration receipts and a fixed placeholder
identity: no `identity.json`, `config/`, `home/` or engine secret, so the same
published template MAY be copied by every project that shares the cache, and
an explicit project purge MUST NOT remove it.

The key naming a published template MUST be computed, once per process, from
exactly the inputs that determine the bytes a fresh chain-built store would
hold: a fixed format constant, the pinned engine version, the pinned engine
executable's verified digest, the target triple, the current schema version,
the current usage-schema version, the digest of every compiled migration and
usage-migration registry definition in declared order, and the creation-time
SQL statements, placeholder literals, commit-message formats and supervisor
configuration behavior that shape an empty store's bytes. The key MUST NOT
depend on the supervisor executable's own bytes or on any source file's
digest, so an instrumented and an ordinary supervisor build interchangeable
templates for the same key. A template built under one key MUST NOT be read,
copied or treated as valid under a different key.

A template MUST be built only under its key's own exclusive lock, by running
the real schema-migration chain and the real usage-branch upgrade against a
fixed placeholder identity on one engine, then validating that engine's
active schema and the same shape assertion a copied stage validates after
adoption, parameterized with the placeholder identity in place of an adopted
one. That shape assertion MUST read every commit on the main branch and on the
usage branch and compare each commit's committer, committer email, author,
author email and message, and the history's length, with the history derived
from the compiled registries: the engine's own first commit under its fixed
system account, then commits under Kuru's fixed author with the
initialization message, one upgrade message per executed step naming its
retained attempt's operation, and the adoption message once adopted. A
completed history read that differs is a verdict against the bytes. A build
MUST refuse to publish, and MUST leave any previously published template for
its key untouched, unless every one of those assertions passes.
Before publication, the captured `data/` tree MUST be scanned for occurrences
of the build directory's own absolute path, either engine secret used during
the build, or the local host name, and publication MUST be refused if any
occurrence is found. The scan finds literal occurrences only, because the
engine stores its data compressed, and it MUST skip a host name shorter than
four bytes, which occurs by chance in compressed storage and would refuse
sound builds. The shape assertion above, which reads the database itself, is
the guarantee that a template holds no project data, recorded host or user
identity or credential; the byte scan is an additional precaution. Publication MUST be a rename that does not replace an
existing published directory for the same key, verifying the moved
directory's identity after the rename; on a publication failure the build's
own verified, unpublished stage MAY still be copied by its own caller, and
MUST be left in place for the next exclusive-lock holder to resolve. A holder
of a key's exclusive lock that builds or quarantines MAY remove that same
key's own abandoned build and capture-stage directories, without waiting for
an engine that has not yet stopped: an entry whose engine lease is still held,
or whose removal fails, MUST be left for a later holder.

A verified copy of a published template MUST be obtained only under that
key's shared lock, and MUST independently verify, while copying, that the
manifest parses to the expected format and key, that the top-level entries
present are exactly the manifest's declared entries, and that every copied
file matches the manifest's declared size and SHA-256 digest; a symbolic link,
an extra hard link, or an entry of a different type than the manifest declares
MUST cause the copy to fail rather than succeed silently. No part of this
verification depends on the copying process's own trust of the manifest's
origin: the manifest only names what to check, and the checks that decide
pass or fail run only against the compiled key, the compiled placeholder
identity and the shape assertion described above.

A published template MUST be moved aside into a quarantine location, and
never read as valid again under its key, only when a completed comparison
against its own bytes returns a value other than the one expected: the
manifest's declared format or key, the per-file verification above, the
placeholder identity row checked during adoption, or the shape assertion.
Every other failure reachable while building, publishing or copying a
template — an engine crash, a lost or malformed reply, any deadline, an
authentication failure, a lock acquisition that would block, a lock or
manifest read error, or an I/O error during the copy — MUST leave every
existing published template, for every key, untouched, and MUST be treated as
ordinary failure rather than as a verdict against the template's bytes. The
quarantine action MUST be bound to the identity of the specific template
directory that was judged: it MUST re-check that the directory holding the
published name still has the judged identity under the key's own exclusive
lock before moving it aside, and MUST skip the quarantine, with a recorded
warning and no error raised to the caller, if the identity no longer matches,
the exclusive lock cannot be acquired without waiting, or the move fails. At
most one quarantined directory per key is kept; an older one MUST be removed,
best-effort, before a new one is quarantined.

Code that creates or opens a project MUST NOT wait for another process's
template build or quarantine, MUST NOT remove another key's published template
or quarantined directory, and MUST NOT remove any template lock file: lock
files for this cache are permanent for the life of the cache directory. Any error
encountered opening the `templates/` area, acquiring or verifying a key's own
lock, or reading its manifest, MUST be treated the same as "no usable
template for this key": the caller proceeds without one, not as an open
failure, except in test-fixture warm-up, where the same errors MUST be
reported rather than silently treated as absence, because fixture warm-up has
no caller to send down a fallback path.

#### Scenario: A template is shared by every project on the machine

- **WHEN** two different projects, on the same machine and engine cache, each
  have a compiled template key that matches a currently published template
- **THEN** each project's creation is free to copy that same published
  `data/` tree, and copying it for one project changes nothing a later copy
  for a different project reads.

#### Scenario: A template build refuses to publish on a failed assertion

- **WHEN** a template build's own validation, shape check or byte scan fails
  before publication
- **THEN** no new directory is published under that key, any previously
  published template for that key is left exactly as it was, and the next
  attempt to use that key starts a fresh build under the same exclusive lock.

#### Scenario: Concurrent new projects never wait on a template build

- **WHEN** one process holds a key's exclusive lock to build or publish a
  template and a second process, for a different project, needs a template
  under the same key at the same time
- **THEN** the second process's attempt to acquire the key's lock does not
  block it: it proceeds immediately as if no template were available for
  that key, without reading any unpublished build or capture-stage directory.

#### Scenario: A verdict against the bytes quarantines; an engine failure does not

- **WHEN** a copy's per-file verification finds a digest or size mismatch
  against the manifest, or the manifest's declared format or key is wrong
- **THEN** the published template for that key is quarantined, identity-bound
  to the directory that was judged, and never read as valid again under that
  key; but when the same copy instead meets a lock contention, an I/O error,
  or any other failure that is not a completed comparison against the
  template's own bytes, the published template is left untouched and the
  failure is treated as ordinary.

#### Scenario: No open path garbage-collects another key or any lock file

- **WHEN** a process builds or copies a template for one key, in a cache
  whose `templates/` area also holds a published template, a quarantined
  directory and a lock file for at least one other key
- **THEN** every other key's published template, quarantined directory and
  lock file remain byte-identical and present afterward, and no lock file is
  ever removed by this process.

#### Scenario: A classifier refusal is engine-side evidence, not a verdict

- **WHEN** the shared template-shape check or the main-pool classification
  it depends on refuses to classify a retained migration branch found on a
  copy or on the build engine
- **THEN** that refusal is reported as an ordinary engine-side failure and
  MUST NOT, by itself, be treated as a verdict against a template's bytes.

#### Scenario: An older client fails closed on an unrecognized rejection

- **WHEN** a supervisor reports the typed template-rejection response and the
  connected client's build predates that response variant
- **THEN** the client fails closed with a parse error on the unrecognized
  reply rather than treating it as success or silently discarding it.
