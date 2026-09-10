## Context

Changing a subprocess working directory does not override GIT_DIR or related
variables exported by Git hooks. The official Git hooks documentation requires
clearing repository-local environment when operating on another repository:
https://git-scm.com/docs/githooks. The pre-push failure is recorded in
/tmp/kuru-note-policy-push.log. The accidental commit was empty; its ref and local
configuration changes were restored without discarding any working files.

## Decisions

Use a shared rooted command constructor to clear repository-local Git variables
before applying deliberate command overrides. Cover indirect Git users such as
cog and Communiqué, not only direct git invocations. Preserve unrelated provider
authentication, PATH and signing configuration. Do not mutate the process-global
environment, bypass hooks or relax check requirements.

## Risks / Trade-offs

Removing a deliberate temporary index would break immutable release recovery;
apply that override after sanitization and verify existing recovery tests. Test
poisoning must occur only in child process environments with temporary repositories,
so the regression cannot repeat the original mutation of the actual checkout.

## Operational surface

The same Rust delivery binaries run locally and on existing CI runners. No new
ports, services, tools, versions, secrets or publication jobs are introduced.

## Integration contract

Explicit root determines repository selection for real Git and Git-using tools.
Use actual subprocess and temporary repository state to prove selection and
caller preservation, including under hook-local environment inheritance.
