## ADDED Requirements

### Requirement: Checked directory-tree removal

The platform SHALL provide a consuming checked directory-tree removal primitive
for a caller-held private `Directory`. It MUST retain and verify the root
identity, reject symlink and Windows reparse descendants, expose rejected versus
uncertain native removal outcomes, and never authorize removal of a replacement
that later occupies the former name. Domain inventory, retry, and receipt policy
remain with the caller.

#### Scenario: Replaced or linked removal root
- **WHEN** a caller-held directory name is replaced or a descendant is a symlink
  or reparse point before removal
- **THEN** the primitive rejects without traversing or removing the replacement
  or link target.

#### Scenario: Native removal uncertainty
- **WHEN** native directory removal may have happened before an error is
  observed
- **THEN** the outcome retains the original identity and enough location detail
  for the caller to reconcile without a destructive automatic retry.
