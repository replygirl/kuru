# Verification

## 1. Doctor reports safe, separate local observations [critical]

- [x] 1.1 @integration (agent) run the typed CLI against synthetic Kuru-native signed-in and signed-out subscription status, selected/unselected routes, valid/invalid/unreadable configuration -> fixed human and JSON output distinguishes the states without credentials, account identifiers, configuration values, or private paths
- [x] 1.2 @integration (agent) run child processes for an unapproved repository Responses route and for the same route with `--trust-workspace-once` -> the unapproved case remains unverified without environment lookup; the explicit one-invocation grant reports only fake-variable presence and persists no approval or data
- [x] 1.3 @manual (agent) review the doctor dispatch and @integration (agent) run isolated auth/config/route fixtures -> the code path reads local Kuru auth status and authorized environment presence only; it does not construct a provider request, refresh credentials, or fall back between routes

## 2. Doctor memory inspection never activates project state [critical]

- [x] 2.1 @integration (agent) run fresh, valid, invalid-activation, absent-owner, and existing-owner fixtures -> structural outcomes are bounded; malformed activation is classified; live inspection uses one read-only attachment to the published owner
- [x] 2.2 @e2e (agent) compare project state and owner lifecycle around doctor child processes -> fresh and corrupt state remain unchanged, no service/tools are created, a live owner remains usable after doctor closes its view, and final owner close reaches managed quiescence
- [x] 2.3 @integration (agent) inspect a valid cold project with no published owner -> the activation marker is unchanged, no service or engine is started, and deeper SQL health is reported unverified

## 3. Public guidance matches current behavior

- [x] 3.1 @integration (agent) build and check the curated docs site and its local links -> `mise run //apps/kuru-docs:check` passed the VitePress build, lint, formatting, generated-site links, and anchors
- [x] 3.2 @manual (agent) reviewed the pages against current data-path, retention, purge, bundled-engine, shell-authority, and diagnostic behavior -> no expiry, secure-erasure, sandbox, full-package-size, or unavailable-recovery claim is present

## 4. Repository validation and deferred delivery evidence

- [x] 4.1 @integration (agent) run owning host/Windows lint, typecheck, formatter, managed-file, docs, and strict Cospec checks -> app and memory host all-target typechecks, app/memory host and Windows-target clippy, Rust formatting, docs validation, managed-file check, and strict Cospec validation all passed
- [~] 4.2 @runtime (agent) complete native supported-platform doctor/memory checks and the repository's hosted 90% coverage gate after archive -> defer: these required delivery checks run on the archived exact branch/PR head
- [~] 4.3 @manual (human) perform a live account/provider canary -> defer: no live account participation was requested; synthetic fixtures establish local behavior only
