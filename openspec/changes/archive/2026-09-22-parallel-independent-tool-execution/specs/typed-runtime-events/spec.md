## ADDED Requirements

### Requirement: Concurrent tool activity retains exact settlement

Projected tool activity SHALL show every actually started call until that call settles, including same-name calls and out-of-order settlements. The runtime SHALL emit exactly one bounded settled observation per admitted call with its original provider call ID and result receipt. Its compatible three-key wire adapter SHALL retain the existing tool-start and tool-observation forms and SHALL NOT expose raw arguments or result bodies to the activity label.

#### Scenario: Two active calls settle in reverse order

- **WHEN** two same-actor tool calls start and the later call settles first
- **THEN** the view still shows the earlier call active, each settlement clears only its own activity, and both projected observations retain their distinct call IDs
