## Context

Instruction discovery currently happens inside `Harness::with_tool_host`, after CLI workspace preflight. The runtime reopens every ancestor `AGENTS.md` by pathname, so neither the ordered source set nor the injected bytes are represented in the immutable authority snapshot.

## Goals / Non-Goals

**Goals:** Capture all present automatic instruction sources from outermost ancestor through the project root, bind their ordered identities and exact bytes into one extensible claim, apply that claim only to runtime commands, and inject the captured bytes.

**Non-Goals:** Discover `CLAUDE.md`, resolve imports, add user-global instructions, interpret instruction contents, or make instruction files configuration layers.

## Decisions

The core snapshot owns an ordered `InstructionSource` list. Each entry has a typed source kind, a bounded safe label, a full path digest for private identity binding and the exact UTF-8 bytes read through the existing per-file and aggregate bounds. The claim digest encodes the ordered source kind, path digest and content; the manifest separately retains the ordered path digests. This shape lets future source kinds and imports join the same claim without creating another trust gate.

The formatted prompt text is constructed once from the captured entries and returned by the snapshot. Runtime construction receives that string and never reopens instruction paths. A mutation after snapshot creation therefore cannot change the bytes injected during that invocation.

Automatic instructions apply to `run`, `dream`, `serve` and the interactive TUI because those paths construct peers and prompts. Authentication, configuration, trust inspection, memory inspection, catalog inspection and direct tools remain independent when they do not construct a runtime.

Trust review prints a fixed instruction claim label and its bounded ordered source labels. It never prints instruction content. Persistent approval still binds the complete manifest, so adding, removing, reordering or changing an instruction source makes the saved approval stale.

## Risks / Trade-offs

- [Instruction files change after snapshot] → Runtime uses the owned snapshot bytes, making the invocation deterministic and preventing unreviewed replacement bytes from reaching a provider.
- [Ancestor paths or contents disclose sensitive text] → Only existing bounded escaped source labels are displayed; contents appear only in the private digest and approved model prompt.
- [Commands unnecessarily consume prompt authority] → The command matrix adds the category only to paths that construct `Harness`.

## Operational surface

The change adds no bind address, network route, container/runner distinction,
secret, connection limit, binary version or architecture requirement. Its only
startup effect is a pre-terminal approval prompt when automatic instructions
make the current runtime command's claim subset nonempty.
