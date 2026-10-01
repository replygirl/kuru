# Spec Delta

## MODIFIED Requirements

### Requirement: Isolated schema construction and publication

Each migration step MUST create or recover a strictly named migration branch at
the exact clean active-main base. DDL, bounded data transformation, version
advance and receipt SHALL form one clean validated Dolt commit whose direct
parent is that base. Active `main` MUST remain at the complete prior version
until an exact-base, fast-forward-only publication succeeds. An acknowledged or
uncertain publication SHALL be accepted only after the original SQL session is
absent and independent reconciliation observes clean main at exactly the base
or validated target; every other result MUST fail closed without reset,
deletion or another mutation.

At and above the publication-record schema version, the same attempt commit
that advances schema and writes the receipt also records that step's branch
name, exact base, receipt operation and definition digest in the durable
publication table, before the fast-forward publish; the record therefore
reaches main in the same ref update as the schema and the receipt, never
before and never separately.

#### Scenario: DDL session disappears before commit

- **WHEN** migration DDL leaves a dirty working root on its isolated branch and the accepted session ends before `DOLT_COMMIT`
- **THEN** active main remains clean at its prior head and version, the dirty attempt remains reachable unchanged, and a fresh exact-base attempt may be constructed.

#### Scenario: Branch creation reply is lost

- **WHEN** the reply to creating a reserved exact-base migration branch is lost
- **THEN** Kuru reconciles the exact branch name and head before either reusing it or failing, without creating an ambiguous second owner.

#### Scenario: Completed attempt survives process loss

- **WHEN** startup discovers one clean unpublished migration branch with the exact base, target schema, receipt and postconditions
- **THEN** Kuru reuses that target for checked publication without replaying its DDL or adding another migration commit.

#### Scenario: Retained attempts outlive their migration step

- **WHEN** main already contains a migration receipt and later conversation commits while clean completed or recognized dirty failed branches for that applied step remain
- **THEN** a branch the publication table records for that step is accepted as historical when it has no uncommitted changes, its head's sole parent equals the recorded base, both head and base are ancestors of main's history, and an `AS OF` read of the head carries that step's schema version and the record's receipt operation, all checked from the pool the caller already holds without opening a connection to the branch or its commits; a branch the table does not record is classified as historical from its committed receipt, registered step and ancestry as before, requiring a clean completed branch's sole parent to validate as that step's source schema; either way every branch remains unchanged and Kuru continues opening or evaluating the next ordered step.

#### Scenario: Fast-forward reply is lost

- **WHEN** the real fast-forward has durably installed the validated target but its reply is lost
- **THEN** Kuru waits for the original session to end, recognizes the exact target once, and does not replay migration or publication.

#### Scenario: Attempt inventory is ambiguous

- **WHEN** reserved attempt names are malformed or excessive, multiple publishable targets exist, or an attempt has an unexpected head, receipt, ancestry or dirty shape
- **THEN** migration fails without changing main or deleting, resetting or merging any attempt.

#### Scenario: Publication record disagrees with main's history

- **WHEN** a retained branch's publication record names a base or head that is not in main's history, names a head whose sole parent is not the recorded base, names a branch with uncommitted changes, or names a receipt operation or definition digest that disagrees with the record's own `AS OF` read of the head
- **THEN** opening fails closed with the disagreement reported, without mutating the branch, the record or main, and without falling back to full classification of that branch.

#### Scenario: Store upgrades to publication records

- **WHEN** a writable open finds a supported store below the publication-record schema version
- **THEN** Kuru classifies every retained main-registry branch in full exactly once, applies the publication-record step through the ordinary migration path, records every branch that classification accepted as published inside that same step's attempt commit, and every later open verifies those branches by record instead of reclassifying them; a read-only open of that store continues to require the writable upgrade first, as for any pending step.

## ADDED Requirements

### Requirement: Migration publication records

Kuru SHALL maintain a durable table of publication records on `main`,
versioned with the schema, naming for every recorded main-registry step its
target version, exact branch name, exact base commit, receipt operation and
definition digest. The schema-authority join SHALL treat this table as
authority, exempt from the zero-rows rule that an otherwise-empty template or
store enforces on project-data tables. A record for an in-progress attempt
SHALL be checked against that attempt's own branch, base, receipt and
definition before the attempt is treated as complete.

#### Scenario: Attempt validation checks its own record

- **WHEN** an in-progress migration attempt at or above the publication-record schema version is validated before publication
- **THEN** Kuru requires the attempt's publication row to name the attempt's own branch, exact base, receipt operation and definition digest, and fails the attempt without publication if any of these disagree.

#### Scenario: Template carries publication records

- **WHEN** a store template is built or a store is copied from one
- **THEN** every retained main-registry branch the template's compiled registries name is also recorded in the publication table, and the template-shape check verifies those records as it verifies every other authority row, treating the publication table as authority rather than as project data that must be empty.
