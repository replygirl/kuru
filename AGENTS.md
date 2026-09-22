# Kuru

Kuru is a Rust chat harness organized around persistent equal parts, their
relationships and isolated memories. The deterministic runtime enforces budgets;
no model is the pool's supervisor. IFS, polyvagal, Freudian and Jungian modes
are computational interpretations, not clinical treatment or claims of sentience.

Use [aligned-team/cospec](https://github.com/aligned-team/cospec) as the minimum
standard for repository discipline. Follow Kuru's architecture and task ownership
when applying that standard. AGENTS.md is canonical; CLAUDE.md imports it with
`@AGENTS.md`. Keep this guide accurate when repository conventions change.

## Layout and boundaries

- `apps/kuru-tui`: `kuru` binary, terminal rendering and command line.
- `apps/kuru-docs`: curated public documentation site and theme.
- `packages/kuru-core`: framework/configuration types and shared contracts.
- `packages/kuru-memory`: managed Dolt, private SQL lifecycle, versioned storage and legacy import.
- `packages/kuru-platform`: checked native filesystems, owned process primitives and Windows private IPC.
- `packages/kuru-archive`: bounded archive codecs, independent of filesystems, targets and installation policy.
- `packages/kuru-connectors`: inference providers, tool host, MCP and outbound A2A.
- `packages/kuru-runtime`: actor pool, peer routing, relationships, dreaming and A2A ingress.
- `packages/kuru-delivery`: native updater, shell bootstrap, archive packaging, release and repository tooling.
- `scripts`: small shell entrypoints for source installation and commit checks.
- `docs`: user and contributor documentation.
- `openspec`: cospec-managed change artifacts and durable capability specs.

Keep provider and protocol knowledge behind connector interfaces. All crates
inherit workspace dependencies; pin exact versions and update Cargo.lock in the
same change. Preserve unknown model/effort capabilities returned by providers.
Kuru owns the complete agent runtime. OpenAI integration must use native
authentication and inference transports: ChatGPT browser OAuth (with device flow
where useful), or `OPENAI_API_KEY`. Do not embed the Codex CLI/app-server, require
another harness for login, or delegate Kuru's agent loop to another harness.
The `codex` provider label selects direct ChatGPT subscription access; `responses`
selects the explicit API-key route. Keep these routes separate across refresh,
relogin and request failures. Bundled native runtime dependencies such as Dolt
remain a separate portability requirement.
Browser sign-in hands its URL to the desktop through the platform boundary;
the user's browser lifetime must not become an owned process tree that Kuru kills.
Keep authentication corrections focused on login, credentials and direct provider
requests. Runtime evaluations and peer-behavior changes are separate work; auth
and transport still require complete functional verification.
Do not concatenate unrelated private part histories into prompts. Retire parts
by archiving and preserve reversibility and role coverage.
Keep peers equal: cross-peer routing and relationship memory belong to the
runtime, not a model acting as a permanent supervisor. Frameworks are switchable
profiles; provider, authentication and protocol additions belong behind their
existing boundaries.

Automatic ancestor configuration must pass exact-root workspace trust preflight
before its configured memory, provider, tool or protocol authority activates.
Ancestor/root `AGENTS.md`, `CLAUDE.md` and bounded relative Markdown imports
are captured once as repository-origin prompt authority under that review;
over-cap sources are omitted whole with visible notices. Nested subtree
activation remains a separate follow-on.
Capture all invocation overrides in one immutable configuration snapshot with
final-leaf provenance; retain the reviewed directory through configured launches.
Persistent approval binds the complete authority manifest, while a one-invocation
grant covers only that command's applicable claims. Keep pure inspection and
fixed account operations independent of unrelated authority. See
[workspace trust](docs/configuration.md#workspace-trust).
Nested project instructions activate only when an actor reaches an individually
authorized file path. Review their new complete manifest through the separate
workspace-trust choice before exposing a result or effect; a newly instructed
mutation must return for actor replanning before changing the file. Direct
`kuru tool` commands do not activate actor prompt instructions.

Full Dolt is the live memory backend. Keep its pinned runtime, provisioning and
actual database fixtures in `kuru-memory`; SQLite is only a read-only migration
dependency. Preserve original legacy data and validate imports before activation.
Each canonical project owns its revision history. Actor work carries an explicit
live or candidate view; every dream write stays on its candidate until validated
promotion. Undo adds a compensating revision and preserves later conversations.
Keep transactions short, reconcile uncertain writes before further mutation, and
publish in-memory topology/configuration only after persistence. Never kill a
process or delete a held lock based on a stale PID or occupied port. The owned
supervisor must reap Dolt before releasing its directory or lifecycle lease.
Writable opens require ownership; attached inspection handles never control the
owner's lifetime. Await command cleanup before releasing the project writer lease,
and hold the stable lifecycle lock through migration/recovery directory moves.

Every ordinary Kuru executable embeds its target's verified full-Dolt archive
and upstream licenses. `kuru-memory` owns the authoritative asset manifest,
local-only build verifier and `bundle:prepare` mise task; the independent
`kuru-delivery` helper prepares bounded, checksum-verified build inputs without
compiling memory. Cargo selects by `TARGET` and fails missing or corrupt inputs;
never add a host fallback or an unbundled build. Runtime provisioning extracts
the embedded bytes and must not download an engine. Keep build-input mirrors
separate from runtime memory settings, route source builds through package-owned
mise dependencies, and verify packaged cold offline conversations after install
and update. See [bundled build inputs](docs/development.md#bundled-engine-build-inputs).
Cached native test jobs keep private bundle preparation directories outside
archived Cargo target caches. They select a runner-temporary mirror through
the existing environment override so each runner creates its own permissions;
do not weaken private-object validation to accept restored directory grants.

Convention priority is: (1) the apps/ and packages/ monorepo structure,
(2) mise's native monorepo task model, (3) Rust, (4) other tools. Each app or
package owns its mise.toml and tasks. Root tasks aggregate or forward by mise
address; they must not grow a parallel task workspace. Cargo's root workspace
exists for shared Rust dependency resolution and combined coverage. Other
language manifests belong to their owning app/package only when necessary.
VitePress's Node/npm dependencies are local to apps/kuru-docs. Do not introduce
Python, Bun, or a root JavaScript/Python project for delivery helpers.
Backend aliases needed by root task discovery may be registered in root mise
configuration; keep their version pins and installation tasks in the owning
app/package. The npm alias must resolve to the same backend as the docs lockfile.
Scope tools needed only by particular tasks to those package tasks, as the
delivery package does for release-note tooling. Installation, packaging and docs
tasks must remain independent of the notes toolchain. Add new language tooling only
when necessary or clearly valuable, within its owning app or package.
The compiler-free release bootstrap lives in `packages/kuru-delivery/support`;
the root installation script forwards binary options and retains source builds.
Keep bootstrap behavior tests and shell lint in the delivery package's mise tasks.
The standalone Windows PowerShell bootstrap may use a minimal audited `Add-Type`
Win32 bridge for native identity, ACLs and durable replacement before Kuru can be
trusted or executed. Use only stock PowerShell/.NET facilities; add no separate
compiler installation, language project or downloaded runtime. Native bootstrap
tests must exercise that bridge. This exception does not relax Rust consumer
unsafe-code rules or create an alternative application platform implementation.
Owned launches that explicitly select stock Windows PowerShell must remove an
inherited `PSModulePath` before startup, allowing that edition to reconstruct its
standard module paths. A PowerShell 7 parent can otherwise break stock cmdlet
autoloading through mise or Rust. Keep this policy at the owned launch sites;
do not strip deliberate module settings from generic configured commands or MCPs.
Before its user command, the built-in ToolHost stock shell initializes its
shipped `Microsoft.PowerShell.Management` and `Microsoft.PowerShell.Utility`
modules from `$PSHOME`; ordinary module autoloading remains available. This
ToolHost-specific bootstrap does not apply to generic configured commands or
MCPs.

Portability and bundled runtime dependencies are product requirements. Installing
Kuru must be sufficient to run it: ship required native runtime engines and their
licenses with the application instead of asking users to install them or depending
on a first-run download. Keep build-time preparation explicit in package-owned
mise tasks. Each supported OS needs native CI that exercises actual memory,
process cleanup, terminal interaction, installation and updating; compiling an
executable or skipping platform tests does not establish support. Put OS-specific
mechanics behind small shared boundaries and preserve privacy, ownership and
recovery guarantees when porting them. Document support only after native checks
demonstrate it.

Keep platform mechanics independent of domain packages. Windows process creation
and private asynchronous IPC live behind safe APIs in `kuru-platform`; filesystem
operations retain handles and distinguish rejected from uncertain publication.
Unix process-group operations retain the owned child through signal-before-reap
and never authorize termination from a reaped numeric identity.
Only its audited Windows interop modules may locally allow unsafe code under the
package's deny-by-default policy. Every consumer retains the workspace prohibition.
Platform CI proves those primitives; application support additionally requires the
database, terminal, connector and delivery checks described above.
Keep archive record parsing in `kuru-archive`; domain packages own exact payload
inventories, hashes and private staging. Test physical duplicates and malformed
records, including the prepared upstream Windows engine archive on every host.
Route concurrent Windows child creation through the platform process API. Its
explicit handle list protects each child, but an unrelated legacy spawn can still
inherit temporarily inheritable handles; Rust's private spawn lock cannot be
coordinated by this library. Audit new process-launching dependencies and consumer
call sites instead of claiming isolation across arbitrary spawn mechanisms.

Use current available dependency and tool releases, verify compatibility, and
commit exact pins with the affected Cargo, npm and mise lockfiles. Pin workflow
actions by commit SHA. Do not loosen pins or remove verification to cure drift.
Follow the lockfile and provenance refresh procedure in [development](docs/development.md).

## Development

For full maintainer setup, use `mise install`, then `mise run setup`; platform
requirements and build-only installation are in [development](docs/development.md)
and [installation](docs/install.md). All routine commands run through mise:
`build`, `build:release`, `run`, `format:fix`, `format:check`, `lint`, `test`,
`typecheck`, `coverage`, `test:install`, `lint:tooling`, and `check`.
Use `docs:dev`, `docs:build`, `docs:preview` and `docs:check` for the docs site.
Package-scoped work uses native mise addresses, for example
`mise run //packages/kuru-core:test`. Keep root tasks as aggregates or forwards.
Run the relevant granular checks before committing. hk runs independent format,
lint, typecheck, tooling, cospec, docs and coverage steps concurrently before a
push; CI gives static categories separate Ubuntu jobs and runs native behavior,
installation and updates on their supported platforms. Keep these scheduling
units explicit instead of invoking `check` from hooks or workflows. The optional
local `mise run check` aggregate uses the same task dependencies. Coverage already
runs the behavioral suite; do not require an ordinary test pass before repeating
it under instrumentation. Preserve the 90% workspace line coverage gate.
Documentation builds and link/content checks are required; keep private
verification records and local evidence outside the published app directory.
Do not exclude application modules or
lower the threshold to make coverage pass. Test observable state and contract
failure modes, including live subprocess/HTTP fixture interactions.
Keep fixtures isolated, drain subprocess output, and bound waits with useful
failure diagnostics.
Test foreign-repository environments in child processes, never by changing the
test runner's global environment. Verify terminal behavior with real PTYs and
inspect visual changes at practical terminal/browser sizes. Synchronize terminal
assertions with completed frames. Record live and local evidence separately;
a successful build is not evidence of a successful deployment.
Instrumented child fixtures must explicitly retain the runner's LLVM_PROFILE_FILE
destination when clearing their environments, so their coverage is collected and
profile files do not appear in source or private-state fixture directories.
Cargo may hard-link its executable outputs. Treat explicitly selected build
artifacts as read-only inputs: retain their identity, bound and verify the bytes
being copied, and check the source name again. Keep private cache, installation
and publication-destination checks strict; accepting a build input does not grant
permission to modify its other links. Repository text must retain LF checkout
semantics so native Windows shell and generated-file checks see identical bytes.
Ordinary real-memory test tasks use the memory package's prepared supervisor
snapshot: concurrent Cargo commands can replace their top-level executable
aliases while another package's tests are running. Keep this preparation in the
owning mise task. Combined coverage must use its own instrumented supervisor,
with the ordinary snapshot opt-in explicitly cleared; never substitute an
uninstrumented executable to make a coverage run pass. Its prerequisites prepare
only verified bundle inputs; the instrumented fixtures provision their engine
cache, without first compiling or snapshotting an ordinary supervisor. Independent
checks may overlap, but do not start competing coverage writers or duplicate live
test suites. Preserve package-owned preparation and Cargo's artifact locking.

Use conventional commits. Never bypass hk hooks. Do not commit directly to
main; use a branch and review. Publishing and release tags are external actions
and require authorization. Local source installations and deterministic tests
need no additional permission when part of an authorized change.

## Cospec workflow

Invoke cospec through `mise run cospec -- ...` (or `mise run //:cospec -- ...`
from a package). The task supplies required compatibility configuration for the
standalone binary; bare `cospec` and `mise exec -- cospec` omit it. See
[development](docs/development.md) for the upstream issue and removal condition.

1. Start each substantive change with `mise run cospec -- new <type> <slug>`.
2. Read `mise run cospec -- instructions <artifact> --change <slug>` and author
   the artifacts allowed by that type. Do not add a generic design/spec bundle
   to a small ci/chore change. Maintain `blocking-changes.md` and declare affected surfaces;
   author required acceptance evidence before implementation.
3. Run `mise run cospec -- validate <slug> --strict`, then
   `mise run cospec -- apply <slug> --json`. Read its context files and obey the
   actual gate: 0 is clear, 1 is an error, 2 is blocked, 3 requires resolving or explicitly
   acknowledging the reported soft blockers. Never infer a pass from files.
4. Implement the scoped tasks and record observed evidence as checks finish.
   Name unrun checks and reasons explicitly; never manufacture runtime passes.
5. Complete tasks, validate, and run `mise run cospec -- archive <slug>` before
   the final branch commit and merge. Confirm the archive actually exists;
   never merge a completed change while leaving its record active on main.

Do not invoke bare OpenSpec, manually move archive directories, or edit generated
files in `openspec/schemas`, `.claude`, `.agents`, `.codex` or `.opencode`.
Regenerate with `mise run cospec -- update`; validate managed drift with
`mise run cospec:managed:check`. Keep Claude Code, Codex and OpenCode integrations
enabled. Preserve the distinction between cospec-generated files and AGENTS.md,
which is authored directly.

## Releases and documentation

`.github/workflows/release.yml` is the single publication entrypoint, through
manual dispatch on main with only the bump strategy as input. Recover interrupted
runs through automatic reuse of their exact version commit and release; keep
reruns anchored to the original dispatch SHA and published assets immutable.
Follow [release operations](docs/release.md) for the
conventional version calculation, signed app commit, exact-SHA checks, verified
archives and Communiqué notes. Preserve the scoped credential names documented
there; no credentials belong in source or generated artifacts.

Build and publish docs only through the `build-docs` and `deploy-docs` jobs inside
that Release workflow and from the exact selected release commit. The staged
Windows candidate acceptance and documentation deployment must succeed before
the publication job can promote the public release. A separate post-publication
Windows job verifies the immutable public download against the exact released
commit and retains its acceptance receipt. Its failure is reported on the run;
it never changes or unpublishes the release. For recovery, rerun the failed
jobs on the existing release run; Pages deployment and GitHub release promotion
are ordered but are not one atomic service operation. Do not add or dispatch a
standalone Pages workflow, including for initial site setup; merging a PR is not
a docs deployment trigger. CI may build and validate a preview without
publishing it.

## Documentation and security

Update user documentation with command, configuration, protocol and workflow
changes in the same change. Keep changing operational detail in its owning docs
page and link to it here rather than duplicating versions or test counts.

When delegating work, give intent, goals, non-goals, constraints, owned files and
observable acceptance criteria. Coordinate shared-workspace writes and integrate
the results through the same cospec, review and verification flow.

Never inspect, print, copy or commit a user's provider credential stores while
developing or diagnosing the application. Native login may manage only Kuru's
own private authentication store; never import another harness's credentials.
API keys come from configured environment variables. Tests use fake secrets and
isolated stores; live login requires the user's participation. File operations remain under the tool root; shell is explicit
process authority and must not be described as sandboxed. State directories
are never tool roots. Avoid external messages, releases or remote publication
unless explicitly requested.
