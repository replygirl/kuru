# Spec Delta

## ADDED Requirements

### Requirement: Runtime activation recovery boundary

Windows runtime-activation recovery SHALL honour its caller's cancellation at its existing await point, the retry-spacing wait taken while the bounded two-second recovery window is open; a cancelled activation MUST then tear its private stage down under the installation-stage ordering before the cache lock is released. A recoverable access-denied no-move that is observed after the recovery window has already elapsed MUST end recovery with the recovery-stopped context. This holds even when it is the first attempt, and even when the caller was cancelled as that result was observed, because no await point follows a terminal decision. The typed publication error and native code MUST be preserved, the stage MUST be preserved and named in the returned error, and the cache lock MUST be released only after the stage is kept. The window, its retry spacing and the set of recoverable errors are unchanged, and no await point is added whose only purpose is a caller's cancellation timing.

#### Scenario: First checked result arrives after the window
- **WHEN** the first activation attempt returns a proven no-move with access denied after the two-second window has elapsed
- **THEN** Kuru performs no further move, reports "runtime activation recovery stopped" with the typed rejected publication error, and preserves the private stage

#### Scenario: Caller cancelled at a checked no-move inside the window
- **WHEN** the caller is cancelled as a recoverable checked no-move is observed while the recovery window is still open
- **THEN** activation is cancelled at the retry-spacing wait rather than completing, and the private stage is torn down under the installation-stage ordering before the cache lock is released

#### Scenario: Caller cancelled at a late first checked no-move
- **WHEN** the caller is cancelled as the first recoverable checked no-move is observed after the window has elapsed
- **THEN** activation completes with "runtime activation recovery stopped" and the typed rejected publication error, names the preserved private stage, and releases the cache lock only after that stage is kept
