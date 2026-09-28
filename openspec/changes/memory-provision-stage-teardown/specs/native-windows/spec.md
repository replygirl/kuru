# Spec Delta

## ADDED Requirements

### Requirement: Runtime activation recovery boundary

Every checked Windows runtime-activation no-move observation SHALL be a cancellation point: activation MUST yield to its caller's cancellation before it decides whether to retry or stop. A recoverable access-denied no-move that is observed after the bounded two-second recovery window has already elapsed MUST end recovery with the recovery-stopped context. This holds even when it is the first attempt. The typed publication error and native code MUST be preserved, and the stage MUST be preserved. The window, its retry spacing and the set of recoverable errors are unchanged.

#### Scenario: First checked result arrives after the window
- **WHEN** the first activation attempt returns a proven no-move with access denied after the two-second window has elapsed
- **THEN** Kuru performs no further move, reports "runtime activation recovery stopped" with the typed rejected publication error, and preserves the private stage

#### Scenario: Caller cancelled at a checked no-move
- **WHEN** the caller is cancelled as a checked no-move is observed, including after the window has elapsed
- **THEN** activation is cancelled rather than completing, and the private stage is torn down under the installation-stage ordering before the cache lock is released
