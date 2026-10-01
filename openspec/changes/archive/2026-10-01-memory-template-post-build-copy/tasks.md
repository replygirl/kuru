# Tasks

## 1. Error phase for the post-build copy

- [x] 1.1 Add a `CreateError` variant for the copy that follows this call's own build, return it from `create_in` for both the published-template and verified-stage copies, and give it a `Display` that names the copy; verify the verdict quarantine and no-cold-retry behaviour are unchanged
- [x] 1.2 Give that variant its own context in `creation_worker::run`, still failing the open after setting the stage aside; verify no other caller matched on `CreateError::Build` for the copy
- [x] 1.3 Tighten `failed_copy_after_the_build_fails_the_open_without_a_cold_retry` so it fails while the error reports a build failure (before the fix) and passes after; verify by running it on real engines *(Observed: with the old `creation_template.rs` and `creation_worker.rs` and the tightened test, the test failed on the read-error case with "build the memory store template: copy the new project from the store template this open built: read store template tree …: injected template read error"; with the fix it passed.)*

## 2. Spec correction and checks

- [x] 2.1 Author the `versioned-memory` delta: restrict the cold clause and its scenario to a template this open did not build, add the post-build copy to the fail-the-open list with its verdict-only quarantine, narrow the startup-lock sentence to the project stage's engine, and add the regression scenario; verify with strict validation
- [x] 2.2 Run the focused memory tests, lint, format and Windows-target lint and record the observed results in `verification.md` *(Observed: see `verification.md`; native CI on the four operating systems runs after the push.)*
