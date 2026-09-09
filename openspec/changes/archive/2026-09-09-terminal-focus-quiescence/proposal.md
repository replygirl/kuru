## Why

Linux CI run 34401929985 failed the terminal focus-loss assertion under coverage: eight trailing bytes arrived after the fixture observed its draft text. Visible text can arrive before the remainder of a terminal frame, so treating that first observation as a completed output boundary can falsely report continued animation.

## What Changes

- Make the real PTY focus-loss check wait for the backend's complete Show/MoveTo composer-frame trailer before recording its output baseline.
- Add a deterministic fixture regression that withholds and splits this trailer, rejecting text-only and partial-escape observations while retaining the strict no-further-output assertion.
- Preserve bounded waits, output draining, real application coverage and the unchanged coverage threshold.

## Capabilities

### Modified Capabilities

None. Focus loss already stops animation in the documented behavior.

## Impact

The Rust terminal integration fixture and focus assertion in apps/kuru-tui, plus canonical contributor guidance on completed-frame synchronization. Release behavior, workflow topology, dependencies and user controls remain unchanged.

## Surfaces

- [ ] interactive — verification of existing behavior only
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
