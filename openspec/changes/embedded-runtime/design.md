## Context

The Dolt foundation owns exact archive/payload hashes, bounded extraction, private
cache activation and an exact-version probe. Delivery already builds independently
of memory and owns native download and packaging helpers. See proposal.md for the
product contract; native-windows extends it to a fifth target separately.

## Goals / Non-Goals

**Goals:** keep the single executable installation/update contract, preserve the
current provisioning integrity checks and make build-time networking explicit.

**Non-Goals:** change Dolt SQL behavior, vendor large binaries into Git, add another
language, introduce runtime downloads or make Cargo silently build a partial app.

## Decisions

1. Memory owns one target asset manifest and a `bundle:prepare` mise task. A narrow
   delivery helper prepares hash-addressed archives without compiling memory.
   This avoids a bootstrap cycle and avoids a second root task workspace. The
   build helper bounds bytes and verifies content before atomic publication;
   explicit local archive import and offline reuse use identical verification.
2. Memory's build.rs selects by Cargo TARGET and only reads/verifies prepared
   local bytes, copying them to OUT_DIR for include_bytes. It never downloads or
   executes a target binary. Missing/wrong/corrupt input fails with the exact mise
   preparation command. Reject build-script networking because it hides a build
   dependency and makes offline behavior implicit. Source installation invokes
   mise preparation; direct Cargo users prepare the same input once.
3. Every standard executable includes the original compressed archive, including
   LICENSES. Provisioning extracts these bytes into the existing private cache
   under its lock, verifies payloads, probes the version and activates atomically.
   Reject companion sidecar archives because mise/direct copy/self-update must
   each install the complete runtime by replacing one executable. Keep an explicit
   exact-version development binary override, never a PATH fallback.
4. Remove memory runtime HTTP provisioning. Retain `memory.offline` compatibility
   while making it unnecessary for the standard engine path. Cold offline cache
   creation must succeed; corrupt existing cache must still fail closed.
5. Build, run, check, coverage, source installation and native release tasks
   prepare the matching target through native mise dependencies. Explicit target
   builds prepare that target, never silently use a host archive. Build-only
   mirror configuration is separate from runtime user settings.
6. Exercise the packaged application, not only its version command: fresh empty
   offline memory/cache, demo conversation, reopen, inspect history/licenses,
   then installer/update roundtrip and another fresh-cache launch. Existing four
   targets run this contract; native-windows adds the fifth under its own gate.

## Risks / Trade-offs

- [Larger executable] → keep original compressed payload (~40 MB) and measure
  actual outer archives against current bounds on native CI; do not blindly
  increase limits.
- [Prepared input tampering] → verify exact size/hash both at preparation and
  build, reject unsafe input files, and use deterministic immutable addressing.
- [Cross-target mismatch] → select using TARGET, fail missing inputs explicitly,
  test target selection independently of the host.
- [Extraction cancellation] → retain staging ownership in the blocking worker
  until it exits; preserve existing activation, lock and failure regressions.
- [Offline source setup] → document importing the pinned input in addition to
  ordinary Rust dependency preparation; end users receive it inside Kuru.

## Operational surface

Preparation runs on developer or native CI hosts and downloads public pinned
upstream archives over HTTPS; it needs no new secret. Cargo itself reads only
local prepared inputs. Existing four macOS/Linux target pins retain Dolt 2.3.3;
the dependent Windows change adds its separately verified x64 asset. Runtime
SQL remains authenticated loopback under the existing owned supervisor and
connection bounds; embedding adds no listener, container or background service.
Build-only mirrors never become runtime download endpoints or user settings.

## Integration contract

The memory-owned manifest carries target, version, URL, compressed/expanded size,
archive digest, executable digest/size and license digest/size. Delivery consumes
the bounded build-input fields; memory retains archive layout and execution policy.
Preparation and build both validate the selected immutable archive. Runtime uses
exact upstream bytes and retains its strict layout validator and engine probe.
Fixtures use actual pinned archives plus mutated copies; no SQL schema, project
identity, authentication or provider contract changes.
