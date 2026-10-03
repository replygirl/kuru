# Tasks

## 1. Bounded producer

- [x] 1.1 Replace the FIFO and reader in `bounded()` with a single producer under a 1024-block `ulimit -f` cap, classify status 153 as exceeding the cap, keep cleanup's KILL of the producer, and verify the scratch harness and end-to-end loop no longer stall
- [x] 1.2 Drop `mkfifo`, `head` and `wc` from the required tools and widen the fixture's stage observations, and verify the bootstrap suite passes

## 2. Regression test

- [x] 2.1 Add a mirror-case test with a hostile `mkfifo`, and verify it reaches the 30 s bound with the original bootstrap and installs with the fix

## 3. Checks

- [x] 3.1 Run the delivery package tests, its `lint:shell`, `format:check`, `lint`, root `lint:windows`, `typecheck`, `lint:tooling`, `docs:check` and `cospec -- validate --all --strict`, and verify each exits 0
