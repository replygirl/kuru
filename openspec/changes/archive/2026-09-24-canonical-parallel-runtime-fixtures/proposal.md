## Why

Three parallel-tool runtime fixtures fail on Windows before exercising their assertions because they give the retained tool root a lexical temporary path while the harness uses its canonical form. Both must receive the same canonical workspace path.

## What Changes

- In `packages/kuru-runtime/src/tests.rs`, canonicalize each fixture project path once and pass it to both the retained `Directory` and `Harness`.
- Keep the production exact-root guard and each fixture's file effects and identity assertions unchanged.

## Impact

Only runtime test setup changes. Native Windows CI must confirm that all three tests reach their intended parallel-read and serial-effect assertions.
