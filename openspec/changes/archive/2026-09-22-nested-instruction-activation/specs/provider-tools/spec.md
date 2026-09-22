## ADDED Requirements

### Requirement: Checked target admission before nested instruction exposure

For prompt-bearing actor turns, Kuru SHALL identify each native file target through its existing checked workspace path and exact permission decision before that target can trigger nested instruction capture. Direct `tool` CLI calls and non-prompt inspection SHALL retain their existing permission behavior without activating instruction authority. A checked path that changes across a foreground permission or trust wait MUST NOT be dispatched under the prior decision.

#### Scenario: Direct file call is denied
- **WHEN** an actor's `file_read` target is denied by its effective rule
- **THEN** no instruction tree at that target is activated and no file content is returned.

#### Scenario: Direct CLI tool inspection
- **WHEN** `kuru tool file_read` is invoked without a prompt-bearing runtime
- **THEN** its file permission is evaluated normally but nested project instructions are not consumed.

### Requirement: Bounded search candidate instruction admission

Native `grep` and `glob` SHALL form a bounded checked candidate set, apply each candidate's exact permission decision, then derive applicable nested instruction authority only from allowed candidates before exposing any candidate path, match, or content to an actor. Permission-denied candidates SHALL NOT activate their instruction trees. Search SHALL preserve its independent hidden and ignore behavior and existing result and traversal limits, and SHALL report candidates or branches omitted by limits or failed review truthfully rather than silently returning a partial result as complete. A trust refusal or required headless review SHALL prevent exposure of the affected candidate results without broadening permission to a subtree.

#### Scenario: Mixed allowed and denied search candidates
- **WHEN** a bounded search sees authorized `src/a.rs` and denied `secret/b.rs`, each with a different nested instruction file
- **THEN** only the authorized path's instruction ancestry is eligible for review and activation, and the denied candidate contributes no path, match, content, or instruction authority.

#### Scenario: Candidate limit before review
- **WHEN** traversal reaches the existing candidate cap before covering the full requested search
- **THEN** Kuru reports the scan limit and reviews only the checked, allowed candidates actually admitted, without suggesting the result or instruction coverage is complete.
