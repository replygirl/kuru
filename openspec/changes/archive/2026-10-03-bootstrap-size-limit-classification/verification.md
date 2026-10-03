# Verification

## 1. Over-cap producers are classified and leave no core dump [critical]

- [x] 1.1 @regression (agent) `size_limit_failures_are_reported_when_the_caller_ignores_sigxfsz` (bootstrap launched through `trap '' XFSZ; exec /bin/bash install.sh`, oversized local manifest and archive) on the unfixed and fixed `install.sh` -> Observed 2026-10-03, macOS arm64: unfixed (commit 59de88ee) fails with stderr `cat: stdout: File too large` / `kuru: local release asset failed (producer 1)`; fixed reports "local release asset exceeds size limit" for both the manifest and the archive and leaves the installation unchanged.
- [x] 1.2 @regression (agent) `bounded_producers_cannot_write_core_dumps` (fixture `curl` records `ulimit -H -c` from inside the producer) on the unfixed and fixed `install.sh` -> Observed 2026-10-03, macOS arm64 (inherited core hard limit `unlimited`, soft `0`): unfixed records `["unlimited", "unlimited", "unlimited"]`; fixed records `["0", "0", "0"]` for the three bounded downloads and installs. Red requires a nonzero inherited hard limit, which macOS and GitHub's Linux runners have by default.
- [x] 1.3 @unit (agent) Bash 3.2 probe: `{ ulimit -f 64; exec cat big; } > out &` with a 65537-byte input, with and without SIGXFSZ ignored -> Observed 2026-10-03: default 153 with 65536 bytes; ignored 1 (`cat: stdout: File too large`) with 65536 bytes. A producer writing to stderr appended to a 100000-byte file under `ulimit -f 64` exits 153 (the documented log-file residual).

## 2. Existing bootstrap behavior is preserved

- [x] 2.1 @integration (agent) `cargo test -p kuru-delivery --all-features --test bootstrap_install`, three runs -> Observed 2026-10-03: 27 passed each run, including the sparse-expansion test that relies on status 153 with a near-empty output.
- [x] 2.2 @integration (agent) `mise run //packages/kuru-delivery:test`, `lint:shell`, `format:check`, `lint`, root `lint:windows`, `typecheck`, `lint:tooling`, `docs:check`, `cospec -- validate --all --strict` -> Observed 2026-10-03, macOS arm64: each exits 0 (package suite 67 s, every binary passed, 1 pre-existing ignored; validate "0 errors, 0 warnings"). format:check and docs:check ran with the shell's NODE_OPTIONS preload unset.
- [~] 2.3 @integration (agent) Linux bootstrap behavior -> defer: no Linux host here; PR CI's native Linux jobs run `bootstrap_install`.
