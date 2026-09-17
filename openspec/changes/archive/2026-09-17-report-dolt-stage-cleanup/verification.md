## 1. Published-stage cleanup [critical]

- [~] 1.1 @regression (agent) run an isolated Windows activation with a held stage-only file handle that prevents stage removal -> defer: this fixture is Windows-only and cannot compile or execute on the macOS host; native Windows CI is pending. The before-fix success path returned Ok after an unchecked TempDir destructor.
- [x] 1.2 @integration (agent) run a successful staged activation on the host -> package-owned exact `provision::native_tests::successful_activation_removes_its_disposable_stage` filter ran 1/1 passed on macOS; verified destination bytes and no disposable stage

## 2. Existing activation semantics

- [x] 2.1 @regression (agent) run the existing native memory activation fixtures -> package-owned rejected-activation and source-open-failure filters each ran 1/1 passed on macOS; both preserve their failed stage and release the cache lease
- [x] 2.2 @unit (agent) run memory typecheck, lint, formatting and diff checks -> memory all-target/all-feature typecheck and Clippy passed; scoped formatting and diff checks passed
