# Proposal

## Why

Session isolation already requires actual provider context to contain only the
current session's private rows and policy-approved same-actor continuity. The
existing two-terminal proof checks first requests and storage, but not subsequent
requests after both live sessions have completed turns.

## What Changes

- Extend the existing real PTY/shared managed Dolt fixture in
  `apps/kuru-tui/tests/terminal.rs` with distinct private actor outputs and second
  turns in both still-live sessions.
- Seed typed continuity summaries from a separate synthetic prior session through
  public memory APIs after both live turns settle; inspect captured HTTP requests
  for own live raw rows, approved same-actor summaries, and excluded foreign raw
  and unrelated-part private context.

## Impact

Test-only source and this change's artifacts. One existing native terminal case
gains two turns and bounded summary setup; no production, provider, configuration,
dependency, deadline, or published documentation changes.
