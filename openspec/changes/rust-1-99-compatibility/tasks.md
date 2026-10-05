# Tasks

## 1. Preserve atomic behavior with the supported API

- [ ] 1.1 Replace deprecated atomic calls with try_update while retaining the existing closure, ordering and result semantics.
- [ ] 1.2 Run the same warnings-denied compiler check that failed before the correction, verify host and Windows lint passes afterward, and exercise existing tool-admission and memory-activity behavior tests.
- [ ] 1.3 Record observed compatibility evidence and archive this scoped fix through Cospec before final delivery.
