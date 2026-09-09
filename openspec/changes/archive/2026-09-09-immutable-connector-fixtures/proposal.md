## Why

The initial Release run (34407995805) failed before publication when the model
catalog test tried to execute its native peer: Linux returned `ETXTBSY` (text file
busy). Each fixture currently writes a new executable while other tests may
fork, exposing executable files to transient inherited writable handles.

## What Changes

- Publish each fixture path as a link to an already compiled, immutable native
  peer instead of writing executable bytes during concurrent fixture startup.
- Keep independent wire plans and transcripts, bounded subprocess assertions,
  and deterministic cleanup of temporary compilation artifacts.
- Verify concurrent fixture isolation and executable sharing with a regression
  that rejects the previous per-fixture executable-copy mechanism.
- Remove pre-release-only wording from the packaged README and installation
  guide so those instructions remain accurate when the first release publishes.

## Capabilities

### Modified Capabilities

None. Existing connector behavior and specifications remain correct.

## Impact

Connector test support and its native fixture, README.md, docs/install.md, and
this cospec record.
No application behavior, dependencies, provider credentials, protocol behavior,
or release workflow topology changes. The failed release created no tag or
published assets; publication resumes from the reviewed correction.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
