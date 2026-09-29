# Tasks

## 1. Narrow the engine input cache key

- [x] 1.1 Establish each hashed path's effect on the archive bytes, with file:line, and verify against the helper source
- [x] 1.2 Replace the key in `.github/workflows/bundle-build.yml` with the manifest, helper and ZIP writer sources and the locked writer-crate digest, and verify the diff leaves every verification step unchanged
- [x] 1.3 Add the key-coverage test and derive the toolchain test's tools from the task, and verify each mutation in verification 1.1 fails
- [x] 1.4 Update `docs/development.md` and verify `docs:check`
- [x] 1.5 Compute old and new keys over recent history and verify the reproduction against GitHub's recorded keys
- [x] 1.6 Run the repository checks and record results
