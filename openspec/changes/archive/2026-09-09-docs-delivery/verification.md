## 1. Build and deploy contracts [critical]

- [x] 1.1 @integration (agent) Install pinned tools and build docs -> app-local npm ci and the pinned VitePress build passed on 2026-09-09; root and package mise locks cover the supported platforms.
- [x] 1.2 @integration (agent) Validate Pages workflow, base paths and built links -> Actionlint passed; review confirmed resolved main-ancestor SHA is checked out before build. Native docs validator accepts all local links/anchors and required artifacts under /kuru/.
- [x] 1.3 @integration (agent) Run the full repository gate -> mise run check passed with docs, release and memory-contention tests included: 209 tests, 97.51% Rust coverage (8554/8772), unchanged 90% requirement.
- [x] 1.4 @manual (agent) Inspect built content and browser behavior -> Chrome/Playwright passed at 1440px and 390px: four distinct normal-click portraits, aria-pressed, keyboard controls, light/dark, reduced motion, search to configuration, nested reload and mobile navigation. Seven screenshots were inspected with no clipping/overflow; no browser or resource errors.

## 2. Hosted follow-through

- [x] 2.1 @runtime (agent) Run the docs build in GitHub Actions on Ubuntu -> run 34391814388 completed docs:check at 2026-09-09 19:01:24 UTC on Ubuntu 24.04, including the production build and native public-content/link checks under the pinned toolchain. The overall run was subsequently canceled for a separately reproduced single-thread fixture correction; this records the completed docs task, not an overall CI pass.

After merge, deploy the requested site through GitHub Pages and report its live
URL and the hosted workflow result. No release tag is authorized by this change.
