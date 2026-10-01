# Spec Delta

## ADDED Requirements

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
one. A build MUST refuse to publish, and MUST leave any previously published
template for its key untouched, unless every one of those assertions passes.
Before publication, the captured `data/` tree MUST be scanned for occurrences
of the build directory's own absolute path, either engine secret used during
the build, or the local host name, and publication MUST be refused if any
occurrence is found. Publication MUST be a rename that does not replace an
existing published directory for the same key, verifying the moved
directory's identity after the rename; on a publication failure the build's
own verified, unpublished stage MAY still be copied by its own caller, and
MUST be left in place for the next exclusive-lock holder to resolve.

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
