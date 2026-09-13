## 1. Dependency and event source

- [x] 1.1 Enable crossterm 0.29.0 `event-stream` only on the TUI dependency, add inherited futures 0.3.34 and the existing async-trait dev dependency for controlled provider fixtures, update the lockfile without version drift, and verify the locked feature graph and all TUI targets compile.
- [x] 1.2 Replace the interactive path's synchronous poll/read pair with exactly one `EventStream` behind a private generic stream seam, and verify event, error and EOF are represented without a second terminal reader or event buffer.
- [x] 1.3 Select Crossterm's supported level-polled Unix TTY event source and verify one real stream retains both a co-ready resize and bracketed paste without unrelated later input.

## 2. Fair scheduler and ordered state

- [x] 2.1 Implement a compact rotating biased-select scheduler for terminal, typed completion, activity and a persistent animation deadline, and verify continuously ready sources receive service within one complete rotation without resetting the motion clock.
- [x] 2.2 Consolidate queued and newly received results into one generation-checked completion handler, cap its captured pre-completion activity drain at 256, and verify exact turn/command/error application precedes runtime refresh and settle.
- [x] 2.3 Preserve explicit cancellation abort-and-await, activity projection, generation fencing, runtime refresh and completion-lock ordering, and verify a raced old result cannot affect the next turn.

## 3. Exit cleanup and regressions

- [x] 3.1 Extend the common run-loop error boundary to abort and await the active dispatch plus every nested runtime actor/provider task through non-dream Harness shutdown for terminal EOF/read errors, draw/backend errors and completion-channel invariant failures before returning the original error to terminal restoration.
- [x] 3.2 Add deterministic scheduler regressions targeting the former blocking-poll delay with a completion arriving after the scheduler starts waiting, plus rotating fairness, bounded activity drain, closed-activity behavior, generation rejection and shared failure cleanup. Record baseline source inspection separately from current runtime evidence.
- [x] 3.3 Extend the native terminal fixtures only as needed for EventStream EOF/error restoration, and retain the full existing PTY/ConPTY, view, scene, adapter, slash, visual, preference, trust and persistence assertion bodies.
- [x] 3.4 Install the controlled-provider drop guard before its started notification and verify every announced provider future has actually entered the lifecycle interval measured by the regression.

## 4. Verification

- [x] 4.1 Run focused TUI formatter, lint, typecheck, scheduler and native terminal targets through their owning mise tasks, recording exact exit status and native-platform limits without substituting a generic stream fixture for actual terminal restoration.
- [ ] 4.2 Coordinate the complete behavioral targets under the single workspace coverage task, verify at least 90 percent line coverage with the production scheduler included, and require the applicable native Windows EventStream/ConPTY job before merge.
