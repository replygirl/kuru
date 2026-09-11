## Why

Windows CI 34588322751 restored the Cargo target cache and then rejected both
bundle-preparation prerequisites because the restored private directory granted
access to another principal. Keep private build inputs out of cross-run archive
restoration while preserving the existing permission and integrity checks.

## What Changes

- `.github/workflows/native-tests.yml`: give the cached native job an absolute
  bundle directory under its runner temporary storage, shared by its preparation,
  coverage, source-install, offline-build and shipping steps.
- `docs/development.md` and `AGENTS.md`: document the boundary between reusable
  Cargo artifacts and private, job-local bundle preparation.

## Impact

The reusable native-test job also supplies release validation. Its jobs, pinned
tools, required checks, artifact paths, deadlines and permissions stay the same;
no new secret or release/Pages action is needed. The package-owned mise graph
continues preparing and verifying inputs. No application source, build verifier,
local default cache, ACL policy or specification changes.

## Surfaces

- [ ] interactive — no product interaction change
- [x] deploy — native CI build-input storage outside the restored Cargo cache
- [ ] integration — existing cache/action interfaces remain unchanged
- [ ] agent-behavior — no model or tool authority change
