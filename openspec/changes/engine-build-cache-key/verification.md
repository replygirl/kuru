# Verification

## 1. The key covers exactly the inputs that decide the engine bytes [critical]

- [x] 1.1 @regression (agent) `cargo test -p kuru-delivery --features tooling --test bundle_build` with the new key-coverage test, then the same test after each mutation in a scratch copy: remove a keyed file from the key, add an unkeyed file to the key, drop a crate from `ENGINE_ARCHIVE_CRATES`, add a tool to the `bundle:build` task, add a `--manifest` override to its `run` -> 5 passed on macOS arm64. `engine_input_cache_key_covers_exactly_the_build_inputs` reports no finding on the tree. `an_unkeyed_build_input_or_a_non_input_in_the_key_is_rejected` runs 11 mutations, each asserting its named finding: `main.rs` dropped from the key, `mise.toml` added to it, `zlib-rs` dropped from the list, the crate digest dropped from the key, an extra `cmake` task tool, a `--manifest` task argument, a task `env.GOFLAGS` value, another default manifest, a new non-test module `bundle/layout.rs`, an undeclared `bundle/packing.rs`, and `tar` added to kuru-archive's lock dependencies. With origin/main's `bundle-build.yml` restored in place, the tree test fails with 32 findings, among them `Cargo.lock`, `mise.lock`, `mise.toml`, `support/bundle.rs` and the `bundle/**` pattern
- [x] 1.2 @integration (agent) run the workflow's crate digest step script locally over this tree's `Cargo.lock`, and over a copy missing one named crate -> macOS `/usr/bin/awk` and Docker `ubuntu:24.04` `/usr/bin/mawk` (linux/amd64, Ubuntu 24.04's default awk; the GitHub runner image's awk and gawk were not run, and the script uses only POSIX awk) both print `sha256=75cfcd6e839522711c2fffa21b634d4226ece95af5526926310ea76e1a325602`; the copy with `zlib-rs` renamed exits 1 with `::error::Cargo.lock has no zlib-rs package`

## 2. Old and new keys over recent history

- [x] 2.1 @integration (agent) reproduce `hashFiles` locally and check it against three keys GitHub recorded (5a344499 `b04dda5d…`, 686eb16b `aaf6ebd1…`, f3e5bc6f `c7fdad73…`), then compute old and new keys for the last main commits and the PR heads the duration report names -> all three reproduce exactly. Simulating runs 644-664 plus main 8450c555, 31d8edef and a8ce468b (24 runs, only main saves): the old key misses 12, the new key 5. Runs 646, 649, 651, 662, 663, 664 and main a8ce468b would hit. The new key still misses on 644 and 648 (main saves after keyed-file changes), PR 645/647 (a real `bundle.rs` change) and 31d8edef (a `main.rs` subcommand) The simulated old outcomes agree with every observed one (658, 659 and 661 hit; 662, 663 and 664 miss)

## 3. Verification strength is unchanged

- [x] 3.1 @manual (agent) diff of `bundle-build.yml` against origin/main -> only the job `env` list, the new digest step, the key and its comment change. The pin read, restore check with its discard-and-rebuild, build, build pin check, lock check, main-only save and upload are byte-identical

## 4. Repository checks

- [x] 4.1 @regression (agent) `mise run //packages/kuru-delivery:test`, `lint`, `lint:windows`, `typecheck`, `format:check`, `lint:tooling`, `docs:check`, `cospec validate --strict` -> each exit 0 on macOS arm64. The delivery test ran 189 lib tests, bundle_build 5, release_workflow 34 and repo_validation 28, with 1 ignored (the opt-in previous-release update). lint:tooling ran actionlint and shellcheck clean and printed "Repository metadata invariants passed."; docs:check printed "Public docs artifacts, local links and anchors passed". The memory package's bundle tests were not run because no memory file changed. The 90% coverage gate was not measured locally

## 5. CI

- [~] 5.1 @runtime (human) GitHub-hosted ubuntu-latest `Pin-verified Windows arm64 engine input` on this branch's pull request -> defer: the branch is not pushed. On its pull request the digest step should print a digest, and the job should miss (new key), rebuild and match the pin. The first main run after merge should miss once and save. A later PR that changes only unkeyed `Cargo.lock` records should then restore in seconds
- [~] 5.2 @runtime (human) the delivery test partitions on windows-latest and windows-11-arm run `bundle_build`'s key-coverage tests -> defer: run only on macOS arm64 locally. The test reads repository text with `/` paths and LF lines, which the repository's LF checkout rule provides
