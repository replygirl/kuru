# unix-update-recovery Specification

## Purpose

Provide checked Unix executable replacement, receipt-driven forward recovery and native source updates while preserving installation ownership, read-only build inputs and uncertain recovery evidence.

## Requirements

### Requirement: Checked durable Unix replacement

Unix Kuru SHALL replace its executable through a receipt-driven same-filesystem transaction. Before creating candidate or backup files it MUST retain exact installation identity and record exclusively derived names in a bounded versioned receipt. Complete images MUST match retained identity, digest and length. Publication MUST revalidate installation and use a checked atomic replacement; uncertainty MUST reconcile actual effects.

#### Scenario: Successful settlement
- **WHEN** a verified release core or checked source build replaces a supported Unix installation
- **THEN** the installed file is the exact replacement, no candidate or backup image remains, the receipt is retired and the stable install lock remains

#### Scenario: Changed destination during preparation
- **WHEN** the destination, retained ancestor, candidate or backup changes before publication or recovery
- **THEN** Kuru refuses the unproved operation and preserves unrelated or uncertain files without overwriting them

### Requirement: Forward-only automatic recovery

Unix Kuru SHALL recover checked pending installation state inline, before another update and at ordinary startup without a new recovery command. Replacement proof MUST finish forward. Restoration MUST require exact evidence that the replacement was never installed; possible publication with an absent installed image MUST refuse and preserve state. Recovery MUST NOT open memory or execute candidate, backup or build input for validation.

#### Scenario: Lost publication reply or process interruption
- **WHEN** a process stops before or after the atomic replacement
- **THEN** recovery derives its action from exact receipt and image evidence, settles the provable original or replacement, and never guesses publication from the saved phase alone

#### Scenario: Installed image absent after possible publication
- **WHEN** published evidence exists or a prepared candidate was consumed and the installed image is absent
- **THEN** recovery retains every recovery image and refuses rather than restoring an older executable

#### Scenario: Busy or incompatible startup receipt
- **WHEN** ordinary startup finds a busy installation transaction or an unknown receipt version
- **THEN** it respectively skips the busy recovery or reports a bounded stderr refusal, preserves state and leaves stdout and the intact running command unchanged

### Requirement: Native source input and pending bootstrap state

Explicit Unix source update SHALL use direct package-owned mise build commands and consume the output read-only through the same installation transaction. It MUST preserve Cargo hardlinks and literal arguments and MUST NOT delegate to the checkout installation shell script or execute output for validation. Unix bootstrap MUST preserve pending receipt state and refuse clobbering a present installation awaiting recovery.

#### Scenario: Read-only linked Cargo output
- **WHEN** the selected native build output is a read-only regular hardlinked file
- **THEN** installation copies verified bounded bytes, leaves source bytes, mode and links unchanged and publishes a single-link executable

#### Scenario: Pending receipt meets first-install bootstrap
- **WHEN** the bootstrap sees a pending receipt
- **THEN** a present installed image causes refusal, while an absent image preserves pending state aside without restoring its backup before the explicitly selected installation
