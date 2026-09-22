## ADDED Requirements

### Requirement: Native search and paging reference

Curated documentation SHALL identify `grep`, `glob`, and `file_read` paging
as checked project-root tools, document their permission behavior, bounds,
binary handling, hidden and ignore defaults, and state that native search does
not need an installed `rg` executable. It SHALL define paging as one-based
logical UTF-8 text lines and explain returned continuation and omission fields.

#### Scenario: User pages a large text file
- **WHEN** a user reads the built-in-tools reference before using file_read
- **THEN** they can request a line offset and limit and use next_offset without
  inferring byte, Unicode-scalar, or external-tool semantics.
