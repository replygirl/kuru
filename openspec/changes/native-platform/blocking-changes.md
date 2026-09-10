# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Independent foundation

The current proposal, every active proposal and archived providers were reviewed.
This package uses Rust/native OS APIs and compiled Rust fixtures; it consumes no
Dolt, embedded bundle, application, archive or updater implementation. Existing
archived monorepo and delivery conventions already supply its task/CI baseline.
The parent task explicitly establishes this independent scope and dependency
classification; no additional product approval gate is inferred.

`native-windows` consumes this foundation and remains hard-blocked by
`native-platform`, `dolt-memory` and `embedded-runtime`. Neither database nor
embedding work acquires a reverse dependency merely to test this package.
Passing package CI proves its operations only, not Windows application support.
