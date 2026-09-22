## ADDED Requirements

### Requirement: Progressive project prompt sources retain complete trust binding

Automatic project skill metadata and effective custom-command bytes SHALL contribute typed source-bound claims to the initial exact-root manifest. Selected project skill body/reference bytes SHALL contribute typed claims only to a complete supplemental manifest, without changing the approved base. The existing once/persist/deny flow and private v2 root record SHALL govern each new effective supplemental source set, and persistent publication MUST compare the exact pre-review generation under the root lock. Revocation or base reapproval during review MUST invalidate a pending publication. User-config entries remain caller authority and MUST NOT bless unrelated repository instructions, hooks, MCPs or tools.

#### Scenario: Later skill body cannot borrow metadata approval
- **WHEN** a project skill's metadata is approved at startup but its body is selected later
- **THEN** the metadata approval alone does not authorize the body's prompt bytes or any reference, and the complete supplemental manifest is reviewed before exposure.

#### Scenario: Revoked approval while waiting
- **WHEN** a persistent skill review starts against one approval generation and that root approval is revoked or replaced before publication
- **THEN** publication fails without restoring the old base or selected material.
