# Tasks

## 1. Narrow the engine input cache key

- [x] 1.1 Establish each hashed path's effect on the archive bytes, with file:line, and verify against the helper source
- [x] 1.2 Replace the key in `.github/workflows/bundle-build.yml` with the manifest, helper and ZIP writer sources and the locked writer-crate digest, and verify the diff leaves every verification step unchanged
- [x] 1.3 Add the key-coverage test and derive the toolchain test's tools from the task, and verify each mutation in verification 1.1 fails
- [x] 1.4 Update `docs/development.md` and verify `docs:check`
- [x] 1.5 Compute old and new keys over recent history and verify the reproduction against GitHub's recorded keys
- [x] 1.6 Run the repository checks and record results

## 2. Apply the independent review

- [x] 2.1 Make the key-coverage test fail closed on the `bundle:build` task (exact command, allowed keys, tool options), `setup:build-tools`, the memory package's and root mise configuration, other tracked mise config files, and the input job's environment, actions and exact toolchain and build steps, and verify each case with a mutation
- [x] 2.2 Require the key's exact shape (one `hashFiles`), and verify a second `hashFiles` and a dropped crate digest are rejected
- [x] 2.3 Reject `include_*!`, `super::super` and any crate or module a keyed file uses outside the key unless exempted with a reason, and verify a moved `crate::recipe` constant is rejected
- [x] 2.4 Pin the compression crates' Cargo features statically (workspace specifications, member use and features, `[patch]`/`[replace]`, resolver, registry dependents in `Cargo.lock`), and verify a `zlib-rs` feature and a new registry dependent are rejected
- [x] 2.5 Correct `docs/development.md` and the PR body to what the test enforces, and list the inputs outside both key and test with the checks behind them
- [x] 2.6 Record the pull request's CI run of the input job and the merge-commit keying in the verification ledger
- [x] 2.7 Reject a tracked `.cargo/config` or `.cargo/config.toml` in the root, `packages/`, the helper's package and the memory package, document it with the other test checks, and verify each case with a mutation
