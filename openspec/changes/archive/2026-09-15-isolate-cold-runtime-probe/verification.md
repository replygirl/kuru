## 1. Cold probe isolation and publication [critical]

- [x] 1.1 @regression (agent) run the actual bundled Windows Dolt version probe from the checked private copy while retaining a handle to that copy, then activate the candidate with the real checked native move -> the copy has a distinct identity outside the candidate and cannot block candidate publication; placing the retained handle inside the candidate blocks before the fix
- [x] 1.2 @integration (agent) provision an actual empty Windows managed cache through extraction, exact-version probing and activation -> one fully verified runtime identity is published and opens without activation retries exhausting
- [x] 1.3 @e2e (agent) install, cold-open, update and cold-open the packaged Windows application with separate empty caches offline -> both installed versions complete a conversation and preserve the bundled runtime and memory contract

## 2. Integrity and ownership

- [x] 2.1 @regression (agent) corrupt source or copied probe bytes at the checked-copy boundary -> full digest verification rejects them before the version command or candidate activation
- [x] 2.2 @unit (agent) cancel the cold probe and exercise a failing exact-version command -> the probe copy and candidate stage remain private and owned until process cleanup, then both drop before the installation lock
- [x] 2.3 @integration (agent) run the existing warm, concurrent cold, activation uncertainty and corrupt-cache controls -> warm opens remain lock-free and full-digest checked; cold publication retains identity reconciliation and corrupt bytes never execute

## 3. Static checks

- [x] 3.1 @integration (agent) run memory formatting, lint, typecheck and focused tests -> affected checks pass

## Observed evidence

- The pre-fix packaged Windows update job `104483898161` completed extraction and exact-version checking, then reported 65 checked `ERROR_ACCESS_DENIED` move attempts over more than two seconds while every reconciliation proved the source remained and destination was absent. The log does not establish which mechanism or actor caused the denial.
- Local corrupt-source and corrupt-copy controls passed. Both full-digest mismatches were rejected at the checked-copy boundary.
- Local failed exact-version, live-probe cancellation and runtime-destruction controls passed. No candidate activated, and staging/probe ownership ended before the installation lock could be reacquired.
- Local concurrent-cold, warm-under-held-install-lock, corrupt-cache, checked activation failure and activation-source-open failure controls passed. The actual embedded Dolt also installed from an empty offline cache, completed exact-version verification and reused the verified payload.
- `cargo check -p kuru-memory --all-targets --all-features --locked --offline`, the equivalent strict Clippy invocation, `cargo fmt --all -- --check`, `git diff --check`, and strict Cospec validation passed.
- Native [Windows memory-runtime job `104500522195`](https://github.com/replygirl/kuru/actions/runs/35004425100/job/104500522195) passed `held_cold_probe_copy_does_not_block_candidate_activation`, concurrent cold publication of one verified native identity, the real embedded Windows empty-cache install, and corrupt-cache rejection before execution. The checked shard completed 104 memory tests with no failures.
- Packaged [Windows application job `104500522229`](https://github.com/replygirl/kuru/actions/runs/35004425100/job/104500522229) passed `packaged_install_and_update_preserve_complete_offline_memory` in 108.40 seconds. [Windows installation/offline job `104500522192`](https://github.com/replygirl/kuru/actions/runs/35004425100/job/104500522192) independently reported that direct install and self-update each persisted chat from an empty offline cache and passed the same acceptance in 111.31 seconds.
