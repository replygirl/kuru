## 1. Safe runtime observations [critical]

- [x] 1.1 @integration (agent) drive a fake-provider retry with actor and external tool success/failure through a real CLI child -> correlated turn/actor/tool/retry records contain only typed safe fields and no fake sentinel in every rotated file. Observed: `/private/tmp/kuru-tracing-cli-provider-tool-test-final5.log` and `/private/tmp/kuru-tracing-cli-failing-shell-test-final2.log` exit 0; the focused four-slot layer test exercises rotation with a fake sentinel.
- [x] 1.2 @runtime (agent) cancel an asynchronous turn after actor/tool work begins -> relevant spans close and no entered span is retained across awaits. Observed: `/private/tmp/kuru-tracing-pty-cancellation-test-final.log` exits 0; actual debug PTY cancellation observes `cancelled` and `span_close`, followed by a successful next turn and terminal restoration.

## 2. Bounded private diagnostic ring [critical]

- [~] 2.1 @e2e (agent) run a runtime-owning CLI command until fixed byte/count rotation -> defer: focused `diagnostics::tests` proves fixed four-slot rotation, restart-oldest selection, late-write disarm, and checked replacement refusal, but no real CLI run has yet forced all byte/count rotations.
- [~] 2.2 @integration (agent) force diagnostic setup and write failure through the app boundary -> defer: checked ring replacement failure disarms the writer and CLI cleanup preserves successful command output, but a direct app-boundary setup/write-failure injection remains unrun.

## 3. Presentation and native evidence

- [x] 3.1 @e2e (agent) compare normal and `--debug` `kuru run --json` plus a real PTY -> JSON stdout and terminal rendering remain unchanged and neither mode emits unsolicited diagnostics. Observed: `/private/tmp/kuru-tracing-cli-provider-tool-test-final5.log` compares JSON except generated session identity with empty stderr; `/private/tmp/kuru-tracing-pty-cancellation-test-final.log` proves debug PTY restoration and no trace rows in the transcript.
- [~] 3.2 @integration (agent) run native Windows ring/privacy/rotation fixtures -> defer: native Windows execution requires CI after source freeze.
- [~] 3.3 @integration (agent) run coordinated workspace coverage -> defer: root schedules the sole coverage writer after focused source freeze.
