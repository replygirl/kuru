## ADDED Requirements

### Requirement: Native search permission and result projection

The provider-facing native tool catalog SHALL expose checked `grep`, `glob`,
and paged `file_read` definitions with bounded argument schemas. Search SHALL
first honor the native selector's catalog gate, then evaluate every discovered
effective target using its canonical project-relative spelling. It SHALL not
request or grant a search root or subtree as an authority target. A broad search
request SHALL NOT expose a target denied by a more-specific rule.
Successful and failed results SHALL pass through the existing redaction and
bounded model-receipt projection boundary.

#### Scenario: Broad grep cannot bypass a file denial
- **WHEN** grep is allowed for a requested subtree but a descendant file has a
  matching deny rule
- **THEN** the descendant is omitted without its path, contents, or matching
  line being returned.
