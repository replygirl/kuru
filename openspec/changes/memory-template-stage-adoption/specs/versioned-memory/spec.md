# Spec Delta

## MODIFIED Requirements

### Requirement: Current-schema staging and preserved failures

Fresh and legacy-import stores SHALL reach the same current schema and receipt
chain in their unpublished private staging directory before `ready.json` is
published. A failed or interrupted stage MUST remain unactivated, be stopped
before movement, and be preserved under the checked interrupted-stage protocol
with its identity, refs, working sets and imported history intact. Recovery
MUST NOT synthesize an activation record from SQL state, reuse a dirty stage or
move a live directory. Existing format-1 activation identity and original
legacy source/snapshot SHALL remain unchanged.

A staging directory MAY instead reach the current schema through an adopted
copy of a machine's store template rather than the migration chain. Recovery
MUST classify such a stage before any class that would start an engine, and
MUST NOT start an engine on one: a copy interrupted before its identity marker
was written, and a copy whose identity marker is present but whose adoption
never completed, are both preserved under the interrupted-stage protocol
without a server ever starting against them.

#### Scenario: Fresh or imported activation

- **WHEN** a new Dolt store is initialized directly or from a preserved SQLite snapshot
- **THEN** its owned live staging session validates the complete current schema and ordered receipts, publishes the existing activation record, and then stops and reaps the server before directory publication.

#### Scenario: Staged migration is interrupted

- **WHEN** migration fails before a staging store publishes `ready.json`
- **THEN** Kuru reaps its server and preserves the whole unactivated stage before constructing another, without modifying the source import or active project directory.

#### Scenario: Previous binary left a ready stage

- **WHEN** recovery finds an already-ready stage with a supported older schema, clean captured revision, valid receipt chain and only valid historical attempts through its recorded schema
- **THEN** it activates that accepted stage under the retained locks without changing its marker, then applies missing migrations through the ordinary active-main path; a newer-step attempt, dirty state or unknown schema fails before activation.

#### Scenario: Copy remnant is preserved without starting an engine

- **WHEN** recovery finds a staging directory with no `identity.json`, a `data/` directory, and no top-level entry outside the allowed set of `data/`, a `staging/` directory holding only temporary record files, and a Unix `lifecycle.lock`
- **THEN** Kuru waits for quiescence within the existing bound and preserves the whole stage under the interrupted-stage protocol without starting an engine against it.

#### Scenario: Unready template copy is preserved without starting an engine

- **WHEN** recovery finds a staging directory whose `identity.json` names a source template and no `ready.json` exists, whatever point the prior adoption attempt reached
- **THEN** Kuru waits for quiescence within the existing bound and preserves the whole stage under the interrupted-stage protocol without starting an engine against it, and never runs adoption SQL as part of recovery.

## ADDED Requirements

### Requirement: Template-born stage identity and adoption

A staging directory copied from a machine's store template SHALL carry an
optional `template` field in its activation identity record naming the
template it was copied from. A store that was not copied from a template
SHALL serialize its identity record with no such field present, byte-for-byte
as before this requirement existed, and the identity record format SHALL
continue to reject an unrecognized field so that a binary predating this
requirement fails closed on a template-born store's identity record instead of
misreading it.

While a staging directory's identity record is not yet marked initialized and
names a source template, Kuru MUST, before any other bootstrap mutation,
compare the named template against the supervisor's own compiled template key
and refuse to proceed if they differ, without treating that refusal as a
judgement on the template's bytes. Once initialized, a template-born store's
identity record MUST NOT be compared against any compiled key again, so that a
store already adopted from a template continues to open under a later
compiled key.

Adoption from a verified template copy MUST happen at most once, only from a
staging directory whose identity record is not yet marked initialized and
names a source template, and MUST NOT be attempted again once that mark is
set or from any recovery path. Adoption MUST verify that the placeholder
instance and project-scope row the template publishes is present, unchanged,
on both the copy's usage branch and its main branch before rewriting either
one, MUST rewrite that row to the copy's own new instance and project scope
and commit the change on the usage branch and then on main, each as its own
Dolt commit, and MUST verify the rewritten row on both branches before the
identity record is marked initialized. A copy's credentials MUST be newly
generated and MUST NOT be usable against any other copy of the same template
or against the template itself.

A completed comparison that finds the row, or the count of rows a rewrite
affected, to be something other than what adoption expects is a verdict
against the template's bytes, and Kuru MUST report it through a response type
distinct from an ordinary failure and from a client-side startup error. An
engine failure, a lost or malformed reply, a deadline, an authentication
failure, a SQL error unrelated to the expected row or count, or a mismatched
compiled template key MUST NOT be reported through that verdict response and
MUST leave the staging directory preserved exactly as an ordinary failed
staging attempt is preserved today.

Stores copied from the same machine's template for the same template key
SHALL share that template's pre-adoption history with identical commit
hashes: the schema-initialization commit, which holds only the fixed
placeholder identity, and the schema migration commits and their retained
branch heads. Those shared commits MUST contain only schema, migration
receipts and the placeholder identity, never project data, another project's
identity or any credential. Each copy's own history SHALL begin at its
adoption commits on main and on the usage branch, which record its own
instance and project scope, and its activation record's initial revision
SHALL be that main adoption commit.

#### Scenario: Cold store identity record is unchanged

- **WHEN** a store is created directly or by legacy import, without any source template
- **THEN** its identity record serializes with no template field present, byte-identical to the record a store without this requirement would have produced.

#### Scenario: Adoption gives a copy its own identity

- **WHEN** two staging directories are independently copied from the same verified template and each completes adoption
- **THEN** each ends with a distinct instance identifier, distinct project scope, distinct credentials and its own initial revision, neither copy's reader credential is accepted by the other's server, and both copies' pre-adoption commit hashes are identical to each other and to the template's.

#### Scenario: A verdict against the template's bytes is typed and distinct

- **WHEN** the placeholder row adoption expects is not present unchanged on either branch, or a rewrite affects a count of rows other than the one expected
- **THEN** Kuru reports the distinct verdict response, and the staging directory is preserved without adoption having been retried.

#### Scenario: An engine failure during adoption is not a verdict

- **WHEN** adoption meets an engine crash, a lost commit reply, a bootstrap deadline, an authentication failure, or a mismatch between the staging directory's named template and the supervisor's own compiled template key
- **THEN** Kuru reports an ordinary failure, not the distinct verdict response, and the staging directory is preserved exactly as an ordinary failed staging attempt is preserved today.

#### Scenario: An initialized template-born store opens under a later compiled key

- **WHEN** a store already adopted from a template is opened by a supervisor whose compiled template key differs from the one the store's identity record names
- **THEN** the open proceeds without any key comparison or adoption attempt, because the comparison applies only while the store's identity record is not yet marked initialized.
