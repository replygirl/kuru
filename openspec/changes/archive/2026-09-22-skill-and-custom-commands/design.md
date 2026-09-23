## Context

P01 stores one immutable base `ConfigSnapshot` and an exact-root v2 approval with path-qualified complete supplemental manifests. P01c's actor ToolHost admission can return an approved prompt update to the runtime, which installs it before the next inference. P02a has a typed built-in command registry; its help, completion and dispatch currently use only static entries. The [Agent Skills specification](https://github.com/agentskills/agentskills/blob/main/docs/specification.mdx) defines YAML `name`/`description` frontmatter and progressive metadata, body and resource disclosure. Its experimental `allowed-tools` field is descriptive input to Kuru, never a permission grant.

## Goals / Non-Goals

**Goals:** Keep project prompt bytes source-bound from capture through foreground review and provider use; make catalog order and collisions deterministic; load a full skill body or reference only when selected.

**Non-Goals:** Execute skill scripts, import marketplace plugins, grant file/shell/MCP permissions from a skill, or add another trust enrollment store.

## Decisions

### Discovery and precedence

Use `<user-config-dir>/kuru/skills/<name>/SKILL.md` and `<root>/.agents/skills/<name>/SKILL.md`; `<user-config-dir>/kuru/commands/<name>.md` and `<root>/.kuru/commands/<name>.md` hold custom prompt entries. The CLI passes its resolved user configuration directory into core capture even if `config.toml` does not exist. Valid names are lowercase ASCII letters, digits and single interior hyphens, matching the skill directory and command filename. Sort names bytewise. A project entry wins an equal-named user entry; a built-in slash command always wins an equal-named custom command. Shadowed project entries are inert and do not contribute authority claims. Reject case/identity ambiguities and report bounded omission counts before presenting an incomplete catalog. Alternative: recursively scan arbitrary ancestor or plugin directories. Rejected because it obscures provenance and activates unrelated repository bytes.

### Progressive checked capture

At startup, open each `SKILL.md` through retained checked directory handles and read only a bounded frontmatter prefix (8 KiB per file, 64 KiB across at most 64 effective skills). Parse YAML `name` and `description` with a pinned Serde parser; reject invalid/missing names, descriptions over the upstream 1024-character bound, or a missing closing fence. Retain the exact prefix bytes, source path digest and file/directory identities. Do not read the Markdown body or other skill files at startup. Capture effective custom-command files fully, up to 64 KiB each and 256 KiB total across at most 64 commands; a command file has the same required `name`/`description` frontmatter followed by nonempty Markdown prompt text. Alternative: eagerly capture all skill bodies. Rejected because it defeats on-demand disclosure and expands startup prompt authority.

Core derives grouped project skill-metadata and custom-command claims in the base manifest. Once base preflight passes, the actor sees only the bounded effective skill catalog and the TUI shows only effective command entries. User-config sources remain caller authority outside the repository manifest but still use checked capture and bounds. Later skill selection reopens the exact checked `SKILL.md`, compares its frontmatter and retained identities, then reads at most 256 KiB of body; an optional reference is one direct Markdown file under that skill's `references/` directory, captured with the same checks. Selected body/reference bytes, path and directory/file identities join the complete supplemental manifest, capped at 128 sources and 1 MiB aggregate. Missing, changed, linked, escaping or over-limit material fails that selection explicitly, without publishing partial instructions. Alternative: infer authority from filename or treat a selected reference as ordinary tool data. Rejected because neither binds the instructions promoted into the actor prompt.

### Review and runtime continuation

The app's existing prompt gate owns one active snapshot for nested instructions and selected skills. `skill_load` is a fixed actor tool with a catalog name and optional `references/<file>.md` argument; it cannot select arbitrary paths. Its checked capture yields the complete new manifest and canonical sorted supplemental source-path set. Existing v2 once/persist/deny review applies when project authority changes. Before publication, revalidate the exact root and active directory identities, and compare the pre-review trust-record generation so a concurrent revoke/base reapproval cannot silently validate a stale wait. Persist under the existing root lock; once is invocation-scoped. Publish only captured body/reference bytes and return a short tool acknowledgment. The runtime settles the original tool call, installs the approved prompt update and asks the actor to continue; no shell or other side effect is replayed. Alternative: put body in a tool result without review. Rejected because a result is data, while the selected body is intended prompt instruction authority.

The skill catalog is embedded in the approved instruction prefix, while activated bodies and references follow in stable source-path order after project instructions. Skill body rendering excludes the YAML frontmatter, especially `allowed-tools`, and explicitly says skills do not alter Kuru tool permissions. Previously activated captured bytes remain stable for this invocation; a later invocation recaptures changed files. User-config skills need no repository trust review but still cannot grant tool capabilities.

### Custom prompt commands

Build an invocation-local command registry from P02a's built-ins and the approved effective custom entries. Help, leading-token completion and dispatch read that same registry. Running `/name arguments` turns the captured Markdown body plus a separately labeled literal argument string into an ordinary user turn in the current session. No shell interpolation, process launch or implicit tool permission occurs. Malformed or colliding entries receive bounded diagnostics, and an unknown command remains local. Alternative: execute custom entries as shell commands or treat every slash string as a provider prompt. Rejected because the canonical plan requires explicit registration and ordinary tool authority.

## Risks / Trade-offs

- [Catalog metadata changes during a turn] → Keep the reviewed prefix and identities immutable; refuse a selection whose file identity or prefix changed and require a fresh invocation.
- [A persistent review races revocation] → Compare the fresh v2 generation under the existing root lock before approval publication; deletion or recreation cannot satisfy an old review.
- [Large or hostile YAML] → Read only the bounded frontmatter prefix, parse to typed fields, and render bounded escaped omission notices without raw parser or secret text.
- [A skill asks for stronger tools] → `allowed-tools`, scripts and prose grant no capability; independent ToolHost permission checks continue to govern effects.
- [A large supplemental union repeats reviews] → Stable sorted source sets and complete manifests retain correctness; the source/byte caps bound prompt and trust-record growth.

## Operational surface

This is an in-process TUI and headless-run feature. It adds no listener, bind address, container mount, service, credential or external secret. Skill and command paths are opened beneath the retained exact project root or the caller's resolved user configuration directory. The existing per-invocation tool-call budget still counts `skill_load`; source-count and byte caps bound catalog and activated prompt growth. The ordinary supported binary/architecture and native CI matrix remain unchanged.

## Integration contract

Kuru owns the user/project skill directories and parses the Agent Skills `SKILL.md` YAML frontmatter contract for `name` and `description`; optional upstream fields are accepted as metadata but cannot change Kuru permission selectors. Only direct `references/<file>.md` files can be promoted as selected references. No external SDK, route, remote mount, schema ID conversion or plugin registry is involved. The existing connector tool schema exposes one fixed `skill_load` name/reference call; core owns checked source identities, the app owns approval, and runtime owns prompt continuation.
