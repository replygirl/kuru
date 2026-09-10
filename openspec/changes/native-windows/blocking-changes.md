# Dependencies

## Blocked by

- [ ] `native-platform` — supplies the independently verified Rust filesystem, owned-process and private IPC primitives; this product change adopts them and verifies real consumers.
- [ ] `dolt-memory` — supplies the async memory API, full Dolt schema, writer ownership, supervisor lifecycle, migration and candidate promotion contracts that the Windows implementation must preserve.
- [ ] `embedded-runtime` — supplies the package-owned verified bundle preparation and executable embedding contract; Windows adds its target without introducing runtime downloads.

## Soft-blocked by

None.

## Ordering and established foundations

All three providers are active changes in this planning home. Their proposals were
reviewed along with every other active proposal and the archived changes. The
user's native Windows and embedded-runtime requirements establish these hard
dependencies; this change does not make any provider depend on Windows product
integration. `native-platform` is independently tested with compiled Rust
fixtures and does not depend on Dolt or embedded runtime.
`embedded-runtime` establishes the existing four platforms first. These blockers
must be resolved before applying this change for implementation.

The archived native delivery, direct release installation, release rerun and
release-only documentation changes already establish the package ownership,
verified bootstrap, strategy-only dispatch, automatic recovery and final Pages
publication conventions. They remain the baseline and are not new blockers.
