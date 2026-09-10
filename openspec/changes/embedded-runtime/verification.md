## 1. Complete offline installation [critical]

- [x] 1.1 @e2e (agent) Launch only the packaged Kuru executable with empty offline cache and isolated data, run demo chat, reopen and inspect memory plus license hashes -> persisted real Dolt conversation without separate installation or download
- [x] 1.2 @integration (agent) Exercise source/direct installation and native update, then launch the resulting executable with another empty offline cache -> every install path carries the complete matching runtime
- [ ] 1.3 @integration (agent) Run packaged runtime smoke on existing macOS/Linux architecture matrix -> actual conversation/reopen passes on every currently supported target; Windows is verified in the dependent native-windows change

## 2. Reproducible build preparation [critical]

- [x] 2.1 @integration (agent) Prepare actual pinned local archive offline and build through mise from missing generated inputs -> verified exact target bytes embedded with licenses and no Cargo networking
- [x] 2.2 @regression (agent) Feed corrupt/truncated/unsafe prepared archives and missing or mismatched target inputs -> explicit build/preparation refusal, no fallback or partial executable

## 3. Safe extraction [critical]

- [x] 3.1 @integration (agent) Exercise actual embedded extraction, concurrent first startup, cancellation, corrupt cache and adversarial archive cases -> only fully verified runtime activates and existing cache remains preserved on failure
- [x] 3.2 @manual (agent) Inspect runtime provisioning dependencies and all install/build docs -> no runtime engine HTTP path and no separate end-user Dolt installation requirement

## 4. Repository completion [critical]

- [ ] 4.1 @integration (agent) Run mise run check and observe native hosted checks -> strict lint/tests/docs/cospec and at least 90 percent meaningful workspace coverage pass with recorded SHA
- [ ] 4.2 @manual (agent) Review canonical guidance and archive through cospec -> portability and bundled runtime contract durably reinforced without premature platform claims

### Observed local evidence, 2026-09-10

The macOS arm64 installed-runtime regression first rejected the prior executable
at its initial offline conversation because the engine cache was empty. The same
test then passed against the embedded executable in 38.13 seconds. It packages
and installs the actual executable, clears inherited environment and PATH tools,
sets memory offline mode, persists and resumes a conversation, inspects the real
transcript and Dolt revision history, and validates extracted executable and
license sizes and digests. Self-update replaces the running installed image;
the updated executable repeats the conversation checks with another absent cache
and data directory. The measured debug executable was 90,746,168 bytes and its
archive 47,849,433 bytes. This fixture does not configure an OS egress firewall;
runtime HTTP removal is independently established by source/dependency review.
Evidence: `/tmp/kuru-embedded-runtime-{red,green,lint}.log`.

Memory checks passed five shared build-policy cases, thirteen provisioning
cases and one catalog case, including real cold-offline extraction and exact
license reuse. A cancellation barrier proves the blocking extraction worker
retains its staging directory and stable lock through completion after its
caller is cancelled. Actual Cargo builds with missing input and a same-size
corrupt archive both failed in build.rs with the target-specific preparation
command; restoring the valid local input built successfully. Strict memory
lint passed. Details: `/tmp/kuru-embedded-memory-evidence.md`.

The delivery preparation helper passed nine integration and three unit checks,
including local import, incorrect hashes and targets, unsafe input paths,
concurrent imports and cancellation. A concurrent stable-lock creation failure
was reproduced and corrected with exclusive creation followed by opening the
existing lock on EEXIST. Strict lint passed. Evidence:
`/tmp/kuru-embedded-delivery-evidence.md`.

Review found that explicit `--target host` could conflict with an inherited
`CARGO_BUILD_TARGET`. Build tasks now capture the original selection for their
default and clear Cargo's inherited target before applying the explicit option.
An actual app build under a foreign target environment passed with an explicit
host override. Foreign-target preparation selected and verified the requested
Linux archive while the helper remained a native macOS executable. Source
installation rejected a foreign target with its prior destination inode and
contents intact.

Native source installation then passed with Cargo, bundle preparation and mise
offline flags enabled, building a new release profile in 62.56 seconds. The
52,167,840-byte installed executable exactly matched the release build. It passed
the packaged acceptance test in 19.11 seconds, including direct install and
self-update conversations from separate empty offline caches. Its archive was
40,670,792 bytes. The initial local-process sandbox refusal was resolved through
normal tool permission without code, assertion or timeout changes. Evidence:
`/tmp/kuru-embedded-source-acceptance.md` and
`/tmp/kuru-embedded-source-release-acceptance-green.log`.

The developer and public documentation build, format and generated-site link
checks passed; workflow actionlint passed. Source review confirmed the default
memory path has no HTTP client dependency and all Dolt installation guidance
describes bundled extraction. Full workspace coverage and the four hosted
platform checks remain pending and are not inferred from these checks.

After integration with the native-platform foundation, the complete local
`mise run check` passed in 540.09 seconds with 12,786/13,138 covered lines
(97.3207%). This includes the actual packaged-runtime test in both ordinary and
instrumented runs, with no production exclusions or threshold change.
Log: `/tmp/kuru-embedded-main-check.log`. Hosted checks on the committed embedded
implementation remain pending.

The full gate passed again after the Windows filesystem corrections and final
integration guidance, in 467.28 seconds with the same 12,786/13,138 covered lines
(97.3207%). Log: `/tmp/kuru-embedded-platform-corrected-check.log`.

### Hosted checkpoint and CI storage correction

At candidate `6d55ac7432929f08713e330c7010e661e5aedda8`, the native Linux ARM
and Intel macOS jobs completed release build/package, real Dolt memory tests
and installed/self-updated offline conversations. Linux ARM ran 57 memory
tests; its executable/archive measured 51,730,640/40,148,150 bytes and packaged
acceptance passed in 32.48 seconds. Intel macOS ran 58 memory tests; its
executable/archive measured 55,802,432/43,225,512 bytes and acceptance passed in
77.46 seconds. The platform-dependent build-input test counts differ because
the macOS fixture also tests standard temporary-directory aliases. Evidence:
`/tmp/kuru-embedded-runtime-hosted-checks.md` and the corresponding native logs.

The [Ubuntu x64 full gate](https://github.com/replygirl/kuru/actions/runs/34503834945/job/102960966892)
failed while linking with explicit `No space left on device` (OS 28). A memory
provisioning test also printed a failure before the aggregate stopped, but its
panic was not emitted; its cause is not inferred from the linker failure.
The runner already disabled incremental compilation and excluded workspace
crates from its dependency cache. The narrow correction sets dev/test debug
information to `line-tables-only` in the full-check CI job before cache lookup;
it retains source-line diagnostics, assertions, tests and the coverage floor.

A fresh isolated local target using those profiles compiled and passed the
actual packaged offline installation/update regression (88,287,272-byte
executable; 47,120,716-byte archive). The independent platform coverage task
retained exactly 458/486 covered lines (94.2387%) with one unit and seventeen
integration cases in both ordinary and instrumented runs. Total target
allocation was 2,056,716 KiB on macOS ARM; Linux allocation is still unmeasured.
An initial sandbox refusal to start the local server was resolved through the
normal execution permission, without a changed assertion or timeout. Tooling
lint and actionlint passed. Evidence: `/tmp/kuru-ci-lines-evidence.md`.
The corrected full local and hosted gates remain required before archive.

The final macOS ARM job also completed successfully at `6d55ac7`: full check
914.00 seconds with 12,786/13,138 covered lines (97.3207%), source installation
147.35 seconds, and installed-runtime acceptance 25.21 seconds. Its installed
executable/archive measured 52,184,496/40,661,955 bytes. The aggregate failed
because Ubuntu x64 exhausted disk; no hosted corrected pass is claimed yet.

The complete local gate with the proposed CI profile then passed in 518.17
seconds, again covering exactly 12,786/13,138 lines (97.3207%). Its isolated
target totaled 9.1 GiB after all ordinary and instrumented workspace suites;
this is the macOS measurement, not an inferred Linux result. No source was
excluded and no threshold changed. Log: `/tmp/kuru-ci-profile-full-check.log`.
