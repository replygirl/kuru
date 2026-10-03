# Tasks

## 1. Regression tests

- [x] 1.1 Add a test that runs the bootstrap with SIGXFSZ ignored for an oversized manifest and archive, and verify it reports "failed (producer 1)" against the unfixed bootstrap
- [x] 1.2 Add a test that reads the producer's core-dump hard limit through the fixture `curl`, and verify it reads the inherited nonzero limit against the unfixed bootstrap

## 2. Bootstrap

- [x] 2.1 Set `ulimit -c 0` in the producer group, add the cap-sized-output fallback for nonzero statuses other than 153, restore `wc` to the required tools, and verify both regression tests and the existing size-limit and sparse tests pass
- [x] 2.2 Note in `docs/install.md` that the per-step limit applies to installer output appended to a regular file, and verify `docs:check` passes

## 3. Checks

- [x] 3.1 Run `bootstrap_install` three times, the delivery package tests, `lint:shell`, `format:check`, `lint`, root `lint:windows`, `typecheck`, `lint:tooling`, `docs:check` and `cospec -- validate --all --strict`, and verify each exits 0
