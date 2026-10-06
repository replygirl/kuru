# Spec Delta

## ADDED Requirements

### Requirement: Package-manager ownership refusal

Unix self-update MUST classify the resolved installed path against component-exact mise and Homebrew roots and refuse their files before network, build, staging or update-state effects. The refusal SHALL identify the manager and its upgrade command. Non-manager destinations MUST retain existing checked regular-file, single-link and parent-identity protections and pass current-user ownership/access preflight. Windows update behavior SHALL remain unchanged.

#### Scenario: Managed release or source update

- **WHEN** either self-update mode runs from a resolved mise or Homebrew installation, including a symlink alias
- **THEN** it refuses with the owning manager's actionable command and leaves executable bytes, identity and inventory unchanged without release requests, source builds or update-state creation

#### Scenario: Unsafe unmanaged destination

- **WHEN** a non-manager installed object is a link, foreign-owned file, changed identity or is in a parent without replacement access
- **THEN** checked preflight refuses before self-update effects and preserves all existing objects

#### Scenario: Component boundaries and existing installations

- **WHEN** an ordinary checked user-owned installation has path components resembling but outside manager installation roots
- **THEN** it retains its verified release/source behavior without being falsely classified as package-manager-owned
