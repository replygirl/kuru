# Spec Delta

## MODIFIED Requirements

### Requirement: Install stage teardown ordering

Once runtime provisioning has created a private install stage under the exclusive installation lock, it MUST release that lock only after the stage is resolved, on every exit that follows: publication, an error before publication, caller cancellation, unwinding, and the end of activation recovery. The stage is resolved in one of three ways. It is removed through the checked stage removal. It is retained after a refused or uncertain removal, and that retention is reported to diagnostics and recorded in a `.leftovers` receipt that names the stage and whether its engine was published. Or, for a failed activation, it is deliberately preserved as evidence and named in the returned error. A removal failure of such a stage MUST NOT be discarded silently, and a stage whose removal is uncertain MUST NOT be deleted again. Teardown on a cancelled or unwinding owner MAY block its thread only for the existing bounded stage-cleanup window. A stage counts as created once its owner-only private directory exists; a container abandoned because that directory could not be created is outside this requirement.

The receipt MUST record the first cause, its native OS error when one exists, and the attempt count and elapsed time of the bounded recovery window when that window ran. It MUST also record the descendant of the stage whose removal refused, relative to the stage, when the checked removal names one. On Windows, when the stage's engine was probed, the receipt MUST record the probe child's process id and creation time, and whether a process object with that exact identity could still be opened when the refusal was recorded (`retained`, `released`, or `unknown` with its cause). Recording these facts MUST NOT change how the stage is resolved, and a failure to observe one of them MUST be recorded rather than fail the release.

#### Scenario: Held stage on a cancelled or failed installation
- **WHEN** an installation is cancelled or fails before publication while a handle inside its private stage refuses removal
- **THEN** the stage stays on disk, its retention is reported and receipted with `published: false` while the installation lock is still held, the receipt and diagnostic name the retained stage, an error returned through the lease names it, and the lock is released afterwards

#### Scenario: Retained stage after publication
- **WHEN** a verified engine is published but its private stage cannot be removed
- **THEN** the retained-stage receipt and diagnostic are written before the installation lock is released, and the open still succeeds

#### Scenario: Unheld stage
- **WHEN** an installation publishes, fails or is cancelled with nothing holding its stage
- **THEN** the stage is gone before the installation lock is released

#### Scenario: Refusing descendant and probe child are recorded
- **WHEN** a probed stage's removal is refused by one of its descendants
- **THEN** the receipt names that descendant relative to the stage and, on Windows, the probe child's identity and whether its process object was still open at the refusal, and the stage is resolved exactly as it would be without them

## ADDED Requirements

### Requirement: Leftover stage collection

Every acquisition of the installation lock by runtime provisioning MUST attempt each `.leftovers` receipt present at that moment once, before any other work under the lock, through one checked removal of the stage it names. A warm open MUST NOT wait for the lock. It attempts the receipts only when the lock is free, and a busy lock leaves every receipt and stage unchanged. A refused or uncertain removal MUST leave both the stage and its receipt, MUST NOT be retried within the same sweep, and MUST be recorded in that receipt under the lock: it increments `sweep_refusals` and replaces `last_sweep_refusal` with the refusal's phase, cause, native OS error, refusing descendant, and (on Windows) the probe child's state at that refusal. The receipt's other fields MUST be kept, and its `version` stays `1`. When a sweep leaves no receipt and the receipts folder holds nothing but an empty `staging` directory, the sweep MUST attempt one checked removal of the receipts folder under the lock. A refusal leaves the folder for the next sweep and is reported to diagnostics. A `staging` directory that still holds a record MUST NOT be removed. A warm open whose lock attempt fails with an error, as opposed to a busy lock, MUST report that to diagnostics and MUST NOT write anything.

#### Scenario: Next acquirer attempts each receipt
- **WHEN** a receipted stage exists and runtime provisioning takes the installation lock on a cold or a warm open
- **THEN** it attempts that stage's removal once before any other work under the lock, and on success removes the stage and then its receipt

#### Scenario: Busy lock on a warm open
- **WHEN** a warm open finds a receipt while another holder has the installation lock
- **THEN** the open does not wait, the stage and receipt are unchanged and record no refusal, and the next acquisition attempts the receipt

#### Scenario: Refused sweep
- **WHEN** a sweep's checked removal of a receipted stage is refused or uncertain
- **THEN** the stage and its receipt stay, the receipt keeps its original fields and records `sweep_refusals` and the last refusal's cause, OS error and descendant, nothing is deleted on uncertainty, and the sweep does not retry

#### Scenario: Last receipt collected
- **WHEN** a sweep collects the last receipt and the receipts folder holds only an empty `staging` directory
- **THEN** the sweep attempts one checked removal of the receipts folder, and a refusal leaves it for the next sweep and is reported to diagnostics

#### Scenario: Staging record kept
- **WHEN** a sweep leaves no receipt but the receipts folder's `staging` directory still holds a record
- **THEN** the receipts folder and its `staging` directory are kept

#### Scenario: Failed lock attempt on a warm open
- **WHEN** a warm open finds a receipt and its non-blocking lock attempt fails with an error
- **THEN** one diagnostics record reports the skipped sweep and its cause, and nothing on disk changes
