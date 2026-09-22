## ADDED Requirements

### Requirement: Native file mutations use durable checkpoints

The native ToolHost SHALL admit `file_write`, `file_edit` and `file_delete` only with a project-bound private checkpoint store, after the existing exact-file permission, root containment, protected-path and instruction-activation decisions. It MUST retain those decisions through final target revalidation and report any checkpoint or file-publication uncertainty without treating shell or MCP effects as covered.

#### Scenario: No checkpoint store
- **WHEN** a host has no usable private checkpoint store for a write-capable native file call
- **THEN** that call is not advertised or executed as a recoverable file mutation.

#### Scenario: Denied file mutation
- **WHEN** the exact target's file permission is denied or nested instructions require a replan
- **THEN** no target bytes or checkpoint publication is performed for the stale call.
