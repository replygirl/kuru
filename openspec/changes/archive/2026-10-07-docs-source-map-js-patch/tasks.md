# Tasks

## 1. Patch the docs development dependency

- [x] 1.1 Update only `apps/kuru-docs/package-lock.json` so `source-map-js` resolves to 1.2.2, using the docs app's pinned Node/npm tooling; verify the manifest stays unchanged and the registry publishes the patched version.
- [x] 1.2 Run the docs formatting, lint, build, content, and link checks; verify the lockfile delta only changes the `source-map-js` resolution and required integrity metadata.
