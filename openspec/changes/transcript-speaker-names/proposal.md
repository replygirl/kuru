# Proposal

## Why

The interactive transcript labels a part's reply with its display name (for example `Self`) while the reply is the live tail, but after the turn persists and the public page is reinstalled, every earlier reply is labeled with the part's raw UUID (`e1567a83-468e-5927-a7f7-dcd081401411`). Maintainer screenshots of 2026-10-19 show two earlier replies with UUIDs and the newest with `Self`.

The persisted projection (`Transcript::from_page`/`append_turn`) labels assistant entries with the record's `speaker_id` because it is a static projection with no access to the view's roster. The live path uses `View::actor_name` (roster, then relationships, then a 16-character id prefix), so the same speaker is named two ways and unknown speakers get two different fallbacks.

## What Changes

- One shared speaker-label function (roster name, relationship `kind · members`, 16-character prefix) serves the live reply, activity and persisted projections; persisted assistant entries get exactly the live label.
- Transcript items keep the raw speaker id. Every roster or relationship change re-derives the labels of all items that carry one, so renames and framework switches update earlier replies.
- Newest, older, newer, search and restore page installs, and the initial view projection, resolve labels through that function. Non-assistant roles and records without a speaker are unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`apps/kuru-tui/src/ui.rs`, `apps/kuru-tui/src/ui/transcript.rs` and a short passage each in `docs/interface.md` and `docs/usage.md`. No memory record, migration, CLI output, protocol or dependency change. `kuru sessions`, `/export`, `kuru sessions export`, `kuru run --json`, `kuru memory export` and the fork-boundary picker are not transcript projections and still print the stored speaker id; a memory-side display-name snapshot in `PublicTurnRecord` is an explicit follow-on and non-goal. The interface docs state that reloaded replies share the live name, naming the removed-relationship exception, and the usage docs list the outputs that keep stored identities.

## Surfaces

- [x] interactive — transcript speaker labels in the TUI
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
