# Spec Delta

## ADDED Requirements

### Requirement: Store creation path selection

When a writable open finds the active store absent and recovery has run,
Kuru MUST choose between the schema-migration-chain cold path and a
template-based creation path before starting any engine for the new store,
and MUST NOT change that choice once a stage exists for this open.

Kuru MUST take the cold path, never attempting a template, when a legacy
import is present, when the open is configured with an explicit engine
binary override, or when the caller has explicitly requested the cold path.
For every other new-store open, Kuru MUST attempt a template-based path
first: if a published template for the process's compiled key is available
under the template cache's own rules, Kuru MUST copy a new project's stage
from it; if none is available because no template has been published for
that key yet, Kuru MUST build one under the key's own exclusive lock, on one
engine, before copying the new project's stage from the template it just
published, so that the schema-migration chain runs at most once per machine
and key regardless of how many projects are created afterward. Kuru MUST
fall back to the cold path, without the new project's own open ever
appearing to fail because of the template, whenever the template cache's own
rules already treat this open as having no usable template: a busy or
unreadable key lock, a lock verification failure, a damaged or foreign
template's structural check, or any other condition the template cache
requirement already treats as "no usable template for this key."

A creation worker performing a template copy or a template build MUST hold
the project's own startup lock and, for as long as the copy or build runs,
the template key's own lock, in the same ownership shape an ordinary
migration-chain stage worker already holds the startup lock: the open's own
calling frame MUST NOT hold either lock while the copy or build is in
flight, and both MUST return to their normal release path only after the
worker's engine, if one was started, is reaped.

When this open is the first to build a template for its key, on an otherwise
idle project, the build MUST run to completion and publish before this
open's own project stage is copied from it; a different, concurrently
opened project MUST NOT wait for that build and MUST instead take the cold
path immediately if it also needs a template under the same key while the
build is in progress, consistent with the template cache's own
no-waiting rule.

#### Scenario: An ordinary new project copies a warm template

- **WHEN** a new project is opened on a machine whose engine cache already
  holds a published template for the process's compiled key
- **THEN** the new project's stage is copied from that template, adopted and
  validated on one engine, then activated, without the schema-migration
  chain running for this project.

#### Scenario: The first project on a machine builds the template once

- **WHEN** a new project is opened on a machine and engine cache that holds
  no published template for the process's compiled key yet, and no legacy
  import, configured engine binary or explicit cold request applies
- **THEN** Kuru builds and publishes a template for that key on one engine
  before copying this project's stage from the newly published template, and
  the schema-migration chain runs exactly once for that key.

#### Scenario: Legacy import and a configured engine binary stay cold

- **WHEN** a new project's open carries a legacy import to run, or the open
  is configured with an explicit engine binary override
- **THEN** Kuru takes the schema-migration-chain cold path without
  attempting to read, build or copy any template.

#### Scenario: A busy or damaged template sends the opener cold, not into an error

- **WHEN** a new project's open meets a key lock that would block, a lock or
  manifest error, or a template that fails its structural check, for the
  process's compiled key
- **THEN** the open proceeds on the schema-migration-chain cold path and
  completes as an ordinary new-project open, without surfacing a
  template-specific error to the caller.

#### Scenario: A concurrent new project never waits on another project's template build

- **WHEN** one project's open is building and has not yet published a
  template for a key, and a second, different project is opened and needs a
  template under that same key at the same time
- **THEN** the second project's open does not wait for the first project's
  build or startup lock; it takes the schema-migration-chain cold path
  immediately and completes independently of when, or whether, the first
  project's build publishes.
