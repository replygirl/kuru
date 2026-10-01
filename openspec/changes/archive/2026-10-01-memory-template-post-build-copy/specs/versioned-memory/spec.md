# Spec Delta

## MODIFIED Requirements

### Requirement: Store creation path selection

When a writable open finds the active store absent and recovery has found no
completed stage to reuse, Kuru MUST choose how to create the store before
starting any engine for it, and MUST NOT run more than one creation attempt
from the template in that open.

Kuru MUST take the cold staged build (initialization, an optional legacy
import, every schema step, validation and `ready.json` in a private staging
directory), never reading, building or copying a template, when a legacy
import is present, when the open is configured with an explicit engine binary,
or when a test fixture explicitly requests the cold path. For every other new
store Kuru MUST create the store from the per-machine store template of its
engine cache and compiled key:

- When a template is published for that key, Kuru MUST copy it into a new
  staging directory under the key's shared lock, write the stage's identity
  record last, and run the stage's one engine start, which adopts the copy,
  validates it, checks the template shape and publishes `ready.json`, before
  the unchanged quiescence, move onto the active path and active start: two
  engine starts in all, with no schema step run for this project.
- When no template is published for that key and the key's exclusive lock is
  free without waiting, Kuru MUST first build and publish the template on one
  engine, running the schema chain once, and then copy this project's stage
  from the template it built, or from the build's verified stage when
  publication failed, before the same adoption start and active start: three
  engine starts in all, and the schema chain runs once for the machine and
  key.
- The cold staged build keeps its own engine starts.

A creation worker MUST perform the copy, the build and the stage's engine
start. It MUST own the project's startup lock for that work, and the
template key's lock while a copy or build runs (the build's exclusive lock as
the build engine's reap guard, and every copy's lock held by the thread that
writes it), in the ownership shape of the migration worker: the opening frame
MUST NOT hold either lock while that work is in flight, a cancelled open MUST
leave the worker to finish, and the startup lock MUST return only after the
engine started on the project's stage has been reaped. The template build
engine runs on the key's build store, never on the project's stage, and the
key's exclusive lock, not the startup lock, is its reap guard: when that
engine's supervisor overruns its reap allowance, the guard passes to the
background reaper and MAY be released after the startup lock has returned.
No product opener waits on that guard. A later opener of the same project
waits for the startup lock within its own startup deadline, then reuses the
stage the worker left ready or finds it preserved.

Kuru MUST create the store cold instead, in a new staging directory and
without surfacing an error, when a template this open did not build cannot be
used now: the key lock is held by another process, the template root or the
key's lock file cannot be opened, locked or verified, the published template
fails its structural check (it is then quarantined, identity-bound and
best-effort), or the copy from that template fails with a verdict against the
template's bytes or an I/O error. This cold fallback MUST NOT apply to the
copy from a template, or verified build stage, that this open's own build
produced; that copy fails the open as described below. A copy that wrote
anything MUST first be preserved under the interrupted-stage protocol without
an engine start; only a verdict quarantines the template. A different project
opened while a build for the same key is in progress MUST take the cold path
at once and MUST NOT read any unpublished build or capture stage.

Any failure of a template build the open started (its engine, its own
validation and shape assertions, its capture and byte scan, or its
publication when no verified stage remains to copy from), any failure of the
copy of this project's stage from the template that build published or from
the build's verified stage, and any failure of the copied stage's own engine
start, MUST fail the open with its error and MUST NOT be retried in that open,
on the template path or the cold path, so the schema chain never runs twice
in one open. The error MUST name the phase that failed: a failed copy after a
successful build MUST NOT be reported as a failed build. When that
failure is a verdict against the template's bytes (the copy's byte
verification, adoption's placeholder, working-set or rewrite comparison, or
the template shape), Kuru MUST also quarantine the published template the
copy was taken from, bound to the identity it had when it was judged; a copy
taken from the build's unpublished verified stage quarantines nothing and
leaves that stage to the key's next sweep. Every other failure, including an
I/O error during the copy and a mismatched compiled template key, MUST leave
every template untouched. The unready stage is preserved: a copy remnant
under the interrupted-stage protocol without an engine start, by the creation
worker when the copy fails and otherwise by the next open's recovery, when
the failure came before the stage's engine was serving, and by the staging
job itself after.

#### Scenario: An ordinary new project copies a warm template in two starts

- **WHEN** a new project is opened on a machine whose engine cache holds a
  published template for the process's compiled key
- **THEN** the project's stage is copied from that template, adopted,
  validated and marked ready on one engine start, then activated with the
  active start, so the open makes two engine starts and runs no schema step.

#### Scenario: The first project for a key builds the template once, in three starts

- **WHEN** a new project is opened with an engine cache that holds no
  published template for the process's compiled key, and no legacy import,
  configured engine binary or cold request applies
- **THEN** Kuru builds and publishes the template on one engine before
  copying this project's stage from it, the open makes three engine starts,
  the schema chain runs once, and a later new project copies the published
  template in two.

#### Scenario: Legacy import and a configured engine binary stay cold

- **WHEN** a new project's open carries a legacy import, or the open is
  configured with an explicit engine binary
- **THEN** Kuru takes the cold staged build without reading, building or
  copying any template.

#### Scenario: A busy, unreadable or damaged template sends the opener cold

- **WHEN** a new project's open meets a key lock another process holds, a
  lock or manifest error, a template that fails its structural check, or a
  verdict or I/O error while copying a template this open did not build
- **THEN** the open completes on the cold staged build without a
  template-specific error, any partial copy is preserved without an engine
  start, and only a verdict against the template's bytes quarantines it.

#### Scenario: A verdict on the copy's own engine fails the open and quarantines

- **WHEN** a copied stage's engine start refuses the copy's placeholder
  identity or template shape
- **THEN** the open fails with the typed template verdict and no retry, no
  store appears at the project's active path, the unready stage is preserved,
  and the template it was copied from is quarantined; an engine failure of
  that start instead leaves the template untouched.

#### Scenario: A failed template build fails the open without a cold retry

- **WHEN** a new project's open builds the template for its key and that
  build's validation, shape assertion or byte scan refuses its result
- **THEN** the open fails with that error after the build engine's single
  start, makes no other engine start, runs no cold staged build, publishes
  and quarantines nothing, and leaves no store at the project's active path.

#### Scenario: A failed copy from the template this open built fails the open without a cold retry

- **WHEN** a new project's open builds and publishes the template for its key,
  and copying this project's stage from that template fails with a verdict
  against its bytes or an I/O error
- **THEN** the open fails with an error that names the copy from the
  template this open built and does not report a failed build. The build
  engine's single start is the open's only engine start, and no cold staged
  build runs. The copy remnant is preserved under the interrupted-stage
  protocol without an engine start, and no store appears at the project's
  active path. A verdict quarantines the template this open published, so
  the next new project builds again. An I/O error leaves it published, so
  the next new project copies it in two engine starts.

#### Scenario: A concurrent new project never waits on another project's template build

- **WHEN** one project's open is building and has not yet published a
  template for a key, and a second, different project is opened at the same
  time
- **THEN** the second project's open does not wait for the build: it takes
  the cold staged build at once and completes with its own engine starts,
  independently of when, or whether, the first project's build publishes.
