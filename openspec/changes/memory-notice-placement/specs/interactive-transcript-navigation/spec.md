# Spec Delta

## ADDED Requirements

### Requirement: Local display items keep their position

Each local transcript item (command output, local notices and errors) SHALL retain the identity of the last persisted-identity item displayed before it, or none when it preceded every record. Installing a newest, restored, older, newer or search page SHALL re-place each retained local immediately after that anchor when present, preserving relative order among locals with the same anchor; before all public records when it has none; and, when its anchor is absent, before the next later surviving item or else at the end. Local items bound to a now-persisted turn SHALL be replaced by the public record. Page eviction, newer-page insertion, cursor lookup and search MUST count only public items, so interleaved locals cannot change which public records are retained or matched. Local count and byte bounds and their omission accounting SHALL be unchanged.

#### Scenario: Help output after a completed turn
- **WHEN** a user runs `/help` and then submits and completes a turn
- **THEN** the help output stays between the record it followed and the new user message instead of moving above the newest user message

#### Scenario: Anchor outside the newest window
- **WHEN** a local's anchor is no longer in the installed newest page
- **THEN** the local is placed before the first later item that survives rather than re-attached to the tail
