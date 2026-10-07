# Verification

## 1. Successive standalone calls reuse checked storage [critical]

- [x] 1.1 @e2e (agent) run three completed real standalone CLI conversations against one isolated project -> actual final task58536 exited0; case1/1 in37.57s using the product default30, with no retained inspection attachment across calls. Authenticated service generation and native instance/port remained equal across three completed demo conversations; all their distinct public sessions survived restart.
- [x] 1.2 @e2e (agent) await the actual final idle retirement then launch again -> same actual case awaited owner release after idle expiry, checked service/native endpoint absence, then completed another conversation on a different generation. Observed native instance647bb50e-e525-4e3b-bed3-8499402d63c8/port54647, elapsed release observation31.434s including subsequent startup, successor generationd4b7f8c9-0344-4607-af32-bb68b1a38f03; ServiceCleanup::release completed, app closing guard1/1 .05s.

## 2. Idle policy preserves owned lifecycle [critical]

- [x] 2.1 @integration (agent) observe existing serve events through detach, gap, reattach and actual bounded idle interval -> task98576 exited0, new owner event case1/1 in3.50s: active clients answered beyond the interval, reattachment invalidated the old empty deadline, and final idle expiry reaped the owner and released discovery/authority. Existing bounded transport tasks retain ownership through their handshake, including a rejected hello; only authenticated admission marks the starter reached.
- [x] 2.2 @integration (agent) exercise existing maintenance, shutdown race and ordered-close fixtures -> task14746 exited0, five selected existing cases5/5 in9.99s. Product30 retention did not delay explicit maintenance; live-peer refusal, retained election gate, ordered reap/endpoint release, shutdown-racing successor and explicit zero-interval two-client retirement assertions all passed.
- [x] 2.3 @regression (agent) validate config and internal launch arguments including legacy optional forms -> the fifth14746 case checked legacy forms, starter token/exact hello bytes and idle0/30/300 plus invalid values; core task90684 exited0, schema/parser parity and existing shared-bound tests2/2 in.04s. Invalid negative, noninteger and greater-than300 values were refused.

## 3. Repository and portability checks

- [x] 3.1 @regression (agent) run affected host/Windows lints, typechecks, format, docs and managed checks -> task31648 exited0 across11 affected core/memory/app/delivery static tasks (host lints, memory/app/delivery Windows lints, all-target/all-feature types). Final fixture isolation and documentation refinements were checked by17715: app host/Windows lint, typecheck, docs build/content and format all0. Managed99487 exited0/no drift. Windows results are compilation only.
- [~] 3.2 @runtime (agent) run hosted full native CI after normal publication -> defer: PR publication belongs to root after reviewed archived handoff; no local native Windows/Linux execution is claimed

## Evidence limits

Native acceptance above ran on macOS aarch64 with the package-owned verified
bundle/supervisor preparation, healthy isolated shared cache and FD4096. No paid
provider calls, latency benchmark or live account was used. Same authenticated
service generation plus unchanged native endpoint instance/port establishes the
retained engine binding; no numeric process identity grants cleanup authority.
Reap-before-owner-release is also covered by the unchanged ordered-close case.
Existing memory/CLI lifetime fixtures and the deliberate cold-open measurement
select zero explicitly; product default policy remains30 in all builds.

Initial standalone compilation found a LowerHex formatting incompatibility and
was corrected before execution; subsequent selected runs passed. Initial sandbox
cache/SystemConfiguration and multi-task syntax errors were invocation failures,
not runtime test results. No source pins, locks, persistent tool/trust settings,
production RPC/admission policy or existing deadlines were changed.
