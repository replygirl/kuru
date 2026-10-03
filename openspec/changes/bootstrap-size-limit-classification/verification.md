# Verification

## 1. Over-cap producers are classified and leave no core dump [critical]

- [ ] 1.1 @regression (agent) `size_limit_failures_are_reported_when_the_caller_ignores_sigxfsz` (bootstrap launched through `trap '' XFSZ; exec /bin/bash install.sh`, oversized local manifest and archive) on the unfixed and fixed `install.sh` -> unfixed fails on "failed (producer 1)"; fixed reports "local release asset exceeds size limit" both times
- [ ] 1.2 @regression (agent) `bounded_producers_cannot_write_core_dumps` (fixture `curl` records `ulimit -H -c` from inside the producer) on the unfixed and fixed `install.sh` -> unfixed records the inherited nonzero hard limit; fixed records `0` for every bounded download and installs
- [ ] 1.3 @unit (agent) Bash 3.2 probe: `{ ulimit -f 64; exec cat big; }` with a 65537-byte input, with and without SIGXFSZ ignored -> 153 with 65536 bytes by default; 1 (`File too large`) with 65536 bytes when ignored

## 2. Existing bootstrap behavior is preserved

- [ ] 2.1 @integration (agent) `cargo test -p kuru-delivery --all-features --test bootstrap_install`, three runs -> all pass, including the sparse-expansion test that relies on status 153 with a near-empty output
- [ ] 2.2 @integration (agent) `mise run //packages/kuru-delivery:test`, `lint:shell`, `format:check`, `lint`, root `lint:windows`, `typecheck`, `lint:tooling`, `docs:check`, `cospec -- validate --all --strict` -> each exits 0
- [ ] 2.3 @integration (agent) Linux bootstrap behavior -> PR CI's native Linux jobs run `bootstrap_install`
