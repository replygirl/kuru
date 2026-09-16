## 1. Private-directory guidance [critical]

- [x] 1.1 @regression (agent) pre-create owner-owned Unix Dolt cache and versions directories with mode 0755, then enter real managed provisioning -> focused tests observed exact-path mode-0700 remedies for cache, versions, and the pinned install lock; modes and sentinel bytes remained unchanged and no runtime payload activated
- [x] 1.2 @regression (agent) pre-create unsafe cold-probe, install-destination, and private-home directories through the real extraction/probe preparation boundaries -> focused tests observed the eligible exact path at each boundary, unchanged mode and sentinel content, and no activation
- [x] 1.3 @integration (agent) exercise the shared Movable, Pinned, parent, and ensure-private composition plus the real server `LifecycleLease` boundary with owner-owned Unix mode-0755 directories -> focused tests observed the safe exact-path remedy, retained `PermissionDenied`, no lock creation, and no mode/content mutation without inferring unrun full server startup
- [x] 1.4 @regression (agent) exercise symlinked and foreign-owned controls at the shared memory filesystem boundary -> retained controls refused both paths without mode-0700 guidance or permission mutation
- [ ] 1.5 @equivalence (agent) run native Windows formatter controls plus existing native owner-private filesystem enforcement tests -> a synthetic permission-denied error retains store/migration native file-security guidance, the new Unix-only files formatter is an identity on Windows, existing ACL rejection remains enforced, and no path emits Unix chmod or mode-0700 advice

## 2. Cold-facing documentation

- [x] 2.1 @manual (agent) compare both architecture/framework pages with the stable-speaker spec and built-in role tables -> both pages name the four first authored identities, subsequent authored order, exact selection precedence, `mode-authored-order`, `stable-id-order`, and the no-authority guarantee; rendered output contains the complete policy and introduces no focus control
- [x] 2.2 @integration (agent) build and content-check the documentation site -> `mise run //apps/kuru-docs:check` passed formatting, lint, the 12-page VitePress build, generated text artifacts, and local link/anchor validation after app-owned formatting

## 3. Static and behavioral compatibility

- [x] 3.1 @integration (agent) run focused memory format, typecheck, lint, and private-directory/provision/server tests -> Rust formatting, diff-check, all-target memory typecheck, strict all-feature memory lint, and seven focused tests passed with no platform-policy or dependency change
- [ ] 3.2 @integration (agent) run focused core/runtime cold-facing selection tests -> existing behavior and reason strings remain unchanged

## Observed evidence

- Seven serial `mise run //packages/kuru-memory:test -- <filter>` invocations each passed one test with no failures: the new shared OwnerOnly boundary, actual cache/versions/pinned-lock, probe/install/home and lifecycle regressions, plus the retained legacy, ordinary-open and linked/foreign controls.
- Independent source review confirmed every changed production OwnerOnly open/create path composes through `kuru-memory::files` while preserving its `Movable` or `Pinned` retention, all Inherited reads remain unchanged, the original I/O error remains downcastable, and no platform-layer code or dependency changed. Strict Cospec validation and `git diff --check` passed.
- `mise run //packages/kuru-memory:typecheck` passed all targets after package-owned bundle preparation; `mise run //packages/kuru-memory:lint` passed all targets and features with warnings denied. Repository Rust formatting and diff-check passed.
- `mise run //apps/kuru-docs:check` passed formatting, lint, a 12-page VitePress build, generated `llms.txt` artifacts, and local link/anchor checks. Rendered inspection found the complete cold-facing priority, reason strings, and no-authority text in the framework output.
- Windows formatter controls are present in source but remain unverified until native CI; verification 1.5 stays open.
