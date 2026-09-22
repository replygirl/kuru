## Context

`ui.rs` currently has a literal `HELP` list, separate interactive branches for permission/help/picker/quit controls, and another string match in `dispatch_controlled` for runtime commands. `View::key` does not complete slash names. The P01c branch already separates modal instruction review from ordinary composer input and keeps the live TUI view as an owned, synchronous projection of the runtime. P09/P11/P12 will add command backends later.

## Goals / Non-Goals

**Goals:** Make one discoverable built-in command catalog drive public names, help, parsing, completion and dispatch identity; add local view commands without opening memory or changing provider work.

**Non-Goals:** A dynamic plugin system, skill/reference loading, custom-command config, or claiming later commands work before their handlers land. Those P02 skill/custom behaviors have their own dependent change.

## Decisions

1. **Use a small typed registry in the TUI package.** Each descriptor binds a canonical slash name, usage/summary, and enum dispatch ID. Public help and prefix completion iterate those descriptors; both local UI and async runtime handlers match the parsed ID. The same name never appears in a second hand-maintained dispatch table. A mutable registry or general callback framework was rejected: built-ins are compile-time known, and a plugin runtime is outside P02.
2. **Keep private modal tokens outside the public catalog.** The existing instruction and permission review, cancellation, and selected-permission controls execute only in their current modal/shortcut context. They do not become discoverable slash commands. This avoids a public entry accidentally bypassing prompt priority or expanding approval semantics.
3. **Complete only the leading token.** `Tab` cycles matching registered names in canonical sorted order; editing resets the cycle, and no completion touches arguments or sends a command. A completion popup with separate focus and navigation was rejected for this first registry slice because the existing editor already has picker and review modals; deterministic inline cycling can be exercised in real PTYs without adding another modal layer.
4. **Handle `/clear` and `/status` against the owned View.** `/clear` drops rendered transcript rows and their per-row completion metadata, resets scroll, and retains the `Harness`, session ID, persisted history, usage and turn count. `/status` renders current `View` fields and cached reported usage as an informational transcript entry. It does not open a store or call a provider. Reusing the runtime slash dispatcher for either would create avoidable asynchronous effects and would not make the view-only boundary clear.

## Risks / Trade-offs

- [A future handler is added without registration] → Unknown commands fail rather than being silently advertised; tests compare each public registry descriptor to help, completion and dispatch, and later owning changes register only completed handlers.
- [Completion steals a review key] → Existing prompt and picker priority stays before composer `Tab` handling; real PTY and view-level tests cover ordinary digits and modal input.
- [Clear appears to delete history] → User text explicitly says the view was cleared and stored history remains; a restart/next-turn fixture verifies the same session still has its persisted context.

## Operational surface

The registry and completion run inside the existing local `kuru` TUI binary on each supported architecture. They add no bind address, service connection, container, runner role, credential, environment variable, or dependency version. `/status` reads the already projected current View; `/clear` changes only that View. Native real-PTY checks exercise the ordinary application binary and existing provider fixture.
