# Spec Delta

## ADDED Requirements

### Requirement: Compatible Linux self-launch after replacement

After its installed pathname is replaced, a running Linux Kuru SHALL launch its own compatible native memory service and lifetime supervisor from its actual running executable. It MUST NOT trim a deleted pathname or silently execute the new occupant. Explicit prepared supervisor selection and checked attachment compatibility MUST remain intact.

#### Scenario: Old mapped executable starts storage after update
- **WHEN** a retained running Kuru has its installed executable atomically replaced and subsequently needs a new project memory owner
- **THEN** its checked self-launch starts its own build's service and supervisor, or refuses unproved self-image authority, and never substitutes the replacement's protocol or engine

#### Scenario: Compatible active owner remains attached
- **WHEN** the project already has a checked compatible owner
- **THEN** replacement does not terminate that owner or change the existing attachment, driver claims, private state or uncertain-operation recovery
