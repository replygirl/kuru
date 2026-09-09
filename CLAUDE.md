# Kuru

Kuru is a Rust chat harness organized around persistent equal parts, their
relationships and isolated memories. The deterministic runtime enforces budgets;
no model is the pool's supervisor. IFS, polyvagal, Freudian and Jungian modes
are computational interpretations, not clinical treatment or claims of sentience.

## Layout and boundaries

- `apps/tui`: `kuru` binary, terminal rendering and command line.
- `packages/kuru-core`: framework/configuration types and SQLite storage.
- `packages/kuru-connectors`: inference providers, tool host, MCP and outbound A2A.
- `packages/kuru-runtime`: actor pool, peer routing, relationships, dreaming and A2A ingress.
- `scripts`: source/release installation, packaging and repository checks.
- `docs`: user and contributor documentation.
- `openspec`: cospec-managed change artifacts and durable capability specs.

Keep provider and protocol knowledge behind connector interfaces. All crates
inherit workspace dependencies; pin exact versions and update Cargo.lock in the
same change. Preserve unknown model/effort capabilities returned by providers.
Do not concatenate unrelated private part histories into prompts. Retire parts
by archiving and preserve reversibility and role coverage.

## Development

Use `mise install`, then `mise run setup`. All routine commands run through mise:
`build`, `build:release`, `run`, `format:fix`, `format:check`, `lint`, `test`,
`typecheck`, `coverage`, `test:install`, `lint:tooling`, and `check`.
Run `mise run check` before a commit. It includes meaningful behavioral tests
and a 90% workspace line coverage gate. Do not exclude application modules or
lower the threshold to make coverage pass. Test observable state and contract
failure modes, including live subprocess/HTTP fixture interactions.

Use conventional commits. Never bypass hk hooks. Do not commit directly to
main; use a branch and review. Publishing and release tags are external actions
and require authorization. Local source installations and deterministic tests
need no additional permission when part of an authorized change.

## Cospec workflow

Every substantive change starts with `mise run cospec -- new <type> <slug>`.
Read the relevant artifact instructions through `cospec instructions`, author
proposal/specs/design/verification/tasks appropriate to the change, then run
`mise run cospec -- validate <slug> --strict` and
`mise run cospec -- apply <slug>`. Obey the actual gate exit code. Author an
acceptance ledger before implementation and record observed evidence as checks
finish; no fabricated passes or silent deferrals. Complete tasks, validate and
archive with `mise run cospec -- archive <slug>` before the final branch commit.
Do not invoke bare OpenSpec, manually move archived changes, or hand-edit
`openspec/schemas` or `.claude` generated files. Regenerate those with
`mise run cospec -- update`; check drift with `cospec:managed:check`.

## Documentation and security

AGENTS.md delegates here to avoid duplicated instruction drift. Update these
instructions when repository workflow changes, and update user documentation
with commands/config/protocol behavior in the same change.

Never read, print, copy or commit provider credential stores. Codex owns its
login lifecycle; API keys come from configured environment variables. Do not
load local secret files to diagnose authentication. Tests use fake secrets and
isolated stores. File operations remain under the tool root; shell is explicit
process authority and must not be described as sandboxed. State directories
are never tool roots. Avoid external messages, releases or remote publication
unless explicitly requested.
