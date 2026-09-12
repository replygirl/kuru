## 1. Pinned-engine evidence

- [x] 1.1 Query `@@GLOBAL.dolt_auto_gc_enabled` through the existing isolated real-Dolt server lifecycle fixture and assert the effective value is `0`; verified it uses the pinned provisioned engine and makes no configuration or data mutation.
- [x] 1.2 Run the focused native server-lifecycle regression through its package mise task; verified the observed effective value is `0` and the fixture completed cleanup (1 passed, 11 filtered, 2026-09-12).
