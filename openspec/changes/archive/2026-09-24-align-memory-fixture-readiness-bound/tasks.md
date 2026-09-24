## 1. Align native memory fixture bounds

- [x] 1.1 In `packages/kuru-memory/src/service.rs`, share the existing 120-second overall deadline with the crashed-owner initial readiness loop and preserve receipt/recovery assertions.
- [x] 1.2 In `packages/kuru-memory/src/service.rs`, share the existing 110-second overall deadline with the contained-owner initial readiness loop and preserve Job containment, survivor, and recovery assertions.

## 2. Verify and deliver

- [x] 2.1 Run source format and owning memory lint. Record that macOS-hosted Windows memory cross-check stops in `libsqlite3-sys` at missing Windows SDK headers before Kuru source; native Windows fixture execution remains an exact-head CI gate.
- [x] 2.2 Obtain bounded independent source review and archive this test change before the P29 branch checkpoint.
