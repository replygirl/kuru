# Spec Delta

## ADDED Requirements

### Requirement: Install stage teardown ordering

Runtime provisioning MUST release the exclusive installation lock only after the private install stage it serializes is resolved, on every exit: publication, an error before publication, caller cancellation, unwinding, and the end of activation recovery. The stage is resolved in one of three ways. It is removed through the checked stage removal. It is retained after a refused or uncertain removal, and that retention is reported to diagnostics and recorded in a `.leftovers` receipt that names the stage and whether its engine was published. Or, for a failed activation, it is deliberately preserved as evidence and named in the returned error. A removal failure MUST NOT be discarded silently, and a stage whose removal is uncertain MUST NOT be deleted again. Teardown on a cancelled or unwinding owner MAY block its thread only for the existing bounded stage-cleanup window.

#### Scenario: Held stage on a cancelled or failed installation
- **WHEN** an installation is cancelled or fails before publication while a handle inside its private stage refuses removal
- **THEN** the stage stays on disk, its retention is reported and receipted with `published: false` while the installation lock is still held, an error return names the retained stage, and the lock is released afterwards

#### Scenario: Retained stage after publication
- **WHEN** a verified engine is published but its private stage cannot be removed
- **THEN** the retained-stage receipt and diagnostic are written before the installation lock is released, and the open still succeeds

#### Scenario: Unheld stage
- **WHEN** an installation publishes, fails or is cancelled with nothing holding its stage
- **THEN** the stage is gone before the installation lock is released
