# Verification

## 1. Dependabot uses the owning npm project [critical]

- [x] 1.1 @unit (agent) inspect the configured npm directory and verify its package manifest and lockfile exist -> `directory: /apps/kuru-docs`; both `apps/kuru-docs/package.json` and `apps/kuru-docs/package-lock.json` exist.
- [~] 1.2 @runtime (human) observe a real GitHub Dependabot `npm_and_yarn` update job for `/apps/kuru-docs` -> defer: Dependabot service behavior can only be observed after the reviewed change merges; no remote success is claimed here.
