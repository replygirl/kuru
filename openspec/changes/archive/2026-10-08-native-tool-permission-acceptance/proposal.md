# Proposal

## Why

The current direct-subscription acceptance explicitly avoids tool calls, leaving native tool execution and the real inline permission loop verified only against fixtures. A bounded synthetic live case can establish those existing contracts without accessing real project histories.

## What Changes

- Add one ignored Unix app test using the real TUI, isolated memory and project, and Kuru's existing native authentication internally. Require advertised `gpt-5.6-luna` and `low` before inference.
- Exercise an exact harmless scratch-file write through approve-once and a second write through deny; verify no effect while approval is pending, exact approved bytes, denied absence and provider continuation with matching tool results.
- Bound dispatch to eight streams, 16,000 estimated input tokens per request and 64,000 cumulatively, 16 KiB observed payload and a 180-second live deadline. Reject unexpected external tool targets before execution and retain fixed diagnostics and scalar observations. These client bounds are not a server billing-token ceiling.
- Exercise the same child TUI visibly in the user-authorized cmux `pane:105`, without a duplicate paid run. Retain the existing automated PTY route for reproducibility.
- Update the owning verification documentation with actual outcomes and limits.

## Impact

The app test and verification documentation are the affected surfaces. Ordinary CI compiles the ignored case without making paid requests; offline guard checks exercise failure modes. No production behavior, authentication storage, shell isolation, mise files or HTML roadmap changes are required.
