# Tasks

## 1. Disable implicit exec installs in CI

- [x] 1.1 Set `MISE_EXEC_AUTO_INSTALL: "false"` in the workflow-level env of ci, quality, native-tests, bundle-build and release, and verify each file has exactly one such line
- [x] 1.2 Document the setting and the Windows shim mechanism in docs/development.md Shared build cache, and verify `docs:check` passes
- [x] 1.3 Verify with lint:tooling, format:check, cospec validate --strict and cospec:managed:check, and confirm `mise.toml` and `mise.lock` are unchanged
