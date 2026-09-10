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
- `packages/kuru-core`: framework/configuration types and SQLite storage.
- `packages/kuru-connectors`: inference providers, tool host, MCP and outbound A2A.
- `packages/kuru-runtime`: actor pool, peer routing, relationships, dreaming and A2A ingress.
- `packages/kuru-delivery`: native updater, shell bootstrap, archive packaging, release and repository tooling.
- `scripts`: small shell entrypoints for source installation and commit checks.
- `docs`: user and contributor documentation.
- `openspec`: cospec-managed change artifacts and durable capability specs.

Keep provider and protocol knowledge behind connector interfaces. All crates
inherit workspace dependencies; pin exact versions and update Cargo.lock in the
same change. Preserve unknown model/effort capabilities returned by providers.
Do not concatenate unrelated private part histories into prompts. Retire parts
by archiving and preserve reversibility and role coverage.
Keep peers equal: cross-peer routing and relationship memory belong to the
runtime, not a model acting as a permanent supervisor. Frameworks are switchable
profiles; provider, authentication and protocol additions belong behind their
existing boundaries.

Dolt is the chosen next storage backend, in a separate PR after the initial
release and before production adoption. That transition includes versioned memory
updates and dreaming history, installation, and migration of development data.
The current implementation still uses SQLite until that follow-up lands.

Convention priority is: (1) the apps/ and packages/ monorepo structure,
(2) mise's native monorepo task model, (3) Rust, (4) other tools. Each app or
package owns its mise.toml and tasks. Root tasks aggregate or forward by mise
address; they must not grow a parallel task workspace. Cargo's root workspace
exists for shared Rust dependency resolution and combined coverage. Other
language manifests belong to their owning app/package only when necessary.
VitePress's Node/npm dependencies are local to apps/kuru-docs. Do not introduce
Python, Bun, or a root JavaScript/Python project for delivery helpers.
Scope tools needed only by particular tasks to those package tasks, as the
delivery package does for release-note tooling. Installation, packaging and docs
tasks must remain independent of the notes toolchain. Add new language tooling only
when necessary or clearly valuable, within its owning app or package.
The compiler-free release bootstrap lives in `packages/kuru-delivery/support`;
the root installation script forwards binary options and retains source builds.
Keep bootstrap behavior tests and shell lint in the delivery package's mise tasks.

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
Documentation builds and link/content checks are part of `check`; keep private
verification records and local evidence outside the published app directory.
Run `mise run check` before every commit. It includes meaningful behavioral tests
and a 90% workspace line coverage gate. Do not exclude application modules or
lower the threshold to make coverage pass. Test observable state and contract
failure modes, including live subprocess/HTTP fixture interactions.
Keep fixtures isolated, drain subprocess output, and bound waits with useful
failure diagnostics. Commands targeting a repository must clear inherited Git
repository-selection variables before applying deliberate command overrides;
changing the working directory alone does not isolate commands run from hooks.
Test foreign-repository environments in child processes, never by changing the
test runner's global environment. Verify terminal behavior with real PTYs and
inspect visual changes at practical terminal/browser sizes. Synchronize terminal
assertions with completed frames. Record live and local evidence separately;
a successful build is not evidence of a successful deployment.

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

Build and publish docs only through the final `build-docs` and `deploy-docs` jobs
inside that Release workflow, after release publication succeeds and from the
exact released commit. For recovery, rerun those jobs on the existing release
run. Do not add or dispatch a standalone Pages workflow, including for initial
site setup; merging a PR is not a docs deployment trigger. CI may build and
validate a preview without publishing it.

## Documentation and security

Update user documentation with command, configuration, protocol and workflow
changes in the same change. Keep changing operational detail in its owning docs
page and link to it here rather than duplicating versions or test counts.

When delegating work, give intent, goals, non-goals, constraints, owned files and
observable acceptance criteria. Coordinate shared-workspace writes and integrate
the results through the same cospec, review and verification flow.

Never read, print, copy or commit provider credential stores. Codex owns its
login lifecycle; API keys come from configured environment variables. Do not
load local secret files to diagnose authentication. Tests use fake secrets and
isolated stores. File operations remain under the tool root; shell is explicit
process authority and must not be described as sandboxed. State directories
are never tool roots. Avoid external messages, releases or remote publication
unless explicitly requested.
