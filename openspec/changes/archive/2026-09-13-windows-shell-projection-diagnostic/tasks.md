## 1. Failure-only Windows shell diagnostics

- [x] 1.1 Add the three fresh-root control launches around the existing projected ToolHost shell assertion, retaining the original failure and bounded owned cleanup.
- [x] 1.2 Record native execution as pending: 2026-09-13 macOS cross-target typecheck stopped before `kuru-connectors` at aws-lc-sys because the host has no Windows SDK headers (`stdlib.h`/`windows.h`, `VCINSTALLDIR` unset). The fixture’s native diagnostic outcome remains unobserved.
