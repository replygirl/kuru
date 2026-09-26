# Verification record

## Phase 1 foundation checks (2026-09-16)

The local regression suite covers typed content and durable migration,
provider-visible streaming with settled final answers, tool approvals and
workspace-bound grants, whole-request context fit, and durable session usage.
Real HTTP, process, terminal and Dolt fixtures exercise failures and cancellation
as well as successful requests.

Four-mode parity fixtures retain the built-in roles, instructions, recipients,
speaking decisions and namespace layout. Test-only alternate profiles exercise
all six policy boundaries through actual requests and storage: rejected peer
effects, retained mandatory input, restricted optional context, private namespace
selection and candidate-safe dreaming. Permission denial, exact retry, deferred
publication and undo preserving later history remain part of the same suite.

Observed local results are recorded in the archived change ledgers. Hosted CI
and release results are recorded separately; the dated release evidence below
retains its stated version and platform scope.

## Native CI and release candidates (2026-09-15)

Hosted CI runs separate quality jobs for format, Rust lint, typechecking,
repository/workflow tooling, Cospec validation, managed files, documentation,
and RustSec advisories. The final CI gate requires both quality and every native
verification category to succeed. Local reproduction and task ownership are
documented in [development](development.md).

Ubuntu x86-64 and macOS arm64 run instrumented native behavior with the 90%
workspace line-coverage gate. ARM Linux and Intel macOS have additional native
build, real-memory, packaging, and packaged offline-runtime checks. Windows
runs four parallel coverage shards: delivery/archive, application, memory/runtime,
and connectors/core/platform. Its aggregate requires every shard job to have
succeeded and a checked result for all four shards, taking each shard's latest
uploaded (successful) attempt within the workflow run. It refuses a missing
shard, a later or malformed attempt, or any source, tree, toolchain or
inventory difference between them, and enforces the same 90% workspace
threshold. A shard's test executables stop at a deadline inside its job limit;
a stalled one is terminated with its owned process tree and the shard fails
with a diagnostics artifact naming the unfinished tests.
A separate Windows job verifies platform primitives. On every OS, an
installation job verifies offline source installation and installed runtime
behavior, then has the previous published release's own updater install the
tree's release build; on Windows it also checks offline build-input failures
and shipping DLLs.
Native tests also cover terminal interaction, process cleanup, and self-update.

The release workflow validates its source and selected version commit, builds
all five native archives, and assembles one complete candidate. Native Windows
then installs the exact staged ZIP through the pinned mise backend with isolated
loopback release metadata, verifies its bundled engine, and exercises an offline
conversation and durable reopen. Linux x86_64, Linux arm64 and macOS arm64
verify their staged archive checksums, run the extracted executable's packaged
offline runtime check, and have the previous release's updater install it; they
do not yet exercise the mise route. Intel macOS has no staged leg yet. This proves staged installation and runtime
behavior; it is not a test of downloading the release from public GitHub.
Documentation builds in parallel with staged acceptance. Documentation deployment
must succeed before publication. A separate post-publication Windows job checks
the exact immutable public download through ordinary mise installation and a
cold offline conversation/reopen, retaining its cleanup-confirmed receipt as an
Actions artifact. Its failure is visible without mutating the release; retry
that job on the same release run. See [release operations](release.md)
for the complete ordering and recovery contract.

The following evidence applies to v0.3.2 at
`5b952cf08f04ea115082b4200b38478dfbc115c5`:

- [Merged-main CI](https://github.com/replygirl/kuru/actions/runs/34946268806)
  passed all quality and native gates, including Windows shard aggregation.
- [Release](https://github.com/replygirl/kuru/actions/runs/34948958911) passed
  all five package builds, staged Windows acceptance, documentation deployment,
  and final publication. The whole workflow completed successfully.

Native code signing remains deferred. These checks establish neither macOS
notarization nor Windows Authenticode, and older records below retain their
original verification scope.

## Native OpenAI authentication (2026-09-11)

Kuru completed a user-participating browser OAuth login, discovered eight
account-visible models and their effort settings, and completed a direct
GPT-6 Astra conversation using the resulting Kuru session. No development tool
inspected or copied credential files. The initial live request exposed a missing
Content-Type header; the parser now validates the bounded event stream when that
header is absent, while rejecting malformed, incomplete and falsely typed bodies.
CLI failures retain the provider's existing error instead of only the aggregate
peer failure. This is functional authentication and transport verification.

The connector suite passes 56 tests, including real HTTP browser/device flows,
concurrent refresh, private publication, logout fencing, actor tool continuation
and missing-header regressions. Installed and updated macOS executables complete
API-key model and conversation requests with an empty PATH, preserve bundled
offline memory, and expose an HTTP 403 without echoing a credential supplied by
the rejecting fixture. The complete local pre-push graph at db33c17 passed
405 tests and 96.752% workspace coverage, along with all seven static checks.

[CI at db33c17](https://github.com/replygirl/kuru/actions/runs/34621868783)
passed every static category and all five native application targets. Windows
passed 444 tests with 94.235% workspace coverage; its separate platform suite
passed 60 tests with 91.378% coverage, including actual browser error propagation
and COM cleanup. Source installation, missing/corrupt offline build-input controls,
installed offline conversations and self-update, and shipping Kuru/Dolt DLL
inspection passed. Ubuntu and macOS ARM workspace coverage exceeded 96%.

The final Windows run passed the original unobserved shell request and both
real-file lock controls: a retained lock must time out, and owned descendant
cleanup must permit actual reacquisition within a bounded observation. Original
process ownership, output and cleanup assertions remain intact. The OpenAI,
updater, plain shell and lock-observation changes are archived with their actual
native evidence. Historical observer-wrapped results and skipped steps from
failed runs are not counted as acceptance of the final implementation.

## Native delivery and public documentation (2026-09-09)

The complete package-owned mise gate passes **209 Rust tests** and **97.51%
line coverage** (8554/8772), including the CLI and native delivery tooling.
The 90% threshold remains unchanged. Rust replaces every Python installer,
validator and PTY/protocol fixture; Node/npm stays local to the VitePress app.
The source installer built and executed kuru 0.1.0 from an isolated destination,
and the installed demo provider completed an offline conversation.
The actual optimized macOS arm64 executable also completed a release roundtrip:
native packaging, checksum-verified installation into a temporary directory and
native self-update, followed by a successful version check.

Hosted Ubuntu found a copied-executable launch failure before the updater ran,
consistent with the Rust/Linux concurrent spawn and writable-descriptor race.
Updater fixtures now prepare independent executables in a dedicated Rust child
and wait for its exit before launch; no retry or timing delay masks the failure.
The preserved-version assertion follows the crate version so release verification
continues to work after a version bump.

The pre-push gate also exposed concurrent SQLite initialization contention.
A controlled WAL-transition regression failed immediately against the original
implementation and passes with a five-second retry limited to SQLITE_BUSY at
that transition. Deadline and non-busy errors, four independent initializing
processes, the existing eight-thread startup and corruption checks all pass.
The process fixture also exercises libtest's single-thread output format and
bounds startup readiness with captured diagnostics and child cleanup.

The release suite uses actual Cocogitto and Communiqué binaries with disposable
Git histories and local HTTP fixtures. It verifies scoped version stamping,
signed-commit payloads, immutable tags, incomplete-draft recovery, checksum
rejections, factual fixture notes and preserved reviewed output. No live release
or tag was created. The release app's actual permissions and scoped repository
access were verified; it bypasses only the PR/check ruleset.

Standalone cospec's duplicated embedded entrypoint required a narrowly scoped
compatibility preload. Tests verify a single JSON response and preserved clear,
hard, soft and missing-artifact gates, including a checkout path with spaces.
No external Bun or OpenSpec installation is required.

The docs production build passes native link, anchor, sitemap and public-content
checks. Chrome verification at 1440px and 390px covers light/dark, four framework
portraits, keyboard activation, motion preferences, local search, deep reloads
and mobile navigation. Seven screenshots were inspected; no overflow, clipping,
page errors or missing resources remained. [Hosted CI run 34393197392](https://github.com/replygirl/kuru/actions/runs/34393197392)
passed on macOS 14 and Ubuntu 24.04, including the full gate, source installation,
coverage/docs artifacts and ci-gate. Detailed evidence remains in the archived
change ledgers. Pages publication belongs to the final jobs of an authorized
Release run. The mistaken standalone dispatch (34397071952) was canceled with
zero deployments; it is not evidence of a live site.

## Hosted CI repair and portable repository layout

The first hosted main run failed in the Linux terminal fixture at shutdown.
A controlled one-second scheduling pause plus terminal resize reproduced the
same timeout locally. Fixtures now observe the current rendered draft and idle
state between commands and continuously drain terminal output through exit.
A separate real-child regression checks delayed output larger than the PTY
buffer (1 MiB) and bounded diagnostics for a stalled process.

The integrated repair passes `mise run check`: **148 Rust tests**, **11 installer
tests**, two Python PTY-driver regressions and **97.64% coverage** (6731/6894),
with all formatting, lint, strict-spec and managed-drift checks. The three PTY
flows retain chat, selectors, cancellation, persistence and terminal restoration.

The app now lives at `apps/kuru-tui`; all runtime source is unchanged. A direct
Cargo install from that path succeeded and the installed executable reports
`kuru 0.1.0`. `AGENTS.md` is canonical; `CLAUDE.md` imports it. Cospec generates
Claude Code, Codex and OpenCode integrations, with repeatable generation and no
drift. Hosted results remain tracked by the PR and main runs in GitHub Actions.

## Private GitHub repository preparation

The initial hosting preparation for [replygirl/kuru](https://github.com/replygirl/kuru)
passed `mise run check`: **147 Rust tests**, **11 installer tests**, and
**97.64% coverage** (6731/6894), plus lint, format, workflow and cospec checks.
Conventional PR-title positive/negative probes passed. A review of all 192 existing
historical blobs found no credential-shaped content; no local state or credential
artifacts were tracked. This is a scoped publication review, not a security audit.

The repository was created privately, with squash-only merging, automatic merged
branch deletion, issues enabled and unused wiki/projects disabled, matching the
cospec reference. Source includes a stable `ci-gate`, PR-title validation,
contributor/security guidance and authenticated private clone instructions.
Hosted results are available in [GitHub Actions](https://github.com/replygirl/kuru/actions).
No release tags or visibility change are part of initial repository creation.

## Quiet ambient correction

The follow-up motion correction passed `mise run check` on 2026-09-09:
**147 Rust tests**, **11 installer tests**, **97.64% workspace line coverage**
(6731/6894), and all lint, format, tooling and cospec checks.

Two new regressions failed against the prior implementation: typing changed
portrait/composer cells, and ambient time changed contour characters. Both now
pass across all four frameworks. Typing, paste and editing leave decoration
alone; timed frames retain every glyph and position, with RGB channels changing
by at most two levels per 250ms sample. The ambient cycle lasts 24 seconds.
Existing mode selector, persistence, busy indicators, focus and static-motion
checks pass. The earlier typing-ripple behavior described below is superseded.

Actual terminal-cell exports were generated and the quiet Freudian contour was
visually inspected. The existing cmux preview was refreshed with the same state
store. The full gate also exposed a PTY fixture waiting for a provider while it
stopped draining terminal output; the fixture now drains the PTY during that wait,
preventing startup backpressure from causing a false cancellation-test timeout.

## Living interface and persistent choices

The second 2026-09-09 interface pass passed `mise run check`: **145 Rust tests**,
**11 installer tests**, and **97.63% workspace line coverage** (6797/6962).
The explicit frame-cost profile is separate from those 145 tests and passed too.
Clippy with warnings denied, formatting, tooling invariants, strict cospec
validation and managed-file drift checks all passed. No dependencies changed.

- Four actual PTY launches verify slash-command and F2/F3/F4 choices, fresh-process
  restoration, cleared effort and a real SQLite rejection of the transaction's
  final write. Additional tests cover provider pairs, invocation precedence,
  canonical path/symlink scope, project isolation and session resumption.
- The terminal smoke test observes ambient output after the old four-second
  cutoff, no idle animation under the startup override, focus pause/resume,
  navigation, paste, resize, chat and restored terminal attributes.
- A delayed local HTTP provider and real terminal verify cancellation, busy
  settings feedback, preserved drafts, rejection of late responses and successful
  subsequent work. It reconstructs terminal cells for assertions because diffs
  omit unchanged letters and spaces.
- Nine visual integration tests and four scene tests check distinct portraits,
  truthful peer routes/relationships, fixed labels and caret during animation,
  compact controls, long future model/effort strings, selectors, Unicode editing,
  narrow bounds, styled replies and error states. Actual colored Ratatui cell
  exports were inspected for all four frameworks and interaction states.
- At 140×50, 200-frame unoptimized profiles measured **1.212 ms/frame** for the
  welcome, **2.256 ms/frame** for a 500-entry conversation and **2.726 ms/frame**
  with its mode picker open. These are local render measurements, not provider
  latency or cross-platform performance guarantees. Ambient scheduling is capped
  at 4 FPS; editing and active work at 12.5 FPS.
- The final OpenAI-backed build was relaunched in the existing cmux
  `Kuru · new design` surface using the same project and preview data store.
  Temporary visual-review browser tabs were closed. External provider completion
  behavior was unchanged in this pass; the live inference checks below remain
  the earlier baseline.

Review artifacts: `KURU_VISUAL_ARTIFACTS=/tmp/kuru-visual mise exec -- cargo test
-p kuru --test visual --locked`. The explicit profile command is documented in
[terminal design](interface.md).

## First visual update

The 2026-09-09 design update passed `mise run check` with **127 Rust tests**,
**11 installer tests** and **97.62% workspace line coverage** (5997/6143), including
Clippy, formatting, tooling checks and strict cospec validation. No dependencies
changed. Seven visual integration tests exercise all frameworks, real peer events,
relationship membership, route animation, reduced motion, Markdown, long selectors,
compact status and Unicode input. The real PTY test checks RGB output, F6, both
motion startup modes, normal chat and terminal restoration.

Actual Ratatui cell exports were visually inspected, and the OpenAI-backed build
was opened in the cmux `Kuru · new design` tab with isolated preview state.
`KURU_VISUAL_ARTIFACTS=/tmp/kuru-visual mise exec -- cargo test -p kuru --test visual`
regenerates HTML/text review artifacts from the actual renderer.

## Initial harness baseline

Validated locally on macOS arm64 on 2026-09-09 with the versions recorded in
[the dependency audit](dependencies.md).

- `mise run check` passed: **115 Rust behavioral tests**, **11 installer tests**,
  Clippy with warnings denied, Rust/TOML/Python formatting, ShellCheck, Ruff,
  Actionlint, repository invariants and cospec validation/drift checks.
- Whole-workspace LLVM line coverage: **97.66%** (4999/5119). No application
  modules are excluded. A separate cross-check omitting trailing `cfg(test)`
  modules and the test-support file measured **97.25%** (3469/3567) for production
  code. The enforced standard LLVM threshold is 90%.
- Real subprocess/HTTP fixtures cover Codex JSON-RPC, Responses native function
  replay, MCP stdio/Streamable HTTP, and A2A 1.0. These are deterministic protocol
  tests, not a claim that every external server/provider has been exercised.
- Native Codex 0.153.4 discovered eight account-visible models and current effort
  strings. A live GPT-6 Astra request completed under the restricted inference
  permission profile. A second live test ran the whole Freudian peer pool,
  executed Kuru's `file_read`, and returned the exact random file marker.
- Real PTY tests exercised chat, selectors, commands, resize, paste and clean
  terminal restoration. Actor cancellation tests verify provider-future teardown
  and permit recovery. The normal OpenAI app was also launched in a cmux pane.
- Source installation through mise and direct Cargo installation both succeeded
  in temporary prefixes. Installed binaries ran all four frameworks and resumed
  sessions. Release packaging, checksum installation and CLI self-update were
  exercised; corruption was rejected while retaining the prior executable.

Fresh interactive browser login was not repeated; live checks used the existing
supported Codex login. Login/device-login/status/logout command routing is tested
with a subprocess fixture. At the initial local baseline, hosted CI and release publication had not run;
the workflows were statically validated. Current hosted checks are linked above.

The [protocol documentation](protocols.md) lists the supported MCP/A2A subset.
Jungian collective memory is project-scoped. Framework profiles are computational
interpretations; these checks establish software behavior, not psychological or
clinical validity.
