## Context

The v0.8.0 Release published immutable Windows assets after staged acceptance succeeded. Its post-public verifier downloaded the same archive and launched the installed Kuru through mise. The first attempt reported a memory readiness deadline. The second completed a cold conversation but `bounded_output` rejected the command because the independent memory service remained in the verifier's Windows Job. The first readiness delay has no established cause; the second failure identifies a process ownership mismatch in the verifier.

## Goals / Non-Goals

- Let only Kuru's explicitly requested independent memory service leave each short verifier command Job, then retire that service through Kuru's authenticated maintenance command before deleting the disposable root.
- Preserve exact public checksum, version, session, history, bundled-engine and receipt checks, command deadlines, and private fixture isolation.
- Name each bounded native command phase without exposing raw paths or credentials.
- Do not alter published v0.8.0 assets, Kuru's service supervisor, platform Job semantics, release workflow, archive size limits, or standalone delivery's dependency graph.

## Decisions

The verifier uses the existing `fixture_allow_independent_service` process option already exercised by staged Windows acceptance. Its Job still owns ordinary mise and Kuru command children; only an explicit native `IndependentService` request may break away.

After all evidence checks, the verifier invokes the same installed Kuru from the same isolated mise environment with the exact temporary `-C` project and `--data-dir` values and `memory purge --yes`. That command takes Kuru's maintenance permit and lifecycle leases before removing the disposable memory. Cleanup also runs after a verification failure if any Kuru memory command was attempted. Failed or uncertain cleanup retains the temporary root and reports both causes. A receipt declares cleanup confirmed only after the purge and root close succeed.

The delivery helper does not link `kuru-memory`; it only calls the already shipped CLI. Native regression tests exercise Job breakaway and the installed CLI's service retirement. A later release's post-public check remains the final end-to-end gate.

## Operational surface

This is the existing Windows runner's post-public Release job. It uses the workflow-pinned native mise executable, the independently verified published Windows Kuru archive, and a fresh runner-local temporary project, APPDATA, mise data, Kuru data, and offline Dolt cache. There is no listener or new bind address, secret, mount, workflow route, version pin, architecture, or runner image change. The SHA256SUMS, signed version commit, and native package checks remain authoritative.

## Integration contract

`MiseInstall` continues to construct owned mise commands through `kuru-platform` and parse the installed CLI's existing JSON responses. The only new process option permits explicit service breakaway, matching the existing staged Windows fixture. Cleanup calls the installed Kuru `memory purge --yes` against the verifier's exact isolated project/data roots; it does not infer a project from cwd or user configuration. Existing receipt fields and public asset identifiers do not change. Phase labels are fixed internal names, not untrusted command output.

## Risks / Trade-offs

Permitting a service to outlive a short command requires an explicit retirement step. The verifier therefore retains its private root if that step fails, allowing a diagnostic without deleting an uncertain owner's files. The public verification may still uncover a separate cold-start readiness failure; this change does not loosen its deadline or claim to explain the first v0.8.0 attempt.
