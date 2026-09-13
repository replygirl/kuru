## 1. Safe instrumentation

- [x] 1.1 Add exact workspace tracing pins and subscriber-neutral runtime turn, actor, cognitive-tool, and external-tool asynchronous spans using the journal digest, and verify focused runtime cancellation and status fixtures. Observed: `/private/tmp/kuru-tracing-pty-cancellation-test-final.log` exits 0.
- [x] 1.2 Add connector typed retry events at existing retry decisions and verify fake retry records expose no remote diagnostic content. Observed: `/private/tmp/kuru-tracing-cli-provider-tool-test-final5.log` exits 0.

## 2. Application diagnostics

- [x] 2.1 Add a real-CLI-only fixed target subscriber and checked private per-project byte/count JSONL ring under the existing writer lease, with focused rotation, replacement refusal, restart retention, and finish-disarm checks. App-boundary setup/write-failure evidence remains tracked in verification 2.2.
- [x] 2.2 Add `--debug` operational detail without `RUST_LOG` or changed application output. Observed normal/debug CLI comparison and real-PTY cancellation preserve output, while focused rotated-layer checks exclude fake secrets.

## 3. Documentation and evidence

- [x] 3.1 Document local fixed retention, redaction limits, and `--debug`, and verify owning docs checks. Observed: `/private/tmp/kuru-tracing-docs-check-final3.log` exits 0.
- [x] 3.2 Run focused runtime/connector/app checks and record native Windows and coordinated-coverage evidence honestly. Focused logs are recorded below; native Windows and coordinated coverage remain deferred in verification.

## Observed focused evidence

- `/private/tmp/kuru-tracing-cli-provider-tool-test-final5.log`: exit 0. A real CLI child compares normal and `--debug` JSON turns except for generated session identity, drives fake-provider retry, actor work, owned external shell success, and checks no fake provider/API sentinel in diagnostics.
- `/private/tmp/kuru-tracing-cli-failing-shell-test-final2.log`: exit 0. A real normal-mode CLI child receives a failing owned shell receipt with the complete redaction marker and no raw command or sentinel in continuation or diagnostics.
- `/private/tmp/kuru-tracing-pty-cancellation-test-final.log`: exit 0. Existing native PTY fixture runs the actual debug binary, cancels in-flight provider work, observes a cancelled actor record and span close, preserves the draft, accepts the next turn, and restores the terminal.
- Native Windows and coordinated coverage are not run in this change worktree.
